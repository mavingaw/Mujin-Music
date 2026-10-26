/*
 * A userspace stand-in for the Linux kernel's ALSA ABI with one USB DAC on
 * it, for proving Parson's ALSA hw path on machines without sound
 * hardware (CI containers have no CONFIG_SOUND and no USB bus).
 *
 * Loaded with LD_PRELOAD, it answers what snd-usb-audio would for card 1:
 *
 * - /dev/snd/controlC1: SNDRV_CTL_IOCTL_* (card info, PCM enumeration).
 * - /dev/snd/pcmC1D0p: SNDRV_PCM_IOCTL_* with the kernel's own hw_params
 *   refinement (the interval/mask rules of sound/core/pcm_native.c and
 *   pcm_lib.c, plus snd-usb-audio's per-altset format/rate rules), the
 *   SYNC_PTR status/control protocol libasound falls back to when the
 *   status page cannot be mmap'd, a DMA ring (read/write or mmap'd, as
 *   ALSA's plug uses it) that drains in real time at the negotiated rate,
 *   and poll() readiness from avail_min. One substream: a second open is EBUSY, as for
 *   a real hw device.
 * - /proc/asound/card1 (and /proc/asound/<id>): usbid, stream0 in
 *   snd-usb-audio's proc.c layout (Running/Stop, Momentary freq) and
 *   pcm0p/sub0/hw_params in pcm.c's layout, generated from the live state.
 *
 * Every other path and file descriptor passes through untouched, so the
 * real libasound (hw plugin, configuration, hints, snd_pcm_dump) and the
 * real program run unmodified on top. The device says what it is: its
 * product string reads "kernel ABI emulator", its USB id is pid.codes'
 * test id (1209:0001), and the symbols parson_snd_usb_emulator and
 * parson_snd_usb_emulator_card let a program detect it and its card.
 *
 * PARSON_SND_EMULATOR_CAPTURE=<path>: every byte the "DAC" consumes is
 * appended there, in the order and at the rate it plays them.
 *
 * Build: cc -shared -fPIC -O2 -o libparson_snd_usb_emulator.so \
 *          snd_usb_emulator.c -ldl -lpthread
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <poll.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <sound/asound.h>

#define CARD 1

/* Lets a program tell that this emulator, not a kernel, answers for a card. */
__attribute__((visibility("default")))
const char parson_snd_usb_emulator[] = "parson-snd-usb-emulator 1";
__attribute__((visibility("default")))
const int parson_snd_usb_emulator_card = CARD;
#define CARD_ID "BenchDAC"
#define USB_ID "1209:0001"
#define PRODUCT "Parson Bench DAC (kernel ABI emulator)"
#define BUS_PATH "usb-0000:00:14.0-1"

/* ---- the device: snd-usb-audio alternate settings ---------------------- */

struct altset {
	int altset;
	int format;
	int bits;
	const unsigned int *rates;
	int nrates;
};

static const unsigned int rates_hi[] = {44100, 48000, 88200, 96000,
					176400, 192000, 352800, 384000};
static const unsigned int rates_mid[] = {44100, 48000, 88200, 96000,
					 176400, 192000};

static const struct altset altsets[] = {
	{1, SNDRV_PCM_FORMAT_S16_LE, 16, rates_mid, 6},
	{2, SNDRV_PCM_FORMAT_S24_3LE, 24, rates_mid, 6},
	{3, SNDRV_PCM_FORMAT_S32_LE, 32, rates_hi, 8},
};
#define NALTSETS ((int)(sizeof(altsets) / sizeof(altsets[0])))
#define CHANNELS 2
#define PERIOD_BYTES_MIN 64
#define PERIOD_BYTES_MAX (1024 * 1024)
#define PERIODS_MIN 2
#define PERIODS_MAX 1024
#define BUFFER_BYTES_MAX (1024 * 1024)

static int phys_width(int format)
{
	switch (format) {
	case SNDRV_PCM_FORMAT_S16_LE: return 16;
	case SNDRV_PCM_FORMAT_S24_3LE: return 24;
	case SNDRV_PCM_FORMAT_S32_LE: return 32;
	case SNDRV_PCM_FORMAT_S24_LE: return 32;
	default: return -1;
	}
}

static int sig_width(int format)
{
	switch (format) {
	case SNDRV_PCM_FORMAT_S16_LE: return 16;
	case SNDRV_PCM_FORMAT_S24_3LE: return 24;
	case SNDRV_PCM_FORMAT_S32_LE: return 32;
	default: return -1;
	}
}

static const char *format_name(int format)
{
	switch (format) {
	case SNDRV_PCM_FORMAT_S16_LE: return "S16_LE";
	case SNDRV_PCM_FORMAT_S24_3LE: return "S24_3LE";
	case SNDRV_PCM_FORMAT_S32_LE: return "S32_LE";
	default: return "UNKNOWN";
	}
}

/* ---- the kernel's interval and mask arithmetic (pcm_lib.c) ------------- */

typedef struct snd_interval interval_t;
typedef struct snd_mask mask_t;

#define P(name) SNDRV_PCM_HW_PARAM_##name

static interval_t *hw_iv(struct snd_pcm_hw_params *p, int k)
{
	return &p->intervals[k - P(FIRST_INTERVAL)];
}

static mask_t *hw_mask(struct snd_pcm_hw_params *p, int k)
{
	return &p->masks[k - P(FIRST_MASK)];
}

static void iv_any(interval_t *i)
{
	memset(i, 0, sizeof(*i));
	i->min = 0;
	i->max = UINT_MAX;
}

static int iv_checkempty(const interval_t *i)
{
	return i->min > i->max ||
	       (i->min == i->max && (i->openmin || i->openmax));
}

static int iv_single(const interval_t *i)
{
	return !i->empty && (i->min == i->max ||
			     (i->min + 1 == i->max && (i->openmin || i->openmax)));
}

static unsigned int iv_value(const interval_t *i)
{
	if (i->openmin && !i->openmax)
		return i->max;
	return i->min;
}

static int iv_test(const interval_t *i, unsigned int v)
{
	return !(i->min > v || (i->min == v && i->openmin) || i->max < v ||
		 (i->max == v && i->openmax));
}

static int iv_refine(interval_t *i, const interval_t *v)
{
	int changed = 0;
	if (i->empty)
		return -EINVAL;
	if (i->min < v->min) {
		i->min = v->min;
		i->openmin = v->openmin;
		changed = 1;
	} else if (i->min == v->min && !i->openmin && v->openmin) {
		i->openmin = 1;
		changed = 1;
	}
	if (i->max > v->max) {
		i->max = v->max;
		i->openmax = v->openmax;
		changed = 1;
	} else if (i->max == v->max && !i->openmax && v->openmax) {
		i->openmax = 1;
		changed = 1;
	}
	if (!i->integer && v->integer) {
		i->integer = 1;
		changed = 1;
	}
	if (i->integer) {
		if (i->openmin) {
			i->min++;
			i->openmin = 0;
		}
		if (i->openmax) {
			i->max--;
			i->openmax = 0;
		}
	} else if (!i->openmin && !i->openmax && i->min == i->max) {
		i->integer = 1;
	}
	if (iv_checkempty(i)) {
		i->empty = 1;
		return -EINVAL;
	}
	return changed;
}

static int iv_refine_first(interval_t *i)
{
	unsigned int last_max = i->max;
	if (i->empty)
		return -EINVAL;
	if (iv_single(i))
		return 0;
	i->max = i->min;
	if (i->openmin)
		i->max++;
	i->openmax = (i->openmax && i->max >= last_max);
	return 1;
}

static int iv_refine_last(interval_t *i)
{
	unsigned int last_min = i->min;
	if (i->empty)
		return -EINVAL;
	if (iv_single(i))
		return 0;
	i->min = i->max;
	if (i->openmax)
		i->min--;
	i->openmin = (i->openmin && i->min <= last_min);
	return 1;
}

static unsigned int mul_sat(unsigned int a, unsigned int b)
{
	if (b == 0)
		return 0;
	if (a >= UINT_MAX / b)
		return UINT_MAX;
	return a * b;
}

static unsigned int div32(unsigned int a, unsigned int b, unsigned int *r)
{
	if (b == 0) {
		*r = 0;
		return UINT_MAX;
	}
	*r = a % b;
	return a / b;
}

static unsigned int muldiv32(unsigned int a, unsigned int b, unsigned int c,
			     unsigned int *r)
{
	uint64_t n = (uint64_t)a * b;
	if (c == 0 || n / c > UINT_MAX) {
		*r = 0;
		return UINT_MAX;
	}
	*r = (unsigned int)(n % c);
	return (unsigned int)(n / c);
}

static void iv_mul(const interval_t *a, const interval_t *b, interval_t *c)
{
	iv_any(c);
	c->min = mul_sat(a->min, b->min);
	c->openmin = a->openmin || b->openmin;
	c->max = mul_sat(a->max, b->max);
	c->openmax = a->openmax || b->openmax;
	c->integer = a->integer && b->integer;
}

static void iv_div(const interval_t *a, const interval_t *b, interval_t *c)
{
	unsigned int r;
	iv_any(c);
	c->min = div32(a->min, b->max, &r);
	c->openmin = r || a->openmin || b->openmax;
	if (b->min > 0) {
		c->max = div32(a->max, b->min, &r);
		if (r) {
			c->max++;
			c->openmax = 1;
		} else {
			c->openmax = a->openmax || b->openmin;
		}
	} else {
		c->max = UINT_MAX;
		c->openmax = 0;
	}
	c->integer = 0;
}

/* c = a * b / k */
static void iv_muldivk(const interval_t *a, const interval_t *b, unsigned int k,
		       interval_t *c)
{
	unsigned int r;
	iv_any(c);
	c->min = muldiv32(a->min, b->min, k, &r);
	c->openmin = r || a->openmin || b->openmin;
	c->max = muldiv32(a->max, b->max, k, &r);
	if (r) {
		c->max++;
		c->openmax = 1;
	} else {
		c->openmax = a->openmax || b->openmax;
	}
	c->integer = 0;
}

/* c = a * k / b */
static void iv_mulkdiv(const interval_t *a, unsigned int k, const interval_t *b,
		       interval_t *c)
{
	unsigned int r;
	iv_any(c);
	c->min = muldiv32(a->min, k, b->max, &r);
	c->openmin = r || a->openmin || b->openmax;
	if (b->min > 0) {
		c->max = muldiv32(a->max, k, b->min, &r);
		if (r) {
			c->max++;
			c->openmax = 1;
		} else {
			c->openmax = a->openmax || b->openmin;
		}
	} else {
		c->max = UINT_MAX;
		c->openmax = 0;
	}
	c->integer = 0;
}

static int mask_test(const mask_t *m, unsigned int bit)
{
	return (m->bits[bit / 32] >> (bit % 32)) & 1;
}

static void mask_set(mask_t *m, unsigned int bit)
{
	m->bits[bit / 32] |= 1U << (bit % 32);
}

static void mask_reset(mask_t *m, unsigned int bit)
{
	m->bits[bit / 32] &= ~(1U << (bit % 32));
}

static int mask_empty(const mask_t *m)
{
	for (int i = 0; i < 8; i++)
		if (m->bits[i])
			return 0;
	return 1;
}

static int mask_min(const mask_t *m)
{
	for (int b = 0; b < SNDRV_MASK_MAX; b++)
		if (mask_test(m, b))
			return b;
	return -1;
}

static int mask_single(const mask_t *m)
{
	int count = 0;
	for (int b = 0; b < SNDRV_MASK_MAX; b++)
		count += mask_test(m, b);
	return count == 1;
}

static int mask_refine(mask_t *m, const mask_t *v)
{
	mask_t old = *m;
	for (int i = 0; i < 8; i++)
		m->bits[i] &= v->bits[i];
	if (mask_empty(m))
		return -EINVAL;
	return memcmp(&old, m, sizeof(old)) != 0;
}

/* ---- refinement: driver constraints and the kernel's rules ------------- */

static mask_t constr_masks[P(LAST_MASK) + 1];
static interval_t constr_ivs[P(LAST_INTERVAL) + 1];

static void init_constraints(void)
{
	memset(constr_masks, 0, sizeof(constr_masks));
	mask_set(&constr_masks[P(ACCESS)], SNDRV_PCM_ACCESS_MMAP_INTERLEAVED);
	mask_set(&constr_masks[P(ACCESS)], SNDRV_PCM_ACCESS_RW_INTERLEAVED);
	for (int k = 0; k < NALTSETS; k++)
		mask_set(&constr_masks[P(FORMAT)], (unsigned int)altsets[k].format);
	mask_set(&constr_masks[P(SUBFORMAT)], SNDRV_PCM_SUBFORMAT_STD);
	for (int k = P(FIRST_INTERVAL); k <= P(LAST_INTERVAL); k++)
		iv_any(&constr_ivs[k]);
	/* snd_pcm_hw_constraints_init */
	constr_ivs[P(CHANNELS)].integer = 1;
	constr_ivs[P(BUFFER_SIZE)].integer = 1;
	constr_ivs[P(BUFFER_BYTES)].integer = 1;
	constr_ivs[P(SAMPLE_BITS)].integer = 1;
	constr_ivs[P(FRAME_BITS)].integer = 1;
	/* snd_pcm_hw_constraints_complete, from the runtime hw */
	constr_ivs[P(CHANNELS)].min = CHANNELS;
	constr_ivs[P(CHANNELS)].max = CHANNELS;
	constr_ivs[P(RATE)].min = 44100;
	constr_ivs[P(RATE)].max = 384000;
	constr_ivs[P(PERIOD_BYTES)].min = PERIOD_BYTES_MIN;
	constr_ivs[P(PERIOD_BYTES)].max = PERIOD_BYTES_MAX;
	constr_ivs[P(PERIODS)].min = PERIODS_MIN;
	constr_ivs[P(PERIODS)].max = PERIODS_MAX;
	constr_ivs[P(BUFFER_BYTES)].min = PERIOD_BYTES_MIN;
	constr_ivs[P(BUFFER_BYTES)].max = BUFFER_BYTES_MAX;
	/* snd-usb-audio: whole periods of at least one millisecond */
	constr_ivs[P(PERIODS)].integer = 1;
	constr_ivs[P(PERIOD_TIME)].min = 1000;
}

/* snd-usb-audio's hw_rule_format: formats with an altset at an allowed rate. */
static int rule_usb_format(struct snd_pcm_hw_params *p)
{
	mask_t keep;
	memset(&keep, 0, sizeof(keep));
	const interval_t *rate = hw_iv(p, P(RATE));
	for (int k = 0; k < NALTSETS; k++)
		for (int r = 0; r < altsets[k].nrates; r++)
			if (iv_test(rate, altsets[k].rates[r])) {
				mask_set(&keep, (unsigned int)altsets[k].format);
				break;
			}
	return mask_refine(hw_mask(p, P(FORMAT)), &keep);
}

/* snd-usb-audio's hw_rule_rate: rates of the altsets whose format is allowed. */
static int rule_usb_rate(struct snd_pcm_hw_params *p)
{
	interval_t range;
	iv_any(&range);
	range.min = UINT_MAX;
	range.max = 0;
	interval_t *rate = hw_iv(p, P(RATE));
	const mask_t *formats = hw_mask(p, P(FORMAT));
	for (int k = 0; k < NALTSETS; k++) {
		if (!mask_test(formats, (unsigned int)altsets[k].format))
			continue;
		for (int r = 0; r < altsets[k].nrates; r++) {
			unsigned int v = altsets[k].rates[r];
			if (!iv_test(rate, v))
				continue;
			if (v < range.min)
				range.min = v;
			if (v > range.max)
				range.max = v;
		}
	}
	range.integer = 1;
	return iv_refine(rate, &range);
}

static int rule_format_from_bits(struct snd_pcm_hw_params *p)
{
	mask_t m = *hw_mask(p, P(FORMAT));
	const interval_t *i = hw_iv(p, P(SAMPLE_BITS));
	for (int k = 0; k < SNDRV_MASK_MAX; k++) {
		if (!mask_test(&m, k))
			continue;
		int bits = phys_width(k);
		if (bits <= 0)
			continue;
		if ((unsigned int)bits < i->min || (unsigned int)bits > i->max)
			mask_reset(&m, k);
	}
	return mask_refine(hw_mask(p, P(FORMAT)), &m);
}

static int rule_bits_from_format(struct snd_pcm_hw_params *p)
{
	interval_t x;
	iv_any(&x);
	x.min = UINT_MAX;
	x.max = 0;
	const mask_t *m = hw_mask(p, P(FORMAT));
	for (int k = 0; k < SNDRV_MASK_MAX; k++) {
		if (!mask_test(m, k))
			continue;
		int bits = phys_width(k);
		if (bits <= 0)
			continue;
		if (x.min > (unsigned int)bits)
			x.min = (unsigned int)bits;
		if (x.max < (unsigned int)bits)
			x.max = (unsigned int)bits;
	}
	x.integer = 1;
	return iv_refine(hw_iv(p, P(SAMPLE_BITS)), &x);
}

enum op { MUL, DIV, MULDIVK, MULKDIV };
struct rule {
	int var;
	enum op op;
	int a, b;
	unsigned int k;
};

/* The rules snd_pcm_hw_constraints_init adds, in its order. */
static const struct rule rules[] = {
	{P(SAMPLE_BITS), DIV, P(FRAME_BITS), P(CHANNELS), 0},
	{P(FRAME_BITS), MUL, P(SAMPLE_BITS), P(CHANNELS), 0},
	{P(FRAME_BITS), MULKDIV, P(PERIOD_BYTES), P(PERIOD_SIZE), 8},
	{P(FRAME_BITS), MULKDIV, P(BUFFER_BYTES), P(BUFFER_SIZE), 8},
	{P(CHANNELS), DIV, P(FRAME_BITS), P(SAMPLE_BITS), 0},
	{P(RATE), MULKDIV, P(PERIOD_SIZE), P(PERIOD_TIME), 1000000},
	{P(RATE), MULKDIV, P(BUFFER_SIZE), P(BUFFER_TIME), 1000000},
	{P(PERIODS), DIV, P(BUFFER_SIZE), P(PERIOD_SIZE), 0},
	{P(PERIOD_SIZE), DIV, P(BUFFER_SIZE), P(PERIODS), 0},
	{P(PERIOD_SIZE), MULKDIV, P(PERIOD_BYTES), P(FRAME_BITS), 8},
	{P(PERIOD_SIZE), MULDIVK, P(PERIOD_TIME), P(RATE), 1000000},
	{P(BUFFER_SIZE), MUL, P(PERIOD_SIZE), P(PERIODS), 0},
	{P(BUFFER_SIZE), MULKDIV, P(BUFFER_BYTES), P(FRAME_BITS), 8},
	{P(BUFFER_SIZE), MULDIVK, P(BUFFER_TIME), P(RATE), 1000000},
	{P(PERIOD_BYTES), MULDIVK, P(PERIOD_SIZE), P(FRAME_BITS), 8},
	{P(BUFFER_BYTES), MULDIVK, P(BUFFER_SIZE), P(FRAME_BITS), 8},
	{P(PERIOD_TIME), MULKDIV, P(PERIOD_SIZE), P(RATE), 1000000},
	{P(BUFFER_TIME), MULKDIV, P(BUFFER_SIZE), P(RATE), 1000000},
};
#define NRULES ((int)(sizeof(rules) / sizeof(rules[0])))

/* Applies every rule until none changes anything (the kernel's fixpoint). */
static int apply_rules(struct snd_pcm_hw_params *p)
{
	for (int pass = 0; pass < 256; pass++) {
		int any = 0, c;
#define STEP(expr, bit)                           \
	do {                                      \
		c = (expr);                       \
		if (c < 0)                        \
			return c;                 \
		if (c) {                          \
			any = 1;                  \
			p->cmask |= 1U << (bit);  \
		}                                 \
	} while (0)
		STEP(rule_format_from_bits(p), P(FORMAT));
		STEP(rule_bits_from_format(p), P(SAMPLE_BITS));
		STEP(rule_usb_format(p), P(FORMAT));
		STEP(rule_usb_rate(p), P(RATE));
		for (int r = 0; r < NRULES; r++) {
			const struct rule *ru = &rules[r];
			interval_t t;
			const interval_t *a = hw_iv(p, ru->a), *b = hw_iv(p, ru->b);
			switch (ru->op) {
			case MUL: iv_mul(a, b, &t); break;
			case DIV: iv_div(a, b, &t); break;
			case MULDIVK: iv_muldivk(a, b, ru->k, &t); break;
			case MULKDIV: iv_mulkdiv(a, ru->k, b, &t); break;
			}
			STEP(iv_refine(hw_iv(p, ru->var), &t), ru->var);
		}
#undef STEP
		if (!any)
			return 0;
	}
	return 0;
}

/* pcm_native.c fixup_unreferenced_params */
static void fixup_unreferenced(struct snd_pcm_hw_params *p)
{
	if (!p->msbits) {
		const interval_t *i = hw_iv(p, P(SAMPLE_BITS));
		if (iv_single(i))
			p->msbits = iv_value(i);
	}
	if (p->msbits) {
		const mask_t *m = hw_mask(p, P(FORMAT));
		if (mask_single(m)) {
			int width = sig_width(mask_min(m));
			if (width > 0)
				p->msbits = (unsigned int)width;
		}
	}
	if (!p->rate_den) {
		const interval_t *i = hw_iv(p, P(RATE));
		if (iv_single(i)) {
			p->rate_num = iv_value(i);
			p->rate_den = 1;
		}
	}
	if (!p->info)
		p->info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_MMAP_VALID |
			  SNDRV_PCM_INFO_INTERLEAVED | SNDRV_PCM_INFO_BLOCK_TRANSFER |
			  SNDRV_PCM_INFO_BATCH | SNDRV_PCM_INFO_PAUSE;
}

static int hw_refine(struct snd_pcm_hw_params *p)
{
	int c;
	p->info = 0;
	p->fifo_size = 0;
	if (p->rmask & (1U << P(SAMPLE_BITS)))
		p->msbits = 0;
	if (p->rmask & (1U << P(RATE))) {
		p->rate_num = 0;
		p->rate_den = 0;
	}
	for (int k = P(FIRST_MASK); k <= P(LAST_MASK); k++) {
		if (!(p->rmask & (1U << k)))
			continue;
		c = mask_refine(hw_mask(p, k), &constr_masks[k]);
		if (c < 0)
			return c;
		if (c)
			p->cmask |= 1U << k;
	}
	for (int k = P(FIRST_INTERVAL); k <= P(LAST_INTERVAL); k++) {
		if (!(p->rmask & (1U << k)))
			continue;
		c = iv_refine(hw_iv(p, k), &constr_ivs[k]);
		if (c < 0)
			return c;
		if (c)
			p->cmask |= 1U << k;
	}
	c = apply_rules(p);
	if (c < 0)
		return c;
	p->rmask = 0;
	fixup_unreferenced(p);
	return 0;
}

/* pcm_native.c snd_pcm_hw_params_choose */
static int hw_choose(struct snd_pcm_hw_params *p)
{
	static const int vars[] = {P(ACCESS), P(FORMAT), P(SUBFORMAT),
				   P(CHANNELS), P(RATE), P(PERIOD_TIME),
				   P(BUFFER_SIZE), P(TICK_TIME)};
	for (unsigned int v = 0; v < sizeof(vars) / sizeof(vars[0]); v++) {
		int k = vars[v], c;
		if (k <= P(LAST_MASK)) {
			mask_t *m = hw_mask(p, k);
			int first = mask_min(m);
			if (first < 0)
				return -EINVAL;
			memset(m, 0, sizeof(*m));
			mask_set(m, (unsigned int)first);
			c = 0;
		} else if (k == P(BUFFER_SIZE)) {
			c = iv_refine_last(hw_iv(p, k));
		} else {
			c = iv_refine_first(hw_iv(p, k));
		}
		if (c < 0)
			return c;
		c = apply_rules(p);
		if (c < 0)
			return c;
	}
	return 0;
}

/* ---- the substream ------------------------------------------------------ */

enum { NODE_NONE, NODE_CTL, NODE_PCM };

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static unsigned char node_of[65536];

static struct {
	int open;
	int fd;
	int state;
	int configured;
	int access;
	int format;
	unsigned int rate, channels, frame_bytes;
	snd_pcm_uframes_t period_size, buffer_size;
	snd_pcm_uframes_t appl_ptr, hw_ptr, avail_min;
	snd_pcm_uframes_t start_threshold, stop_threshold, boundary;
	unsigned char *ring;
	/* The ring is the buffer mmap'd by the program (mmap access). */
	size_t ring_mapped;
	struct timespec started;
	snd_pcm_uframes_t hw_at_start;
	struct timespec trigger;
	int capture_fd;
} pcm = {.capture_fd = -1};

static int (*real_open)(const char *, int, ...);
static int (*real_openat)(int, const char *, int, ...);
static int (*real_close)(int);
static int (*real_ioctl)(int, unsigned long, ...);
static void *(*real_mmap)(void *, size_t, int, int, int, off_t);

static void resolve(void)
{
	if (real_open)
		return;
	real_openat = dlsym(RTLD_NEXT, "openat");
	real_close = dlsym(RTLD_NEXT, "close");
	real_ioctl = dlsym(RTLD_NEXT, "ioctl");
	real_mmap = dlsym(RTLD_NEXT, "mmap");
	real_open = dlsym(RTLD_NEXT, "open");
}

static void release_ring(void)
{
	if (pcm.ring_mapped)
		munmap(pcm.ring, pcm.ring_mapped);
	else
		free(pcm.ring);
	pcm.ring = NULL;
	pcm.ring_mapped = 0;
}

static double ts_s(const struct timespec *t)
{
	return (double)t->tv_sec + (double)t->tv_nsec / 1e9;
}

static double now_s(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return ts_s(&t);
}

/* The frames between hw_ptr `from` and `to` left the ring for the DAC. */
static void consume(snd_pcm_uframes_t from, snd_pcm_uframes_t to)
{
	if (pcm.capture_fd < 0 || !pcm.ring)
		return;
	for (snd_pcm_uframes_t f = from; f < to;) {
		snd_pcm_uframes_t pos = f % pcm.buffer_size;
		snd_pcm_uframes_t run = pcm.buffer_size - pos;
		if (run > to - f)
			run = to - f;
		const unsigned char *src = pcm.ring + pos * pcm.frame_bytes;
		size_t left = run * pcm.frame_bytes;
		while (left) {
			ssize_t n = write(pcm.capture_fd, src, left);
			if (n <= 0)
				break;
			src += n;
			left -= (size_t)n;
		}
		f += run;
	}
}

/* Moves hw_ptr to where a DAC clocking at the negotiated rate is now. */
static void update_hw_ptr(void)
{
	if (pcm.state != SNDRV_PCM_STATE_RUNNING &&
	    pcm.state != SNDRV_PCM_STATE_DRAINING)
		return;
	double elapsed = now_s() - ts_s(&pcm.started);
	snd_pcm_uframes_t target =
		pcm.hw_at_start + (snd_pcm_uframes_t)(elapsed * pcm.rate);
	if (target > pcm.appl_ptr)
		target = pcm.appl_ptr;
	if (target > pcm.hw_ptr) {
		consume(pcm.hw_ptr, target);
		pcm.hw_ptr = target;
	}
	snd_pcm_uframes_t avail = pcm.buffer_size - (pcm.appl_ptr - pcm.hw_ptr);
	if (pcm.state == SNDRV_PCM_STATE_DRAINING) {
		if (pcm.hw_ptr >= pcm.appl_ptr)
			pcm.state = SNDRV_PCM_STATE_SETUP;
	} else if (avail >= pcm.stop_threshold && pcm.stop_threshold < pcm.boundary) {
		/* Underrun: the DAC ran out of samples. */
		pcm.state = SNDRV_PCM_STATE_XRUN;
		clock_gettime(CLOCK_MONOTONIC, &pcm.trigger);
	}
}

static void start_stream(void)
{
	pcm.state = SNDRV_PCM_STATE_RUNNING;
	clock_gettime(CLOCK_MONOTONIC, &pcm.started);
	pcm.trigger = pcm.started;
	pcm.hw_at_start = pcm.hw_ptr;
}

static void sleep_frames(snd_pcm_uframes_t frames)
{
	double seconds = (double)frames / (pcm.rate ? pcm.rate : 48000);
	if (seconds > 0.005)
		seconds = 0.005;
	if (seconds < 0.0005)
		seconds = 0.0005;
	struct timespec t = {0, (long)(seconds * 1e9)};
	nanosleep(&t, NULL);
}

static int pcm_writei(struct snd_xferi *x, int nonblock)
{
	snd_pcm_uframes_t done = 0;
	if (pcm.state == SNDRV_PCM_STATE_XRUN)
		return -EPIPE;
	if (pcm.state != SNDRV_PCM_STATE_PREPARED &&
	    pcm.state != SNDRV_PCM_STATE_RUNNING)
		return -EBADFD;
	while (done < x->frames) {
		update_hw_ptr();
		if (pcm.state == SNDRV_PCM_STATE_XRUN) {
			x->result = (snd_pcm_sframes_t)done;
			return done ? 0 : -EPIPE;
		}
		snd_pcm_uframes_t avail = pcm.buffer_size - (pcm.appl_ptr - pcm.hw_ptr);
		if (avail == 0) {
			if (nonblock) {
				if (done)
					break;
				return -EAGAIN;
			}
			pthread_mutex_unlock(&lock);
			sleep_frames(pcm.period_size);
			pthread_mutex_lock(&lock);
			if (!pcm.open || !pcm.configured)
				return -EBADFD;
			continue;
		}
		snd_pcm_uframes_t n = x->frames - done;
		if (n > avail)
			n = avail;
		const unsigned char *src =
			(const unsigned char *)x->buf + done * pcm.frame_bytes;
		for (snd_pcm_uframes_t i = 0; i < n;) {
			snd_pcm_uframes_t pos = (pcm.appl_ptr + i) % pcm.buffer_size;
			snd_pcm_uframes_t run = pcm.buffer_size - pos;
			if (run > n - i)
				run = n - i;
			memcpy(pcm.ring + pos * pcm.frame_bytes,
			       src + i * pcm.frame_bytes, run * pcm.frame_bytes);
			i += run;
		}
		pcm.appl_ptr += n;
		done += n;
		if (pcm.state == SNDRV_PCM_STATE_PREPARED &&
		    pcm.appl_ptr - pcm.hw_ptr >= pcm.start_threshold)
			start_stream();
	}
	x->result = (snd_pcm_sframes_t)done;
	return 0;
}

static int pcm_drain(int nonblock)
{
	if (!pcm.configured)
		return -EBADFD;
	if (pcm.state == SNDRV_PCM_STATE_PREPARED) {
		if (pcm.appl_ptr == pcm.hw_ptr) {
			pcm.state = SNDRV_PCM_STATE_SETUP;
			return 0;
		}
		start_stream();
	}
	if (pcm.state == SNDRV_PCM_STATE_XRUN) {
		pcm.state = SNDRV_PCM_STATE_SETUP;
		return 0;
	}
	if (pcm.state == SNDRV_PCM_STATE_RUNNING)
		pcm.state = SNDRV_PCM_STATE_DRAINING;
	if (nonblock)
		return -EAGAIN;
	while (pcm.state == SNDRV_PCM_STATE_DRAINING) {
		update_hw_ptr();
		if (pcm.state != SNDRV_PCM_STATE_DRAINING)
			break;
		snd_pcm_uframes_t left = pcm.appl_ptr - pcm.hw_ptr;
		pthread_mutex_unlock(&lock);
		sleep_frames(left);
		pthread_mutex_lock(&lock);
	}
	return 0;
}

static void fill_status(struct snd_pcm_status *s)
{
	update_hw_ptr();
	memset(s, 0, sizeof(*s));
	s->state = pcm.state;
	s->trigger_tstamp = pcm.trigger;
	clock_gettime(CLOCK_MONOTONIC, &s->tstamp);
	s->appl_ptr = pcm.appl_ptr;
	s->hw_ptr = pcm.hw_ptr;
	s->delay = (snd_pcm_sframes_t)(pcm.appl_ptr - pcm.hw_ptr);
	s->avail = pcm.configured ? pcm.buffer_size - (pcm.appl_ptr - pcm.hw_ptr) : 0;
	s->avail_max = s->avail;
}

static void fill_pcm_info(struct snd_pcm_info *info)
{
	memset(info, 0, sizeof(*info));
	info->stream = SNDRV_PCM_STREAM_PLAYBACK;
	info->card = CARD;
	snprintf((char *)info->id, sizeof(info->id), "USB Audio");
	snprintf((char *)info->name, sizeof(info->name), "USB Audio");
	snprintf((char *)info->subname, sizeof(info->subname), "subdevice #0");
	info->dev_class = SNDRV_PCM_CLASS_GENERIC;
	info->subdevices_count = 1;
	info->subdevices_avail = pcm.open ? 0 : 1;
}

static int pcm_hw_params(struct snd_pcm_hw_params *p)
{
	if (pcm.state != SNDRV_PCM_STATE_OPEN && pcm.state != SNDRV_PCM_STATE_SETUP &&
	    pcm.state != SNDRV_PCM_STATE_PREPARED)
		return -EBADFD;
	p->rmask = ~0U;
	int err = hw_refine(p);
	if (err < 0)
		return err;
	err = hw_choose(p);
	if (err < 0)
		return err;
	fixup_unreferenced(p);
	pcm.access = mask_min(hw_mask(p, P(ACCESS)));
	pcm.format = mask_min(hw_mask(p, P(FORMAT)));
	pcm.channels = iv_value(hw_iv(p, P(CHANNELS)));
	pcm.rate = iv_value(hw_iv(p, P(RATE)));
	pcm.period_size = iv_value(hw_iv(p, P(PERIOD_SIZE)));
	pcm.buffer_size = iv_value(hw_iv(p, P(BUFFER_SIZE)));
	pcm.frame_bytes = iv_value(hw_iv(p, P(FRAME_BITS))) / 8;
	if (!pcm.buffer_size || !pcm.frame_bytes || !pcm.rate)
		return -EINVAL;
	release_ring();
	pcm.ring = calloc(pcm.buffer_size, pcm.frame_bytes);
	if (!pcm.ring)
		return -ENOMEM;
	pcm.configured = 1;
	pcm.appl_ptr = pcm.hw_ptr = 0;
	pcm.avail_min = pcm.period_size;
	pcm.start_threshold = 1;
	pcm.stop_threshold = pcm.buffer_size;
	pcm.boundary = pcm.buffer_size;
	while (pcm.boundary * 2 <= (snd_pcm_uframes_t)LONG_MAX - pcm.buffer_size)
		pcm.boundary *= 2;
	pcm.state = SNDRV_PCM_STATE_SETUP;
	return 0;
}

static int pcm_ioctl(int fd, unsigned long request, void *arg)
{
	int nonblock = (fcntl(fd, F_GETFL) & O_NONBLOCK) != 0;
	switch (request) {
	case SNDRV_PCM_IOCTL_PVERSION:
		*(int *)arg = SNDRV_PCM_VERSION;
		return 0;
	case SNDRV_PCM_IOCTL_INFO:
		fill_pcm_info(arg);
		return 0;
	case SNDRV_PCM_IOCTL_TSTAMP:
	case SNDRV_PCM_IOCTL_TTSTAMP:
	case SNDRV_PCM_IOCTL_USER_PVERSION:
		return 0;
	case SNDRV_PCM_IOCTL_HW_REFINE:
		return hw_refine(arg);
	case SNDRV_PCM_IOCTL_HW_PARAMS:
		return pcm_hw_params(arg);
	case SNDRV_PCM_IOCTL_HW_FREE:
		if (pcm.state == SNDRV_PCM_STATE_RUNNING)
			return -EBADFD;
		pcm.configured = 0;
		release_ring();
		pcm.state = SNDRV_PCM_STATE_OPEN;
		return 0;
	case SNDRV_PCM_IOCTL_SW_PARAMS: {
		struct snd_pcm_sw_params *s = arg;
		if (!pcm.configured)
			return -EBADFD;
		if (s->avail_min == 0 || s->boundary == 0 || s->boundary % pcm.buffer_size)
			return -EINVAL;
		pcm.avail_min = s->avail_min;
		pcm.start_threshold = s->start_threshold;
		pcm.stop_threshold = s->stop_threshold;
		pcm.boundary = s->boundary;
		return 0;
	}
	case SNDRV_PCM_IOCTL_STATUS:
	case SNDRV_PCM_IOCTL_STATUS_EXT:
		fill_status(arg);
		return 0;
	case SNDRV_PCM_IOCTL_DELAY:
		update_hw_ptr();
		if (pcm.state == SNDRV_PCM_STATE_XRUN)
			return -EPIPE;
		*(snd_pcm_sframes_t *)arg = (snd_pcm_sframes_t)(pcm.appl_ptr - pcm.hw_ptr);
		return 0;
	case SNDRV_PCM_IOCTL_HWSYNC:
		update_hw_ptr();
		return pcm.state == SNDRV_PCM_STATE_XRUN ? -EPIPE : 0;
	case SNDRV_PCM_IOCTL_SYNC_PTR: {
		struct snd_pcm_sync_ptr *s = arg;
		update_hw_ptr();
		if (s->flags & SNDRV_PCM_SYNC_PTR_APPL)
			s->c.control.appl_ptr = pcm.appl_ptr;
		else if (pcm.configured)
			pcm.appl_ptr = s->c.control.appl_ptr;
		if (s->flags & SNDRV_PCM_SYNC_PTR_AVAIL_MIN)
			s->c.control.avail_min = pcm.avail_min;
		else
			pcm.avail_min = s->c.control.avail_min;
		s->s.status.state = pcm.state;
		s->s.status.hw_ptr = pcm.hw_ptr;
		clock_gettime(CLOCK_MONOTONIC, &s->s.status.tstamp);
		s->s.status.suspended_state = 0;
		return 0;
	}
	case SNDRV_PCM_IOCTL_PREPARE:
		if (!pcm.configured)
			return -EBADFD;
		pcm.appl_ptr = pcm.hw_ptr = 0;
		pcm.state = SNDRV_PCM_STATE_PREPARED;
		return 0;
	case SNDRV_PCM_IOCTL_RESET:
		pcm.appl_ptr = pcm.hw_ptr;
		return 0;
	case SNDRV_PCM_IOCTL_START:
		if (pcm.state != SNDRV_PCM_STATE_PREPARED)
			return -EBADFD;
		start_stream();
		return 0;
	case SNDRV_PCM_IOCTL_DROP:
		if (!pcm.configured)
			return -EBADFD;
		update_hw_ptr();
		pcm.state = SNDRV_PCM_STATE_SETUP;
		pcm.appl_ptr = pcm.hw_ptr;
		return 0;
	case SNDRV_PCM_IOCTL_DRAIN:
		return pcm_drain(nonblock);
	case SNDRV_PCM_IOCTL_PAUSE: {
		int enable = (int)(intptr_t)arg;
		update_hw_ptr();
		if (enable && pcm.state == SNDRV_PCM_STATE_RUNNING)
			pcm.state = SNDRV_PCM_STATE_PAUSED;
		else if (!enable && pcm.state == SNDRV_PCM_STATE_PAUSED)
			start_stream();
		else
			return -EBADFD;
		return 0;
	}
	case SNDRV_PCM_IOCTL_WRITEI_FRAMES:
		if (pcm.access != SNDRV_PCM_ACCESS_RW_INTERLEAVED)
			return -EINVAL;
		return pcm_writei(arg, nonblock);
	case SNDRV_PCM_IOCTL_CHANNEL_INFO: {
		struct snd_pcm_channel_info *c = arg;
		if (!pcm.configured || c->channel >= pcm.channels)
			return -EINVAL;
		unsigned int bits = (unsigned int)phys_width(pcm.format);
		c->offset = SNDRV_PCM_MMAP_OFFSET_DATA;
		c->first = c->channel * bits;
		c->step = pcm.frame_bytes * 8;
		return 0;
	}
	case SNDRV_PCM_IOCTL_REWIND:
	case SNDRV_PCM_IOCTL_FORWARD:
		*(snd_pcm_uframes_t *)arg = 0;
		return 0;
	case SNDRV_PCM_IOCTL_XRUN:
		pcm.state = SNDRV_PCM_STATE_XRUN;
		return 0;
	default:
		return -ENOTTY;
	}
}

static int ctl_ioctl(unsigned long request, void *arg)
{
	switch (request) {
	case SNDRV_CTL_IOCTL_PVERSION:
		*(int *)arg = SNDRV_CTL_VERSION;
		return 0;
	case SNDRV_CTL_IOCTL_CARD_INFO: {
		struct snd_ctl_card_info *c = arg;
		memset(c, 0, sizeof(*c));
		c->card = CARD;
		snprintf((char *)c->id, sizeof(c->id), CARD_ID);
		snprintf((char *)c->driver, sizeof(c->driver), "USB-Audio");
		snprintf((char *)c->name, sizeof(c->name), "Bench DAC");
		snprintf((char *)c->longname, sizeof(c->longname),
			 PRODUCT " at " BUS_PATH ", high speed");
		snprintf((char *)c->mixername, sizeof(c->mixername), "USB Mixer");
		snprintf((char *)c->components, sizeof(c->components), "USB" USB_ID);
		return 0;
	}
	case SNDRV_CTL_IOCTL_PCM_NEXT_DEVICE: {
		int *device = arg;
		*device = *device < 0 ? 0 : -1;
		return 0;
	}
	case SNDRV_CTL_IOCTL_HWDEP_NEXT_DEVICE:
	case SNDRV_CTL_IOCTL_RAWMIDI_NEXT_DEVICE:
		*(int *)arg = -1;
		return 0;
	case SNDRV_CTL_IOCTL_PCM_INFO: {
		struct snd_pcm_info *info = arg;
		if (info->device != 0 || info->stream != SNDRV_PCM_STREAM_PLAYBACK ||
		    info->subdevice > 0)
			return -ENOENT;
		fill_pcm_info(info);
		return 0;
	}
	case SNDRV_CTL_IOCTL_PCM_PREFER_SUBDEVICE:
	case SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS:
		return 0;
	case SNDRV_CTL_IOCTL_ELEM_LIST: {
		struct snd_ctl_elem_list *l = arg;
		l->used = 0;
		l->count = 0;
		return 0;
	}
	default:
		return -ENOTTY;
	}
}

/* ---- /proc/asound ------------------------------------------------------- */

static void append(char *buf, size_t size, const char *fmt, ...)
{
	size_t used = strlen(buf);
	if (used >= size)
		return;
	va_list ap;
	va_start(ap, fmt);
	vsnprintf(buf + used, size - used, fmt, ap);
	va_end(ap);
}

/* snd-usb-audio proc.c: proc_dump_substream_status + _formats. */
static void render_stream(char *buf, size_t size)
{
	update_hw_ptr();
	buf[0] = 0;
	append(buf, size, "%s at %s, high speed : USB Audio\n\nPlayback:\n", PRODUCT,
	       BUS_PATH);
	int running = pcm.open && pcm.configured &&
		      (pcm.state == SNDRV_PCM_STATE_RUNNING ||
		       pcm.state == SNDRV_PCM_STATE_DRAINING ||
		       pcm.state == SNDRV_PCM_STATE_PREPARED);
	if (running) {
		int alt = 0;
		for (int k = 0; k < NALTSETS; k++)
			if (altsets[k].format == pcm.format)
				alt = altsets[k].altset;
		/* Q16.16 frames per 125us microframe, as proc.c prints it. */
		unsigned long q = ((unsigned long)pcm.rate << 16) / 8000;
		append(buf, size,
		       "  Status: Running\n    Interface = 1\n    Altset = %d\n"
		       "    Packet Size = %u\n    Momentary freq = %u Hz (%#lx.%04lx)\n"
		       "    Feedback Format = 16.16\n",
		       alt, (pcm.rate / 8000 + 1) * pcm.frame_bytes, pcm.rate, q >> 16,
		       q & 0xffff);
	} else {
		append(buf, size, "  Status: Stop\n");
	}
	for (int k = 0; k < NALTSETS; k++) {
		append(buf, size,
		       "  Interface 1\n    Altset %d\n    Format: %s\n    Channels: %d\n"
		       "    Endpoint: 0x01 (1 OUT) (ASYNC)\n    Rates: ",
		       altsets[k].altset, format_name(altsets[k].format), CHANNELS);
		for (int r = 0; r < altsets[k].nrates; r++)
			append(buf, size, "%s%u", r ? ", " : "", altsets[k].rates[r]);
		append(buf, size,
		       "\n    Data packet interval: 125 us\n    Bits: %d\n"
		       "    Channel map: FL FR\n    Sync Endpoint: 0x81 (1 IN)\n"
		       "    Sync EP Interface: 1\n    Sync EP Altset: %d\n"
		       "    Implicit Feedback Mode: No\n",
		       altsets[k].bits, altsets[k].altset);
	}
}

/* pcm.c snd_pcm_substream_proc_hw_params_read */
static void render_hw_params(char *buf, size_t size)
{
	buf[0] = 0;
	if (!pcm.open || !pcm.configured) {
		append(buf, size, "closed\n");
		return;
	}
	append(buf, size,
	       "access: RW_INTERLEAVED\nformat: %s\nsubformat: STD\nchannels: %u\n"
	       "rate: %u (%u/1)\nperiod_size: %lu\nbuffer_size: %lu\n",
	       format_name(pcm.format), pcm.channels, pcm.rate, pcm.rate,
	       (unsigned long)pcm.period_size, (unsigned long)pcm.buffer_size);
}

/* The file under /proc/asound/card1 (or /proc/asound/BenchDAC), or NULL. */
static const char *card_relative(const char *path)
{
	static const char *roots[] = {"/proc/asound/card1/", "/proc/asound/" CARD_ID "/"};
	for (int i = 0; i < 2; i++) {
		size_t len = strlen(roots[i]);
		if (strncmp(path, roots[i], len) == 0)
			return path + len;
	}
	return NULL;
}

static int memfd_with(const char *text)
{
	int fd = memfd_create("parson-asound-proc", MFD_CLOEXEC);
	if (fd < 0)
		return -2;
	size_t len = strlen(text);
	if (write(fd, text, len) != (ssize_t)len) {
		real_close(fd);
		return -2;
	}
	lseek(fd, 0, SEEK_SET);
	return fd;
}

/* -1: not ours. -2: ours, failed with errno. Otherwise a readable fd. */
static int open_proc(const char *path)
{
	static char buf[16384];
	const char *rel;
	if (strcmp(path, "/proc/asound/cards") == 0) {
		snprintf(buf, sizeof(buf),
			 " %d [%-15s]: USB-Audio - Bench DAC\n"
			 "                      %s at %s, high speed\n",
			 CARD, CARD_ID, PRODUCT, BUS_PATH);
	} else if ((rel = card_relative(path))) {
		if (strcmp(rel, "usbid") == 0)
			snprintf(buf, sizeof(buf), "%s\n", USB_ID);
		else if (strcmp(rel, "id") == 0)
			snprintf(buf, sizeof(buf), "%s\n", CARD_ID);
		else if (strcmp(rel, "stream0") == 0)
			render_stream(buf, sizeof(buf));
		else if (strcmp(rel, "pcm0p/sub0/hw_params") == 0)
			render_hw_params(buf, sizeof(buf));
		else
			return errno = ENOENT, -2;
	} else {
		return -1;
	}
	return memfd_with(buf);
}

static int open_node(const char *path, int flags)
{
	int kind;
	if (strcmp(path, "/dev/snd/controlC1") == 0)
		kind = NODE_CTL;
	else if (strcmp(path, "/dev/snd/pcmC1D0p") == 0)
		kind = NODE_PCM;
	else if (strncmp(path, "/dev/snd/", 9) == 0 && strstr(path, "C1"))
		return errno = ENOENT, -2;
	else
		return -1;
	if (kind == NODE_PCM && pcm.open)
		return errno = EBUSY, -2;
	int fd = real_open("/dev/null", (flags & (O_NONBLOCK | O_CLOEXEC)) | O_RDWR);
	if (fd < 0)
		return -2;
	if (fd >= (int)sizeof(node_of)) {
		real_close(fd);
		return errno = EMFILE, -2;
	}
	node_of[fd] = (unsigned char)kind;
	if (kind == NODE_PCM) {
		pcm.open = 1;
		pcm.fd = fd;
		pcm.state = SNDRV_PCM_STATE_OPEN;
		pcm.configured = 0;
		const char *capture = getenv("PARSON_SND_EMULATOR_CAPTURE");
		if (capture && *capture)
			pcm.capture_fd = real_open(
				capture, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
	}
	return fd;
}

static int handle_open(const char *path, int flags)
{
	resolve();
	if (!path || path[0] != '/')
		return -1;
	if (strncmp(path, "/dev/snd/", 9) != 0 && strncmp(path, "/proc/asound/", 13) != 0)
		return -1;
	pthread_mutex_lock(&lock);
	int fd = open_node(path, flags);
	if (fd == -1)
		fd = open_proc(path);
	pthread_mutex_unlock(&lock);
	return fd;
}

/* ---- interposed libc entry points --------------------------------------- */

static mode_t mode_arg(int flags, va_list ap)
{
	if (flags & (O_CREAT | O_TMPFILE))
		return va_arg(ap, mode_t);
	return 0;
}

int open(const char *path, int flags, ...)
{
	va_list ap;
	va_start(ap, flags);
	mode_t mode = mode_arg(flags, ap);
	va_end(ap);
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_open(path, flags, mode);
}

int open64(const char *path, int flags, ...)
{
	va_list ap;
	va_start(ap, flags);
	mode_t mode = mode_arg(flags, ap);
	va_end(ap);
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_open(path, flags, mode);
}

int __open_2(const char *path, int flags)
{
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_open(path, flags);
}

int __open64_2(const char *path, int flags)
{
	return __open_2(path, flags);
}

int openat(int dirfd, const char *path, int flags, ...)
{
	va_list ap;
	va_start(ap, flags);
	mode_t mode = mode_arg(flags, ap);
	va_end(ap);
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_openat(dirfd, path, flags, mode);
}

int openat64(int dirfd, const char *path, int flags, ...)
{
	va_list ap;
	va_start(ap, flags);
	mode_t mode = mode_arg(flags, ap);
	va_end(ap);
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_openat(dirfd, path, flags, mode);
}

int __openat_2(int dirfd, const char *path, int flags)
{
	int fd = handle_open(path, flags);
	if (fd != -1)
		return fd < 0 ? -1 : fd;
	return real_openat(dirfd, path, flags);
}

int __openat64_2(int dirfd, const char *path, int flags)
{
	return __openat_2(dirfd, path, flags);
}

int close(int fd)
{
	resolve();
	if (fd >= 0 && fd < (int)sizeof(node_of) && node_of[fd]) {
		pthread_mutex_lock(&lock);
		if (node_of[fd] == NODE_PCM && pcm.open && pcm.fd == fd) {
			update_hw_ptr();
			pcm.open = 0;
			pcm.configured = 0;
			pcm.state = SNDRV_PCM_STATE_OPEN;
			release_ring();
			if (pcm.capture_fd >= 0) {
				real_close(pcm.capture_fd);
				pcm.capture_fd = -1;
			}
		}
		node_of[fd] = NODE_NONE;
		pthread_mutex_unlock(&lock);
	}
	return real_close(fd);
}

int ioctl(int fd, unsigned long request, ...)
{
	va_list ap;
	va_start(ap, request);
	void *arg = va_arg(ap, void *);
	va_end(ap);
	resolve();
	if (fd < 0 || fd >= (int)sizeof(node_of) || !node_of[fd])
		return real_ioctl(fd, request, arg);
	pthread_mutex_lock(&lock);
	int result = node_of[fd] == NODE_CTL ? ctl_ioctl(request, arg)
					     : pcm_ioctl(fd, request, arg);
	pthread_mutex_unlock(&lock);
	if (result < 0) {
		errno = -result;
		return -1;
	}
	return result;
}

/* The status and control pages cannot be mapped, so libasound uses
 * SNDRV_PCM_IOCTL_SYNC_PTR, as on kernels that refuse the mapping. */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset)
{
	resolve();
	if (fd >= 0 && fd < (int)sizeof(node_of) && node_of[fd]) {
		/* The DMA buffer (offset 0) maps; status and control do not. */
		pthread_mutex_lock(&lock);
		void *mapped = MAP_FAILED;
		if (node_of[fd] == NODE_PCM && offset == SNDRV_PCM_MMAP_OFFSET_DATA &&
		    pcm.configured &&
		    length >= (size_t)pcm.buffer_size * pcm.frame_bytes) {
			mapped = real_mmap(NULL, length, PROT_READ | PROT_WRITE,
					   MAP_SHARED | MAP_ANONYMOUS, -1, 0);
			if (mapped != MAP_FAILED) {
				memcpy(mapped, pcm.ring, (size_t)pcm.buffer_size * pcm.frame_bytes);
				release_ring();
				pcm.ring = mapped;
				pcm.ring_mapped = length;
			}
		}
		pthread_mutex_unlock(&lock);
		if (mapped == MAP_FAILED)
			errno = ENXIO;
		return mapped;
	}
	return real_mmap(addr, length, prot, flags, fd, offset);
}

int munmap(void *addr, size_t length)
{
	static int (*real_munmap)(void *, size_t);
	if (!real_munmap)
		real_munmap = dlsym(RTLD_NEXT, "munmap");
	pthread_mutex_lock(&lock);
	int ours = pcm.ring_mapped && addr == pcm.ring;
	if (ours) {
		/* The program lets go of the DMA buffer; keep a private copy. */
		unsigned char *copy = malloc(pcm.ring_mapped);
		if (copy)
			memcpy(copy, pcm.ring, pcm.ring_mapped);
		size_t mapped = pcm.ring_mapped;
		pcm.ring = copy;
		pcm.ring_mapped = 0;
		pthread_mutex_unlock(&lock);
		return real_munmap(addr, mapped);
	}
	pthread_mutex_unlock(&lock);
	return real_munmap(addr, length);
}

/* Playback is writable when avail_min frames are free, or the stream has
 * stopped (POLLERR on xrun), as snd_pcm_playback_poll reports it. */
static short pcm_revents(void)
{
	update_hw_ptr();
	if (!pcm.configured)
		return POLLOUT | POLLWRNORM;
	if (pcm.state == SNDRV_PCM_STATE_XRUN)
		return POLLOUT | POLLWRNORM | POLLERR;
	snd_pcm_uframes_t avail = pcm.buffer_size - (pcm.appl_ptr - pcm.hw_ptr);
	if (pcm.state != SNDRV_PCM_STATE_RUNNING &&
	    pcm.state != SNDRV_PCM_STATE_DRAINING)
		return POLLOUT | POLLWRNORM;
	return avail >= pcm.avail_min ? (POLLOUT | POLLWRNORM) : 0;
}

int poll(struct pollfd *fds, nfds_t nfds, int timeout)
{
	static int (*real_poll)(struct pollfd *, nfds_t, int);
	if (!real_poll)
		real_poll = dlsym(RTLD_NEXT, "poll");
	int ours = 0;
	for (nfds_t i = 0; i < nfds; i++)
		if (fds[i].fd >= 0 && fds[i].fd < (int)sizeof(node_of) && node_of[fds[i].fd] == NODE_PCM)
			ours = 1;
	if (!ours)
		return real_poll(fds, nfds, timeout);
	double deadline = timeout < 0 ? -1 : now_s() + timeout / 1000.0;
	for (;;) {
		int ready = 0;
		for (nfds_t i = 0; i < nfds; i++) {
			int fd = fds[i].fd;
			fds[i].revents = 0;
			if (fd >= 0 && fd < (int)sizeof(node_of) && node_of[fd] == NODE_PCM) {
				pthread_mutex_lock(&lock);
				fds[i].revents = pcm_revents() & (fds[i].events | POLLERR);
				pthread_mutex_unlock(&lock);
			} else if (fd >= 0) {
				struct pollfd one = fds[i];
				if (real_poll(&one, 1, 0) > 0)
					fds[i].revents = one.revents;
			}
			if (fds[i].revents)
				ready++;
		}
		if (ready || (deadline >= 0 && now_s() >= deadline))
			return ready;
		struct timespec t = {0, 1000000};
		nanosleep(&t, NULL);
	}
}

void *mmap64(void *addr, size_t length, int prot, int flags, int fd, off_t offset)
{
	return mmap(addr, length, prot, flags, fd, offset);
}

__attribute__((constructor)) static void emulator_init(void)
{
	resolve();
	init_constraints();
}
