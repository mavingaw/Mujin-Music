"use client";

import {
  type Appearance,
  type HomeRowId,
  CORNER_RADIUS_RANGE,
  DEFAULT_CORNER_RADIUS,
  FONT_CHOICES,
  FONT_LABELS,
  HOME_ROW_LABELS,
  isDefaultAppearance,
  moveHomeRow,
  NOW_PLAYING_LAYOUTS,
  NOW_PLAYING_LAYOUT_LABELS,
  placeHomeRow,
  PRESET_DESCRIPTIONS,
  PRESET_LABELS,
  hexFromArgb,
  TEXT_SCALE_RANGE,
  THEME_PRESETS,
  type ThemePreset,
} from "@parson/music-sdk";
import {
  ArrowDown,
  ArrowUp,
  Check,
  Eye,
  EyeOff,
  GripVertical,
  Monitor,
  Moon,
  RotateCcw,
  Sparkles,
  Sun,
} from "lucide-react";
import { useMemo, useState, type DragEvent, type ReactNode } from "react";
import { useSession } from "@/features/account/session-provider";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import { usePlayer } from "@/features/player/player-api";
import { cn } from "@/lib/cn";
import {
  ACCENT_SWATCHES,
  contrastGrade,
  FONT_SAMPLE_STACKS,
  headlineChecks,
  previewStyle,
  syncLabel,
} from "./appearance-model";
import { useAppearance } from "./theme-provider";

function Section({
  children,
  description,
  id,
  title,
}: {
  children: ReactNode;
  description?: string;
  id: string;
  title: string;
}) {
  return (
    <section aria-labelledby={id} className="border-t border-white/[0.08] py-8">
      <h2 className="mj-title text-xl text-mujin-text" id={id}>
        {title}
      </h2>
      {description && (
        <p className="mt-1.5 max-w-2xl text-[0.8125rem] leading-5 text-mujin-text-tertiary">
          {description}
        </p>
      )}
      <div className="mt-5">{children}</div>
    </section>
  );
}

function Segmented<T extends string>({
  label,
  onChange,
  options,
  value,
}: {
  label: string;
  onChange: (value: T) => void;
  options: { value: T; label: string; icon?: ReactNode; disabled?: boolean }[];
  value: T;
}) {
  return (
    <div
      aria-label={label}
      className="inline-flex flex-wrap gap-1 rounded-full border border-white/[0.08] bg-mujin-surface-1 p-1"
      role="radiogroup"
    >
      {options.map((option) => (
        <button
          aria-checked={value === option.value}
          className={cn(
            "inline-flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[0.8125rem] font-medium transition-colors disabled:opacity-40",
            value === option.value
              ? "bg-mujin-copper text-mujin-on-copper"
              : "text-mujin-text-secondary hover:bg-white/[0.06] hover:text-mujin-text",
          )}
          disabled={option.disabled}
          key={option.value}
          onClick={() => onChange(option.value)}
          role="radio"
          type="button"
        >
          {option.icon}
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ToggleRow({
  checked,
  description,
  label,
  onChange,
}: {
  checked: boolean;
  description: string;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-3">
      <div>
        <p className="text-sm font-medium text-mujin-text">{label}</p>
        <p className="mt-1 max-w-lg text-xs leading-5 text-mujin-text-tertiary">
          {description}
        </p>
      </div>
      <RadioSwitch checked={checked} label={label} onChange={onChange} />
    </div>
  );
}

/** A small picture of a preset, drawn in that preset's own colours. */
function PresetCard({
  active,
  onSelect,
  preset,
  style,
}: {
  active: boolean;
  onSelect: () => void;
  preset: ThemePreset;
  style: React.CSSProperties;
}) {
  const glass = preset === "frosted-glass";
  return (
    <button
      aria-pressed={active}
      className={cn(
        "group relative flex flex-col overflow-hidden rounded-xl border text-left transition-[border-color,box-shadow]",
        active
          ? "border-mujin-copper shadow-[0_0_0_1px_var(--color-mujin-copper)]"
          : "border-white/[0.08] hover:border-white/25",
      )}
      data-preset-card={preset}
      onClick={onSelect}
      type="button"
    >
      <div
        className="mj-theme-preview relative h-32 w-full overflow-hidden"
        style={style}
      >
        {glass && (
          <div
            aria-hidden="true"
            className="absolute inset-0"
            style={{
              background:
                "radial-gradient(80% 90% at 20% 20%, rgb(var(--mj-accent-rgb) / 0.55), transparent 70%), radial-gradient(70% 80% at 90% 90%, rgb(108 159 208 / 0.45), transparent 70%), var(--color-mujin-deep)",
            }}
          />
        )}
        <div className="absolute inset-y-0 left-0 w-[26%] border-r border-[color:var(--color-mujin-border)] bg-mujin-raised/90 p-2 backdrop-blur-md">
          <span className="block h-1.5 w-8 rounded-full bg-mujin-text/70" />
          <span className="mt-3 block h-1 w-10 rounded-full bg-mujin-text-tertiary/60" />
          <span className="mt-1.5 block h-1 w-7 rounded-full bg-mujin-copper" />
          <span className="mt-1.5 block h-1 w-9 rounded-full bg-mujin-text-tertiary/60" />
        </div>
        <div
          className={cn(
            "absolute inset-y-0 left-[26%] right-0 p-3",
            glass ? "bg-mujin-base/40" : "bg-mujin-base",
          )}
        >
          <span className="block font-[family-name:var(--font-display)] text-[0.95rem] font-light leading-none text-mujin-text">
            Night Swim
          </span>
          <span className="mt-1 block text-[0.6rem] text-mujin-text-secondary">
            Mira Sol · 2024
          </span>
          <div className="mt-2.5 flex gap-1.5">
            {[0, 1, 2].map((tile) => (
              <span
                className="block aspect-square w-[30%] rounded-[calc(4px*var(--mj-radius-scale,1))] bg-mujin-surface-3"
                key={tile}
                style={
                  tile === 0
                    ? {
                        background:
                          "linear-gradient(135deg, var(--color-mujin-copper), var(--color-mujin-surface-2))",
                      }
                    : undefined
                }
              />
            ))}
          </div>
          <span className="absolute bottom-2.5 right-3 inline-flex h-5 items-center rounded-full bg-mujin-copper px-2 text-[0.55rem] font-semibold text-mujin-on-copper">
            Play
          </span>
        </div>
      </div>
      <div className="flex flex-1 flex-col gap-1 bg-mujin-surface-1 p-3">
        <span className="flex items-center gap-1.5 text-sm font-medium text-mujin-text">
          {active && <Check className="h-3.5 w-3.5 text-mujin-copper" />}
          {PRESET_LABELS[preset]}
        </span>
        <span className="text-xs leading-[1.1rem] text-mujin-text-tertiary">
          {PRESET_DESCRIPTIONS[preset]}
        </span>
      </div>
    </button>
  );
}

function NowPlayingLayoutPicture({
  layout,
}: {
  layout: (typeof NOW_PLAYING_LAYOUTS)[number];
}) {
  return (
    <div className="relative h-20 w-full overflow-hidden rounded-lg bg-mujin-deep">
      {layout === "immersive" && (
        <div
          aria-hidden="true"
          className="absolute inset-0 opacity-70"
          style={{
            background:
              "linear-gradient(135deg, var(--color-mujin-copper), var(--color-mujin-surface-2) 70%)",
          }}
        />
      )}
      {layout === "art-left" && (
        <>
          <span className="absolute left-3 top-3 h-14 w-14 rounded-md bg-mujin-copper/80" />
          <span className="absolute left-20 top-5 h-2 w-16 rounded-full bg-mujin-text/80" />
          <span className="absolute left-20 top-9 h-1.5 w-10 rounded-full bg-mujin-text-tertiary" />
        </>
      )}
      {layout === "art-centred" && (
        <>
          <span className="absolute left-1/2 top-2 h-11 w-11 -translate-x-1/2 rounded-md bg-mujin-copper/80" />
          <span className="absolute left-1/2 top-15 h-1.5 w-14 -translate-x-1/2 rounded-full bg-mujin-text/80" />
        </>
      )}
      {layout === "immersive" && (
        <>
          <span className="absolute bottom-6 left-3 h-2.5 w-20 rounded-full bg-mujin-text/90" />
          <span className="absolute bottom-3 left-3 h-1.5 w-12 rounded-full bg-mujin-text/60" />
        </>
      )}
    </div>
  );
}

function HomeRowsEditor({
  onChange,
  rows,
}: {
  onChange: (rows: Appearance["home_rows"]) => void;
  rows: Appearance["home_rows"];
}) {
  const [dragging, setDragging] = useState<HomeRowId | null>(null);
  const onDrop = (event: DragEvent<HTMLLIElement>, index: number) => {
    event.preventDefault();
    if (dragging) onChange(placeHomeRow(rows, dragging, index));
    setDragging(null);
  };
  return (
    <ol className="max-w-xl divide-y divide-white/[0.06] overflow-hidden rounded-xl border border-white/[0.08] bg-mujin-surface-1">
      {rows.map((row, index) => (
        <li
          className={cn(
            "flex h-12 items-center gap-2 pl-2 pr-2 transition-colors",
            dragging === row.id && "opacity-50",
            !row.visible && "bg-mujin-base/40",
          )}
          data-home-row={row.id}
          draggable
          key={row.id}
          onDragEnd={() => setDragging(null)}
          onDragOver={(event) => event.preventDefault()}
          onDragStart={(event) => {
            setDragging(row.id);
            event.dataTransfer.effectAllowed = "move";
          }}
          onDrop={(event) => onDrop(event, index)}
        >
          <GripVertical
            aria-hidden="true"
            className="h-4 w-4 shrink-0 cursor-grab text-mujin-text-tertiary"
          />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-sm",
              row.visible ? "text-mujin-text" : "text-mujin-text-tertiary",
            )}
          >
            {HOME_ROW_LABELS[row.id]}
          </span>
          <button
            aria-label={`Move ${HOME_ROW_LABELS[row.id]} up`}
            className="grid h-8 w-8 place-items-center rounded-full text-mujin-text-tertiary hover:bg-white/[0.06] hover:text-mujin-text disabled:opacity-30"
            disabled={index === 0}
            onClick={() => onChange(moveHomeRow(rows, row.id, -1))}
            type="button"
          >
            <ArrowUp className="h-4 w-4" />
          </button>
          <button
            aria-label={`Move ${HOME_ROW_LABELS[row.id]} down`}
            className="grid h-8 w-8 place-items-center rounded-full text-mujin-text-tertiary hover:bg-white/[0.06] hover:text-mujin-text disabled:opacity-30"
            disabled={index === rows.length - 1}
            onClick={() => onChange(moveHomeRow(rows, row.id, 1))}
            type="button"
          >
            <ArrowDown className="h-4 w-4" />
          </button>
          <button
            aria-label={`${row.visible ? "Hide" : "Show"} ${HOME_ROW_LABELS[row.id]}`}
            aria-pressed={row.visible}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-medium",
              row.visible
                ? "text-mujin-text-secondary hover:bg-white/[0.06]"
                : "bg-white/[0.06] text-mujin-text-tertiary",
            )}
            onClick={() =>
              onChange(
                rows.map((entry) =>
                  entry.id === row.id
                    ? { ...entry, visible: !entry.visible }
                    : entry,
                ),
              )
            }
            type="button"
          >
            {row.visible ? (
              <Eye className="h-3.5 w-3.5" />
            ) : (
              <EyeOff className="h-3.5 w-3.5" />
            )}
            {row.visible ? "Shown" : "Hidden"}
          </button>
        </li>
      ))}
    </ol>
  );
}

export default function AppearanceSettings() {
  const { appearance, environment, status, theme, update, reset } =
    useAppearance();
  const { session } = useSession();
  const { imageSrc } = usePlayer();
  const [hexDraft, setHexDraft] = useState<string | null>(null);
  const previews = useMemo(
    () =>
      THEME_PRESETS.map((preset) => ({
        preset,
        // Each preset shown in its own light: Mujin Dark dark, Mujin
        // Light light, the rest as the chosen light or dark setting.
        ...previewStyle(
          preset,
          {
            ...appearance,
            color_scheme:
              preset === "mujin-dark"
                ? "dark"
                : preset === "mujin-light"
                  ? "light"
                  : appearance.color_scheme,
          },
          environment,
        ),
      })),
    [appearance, environment],
  );
  const checks = headlineChecks(theme);
  const adjusted = theme.checks.some(
    (check) => check.adjusted && check.name.startsWith("Accent"),
  );
  const choosePreset = (preset: ThemePreset) => {
    if (preset === "mujin-light") {
      update({ preset, color_scheme: "light" });
    } else if (preset === "mujin-dark") {
      update({
        preset,
        color_scheme:
          appearance.color_scheme === "light"
            ? "dark"
            : appearance.color_scheme,
      });
    } else if (preset === "material-you" && !appearance.accent) {
      // Material You starts from the album art, as on Android.
      update({ preset, accent_from_artwork: true });
    } else {
      update({ preset });
    }
  };
  const darkOnly = appearance.preset === "oled-black";
  const accentValue = appearance.accent ?? "#d98b5f";

  return (
    <div className="max-w-[880px]" data-appearance-settings>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h2 className="mj-title text-2xl text-mujin-text">Appearance</h2>
          <p className="mt-1.5 max-w-xl text-[0.8125rem] leading-5 text-mujin-text-tertiary">
            Every change shows at once. Your choices are saved to this profile
            and follow it to Mujin Music on other devices.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <p
            aria-live="polite"
            className="max-w-[16rem] text-xs leading-5 text-mujin-text-tertiary"
            data-sync-status={status}
          >
            {syncLabel(status, Boolean(session))}
          </p>
          <button
            className="inline-flex h-9 items-center gap-1.5 rounded-full border border-white/15 px-3.5 text-xs font-semibold text-mujin-text-secondary hover:border-white/35 hover:text-mujin-text disabled:opacity-40"
            disabled={isDefaultAppearance(appearance)}
            onClick={reset}
            type="button"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Reset
          </button>
        </div>
      </div>

      <Section
        description="Six looks, each built from the same tokens: every surface, line and word reads its colour from the theme."
        id="appearance-theme"
        title="Theme"
      >
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {previews.map(({ preset, style }) => (
            <PresetCard
              active={appearance.preset === preset}
              key={preset}
              onSelect={() => choosePreset(preset)}
              preset={preset}
              style={style}
            />
          ))}
        </div>
        <div className="mt-6 flex flex-wrap items-center gap-4">
          <Segmented
            label="Light or dark"
            onChange={(color_scheme) =>
              update({
                color_scheme,
                ...(appearance.preset === "mujin-light" &&
                color_scheme === "dark"
                  ? { preset: "mujin-dark" as const }
                  : appearance.preset === "mujin-dark" &&
                      color_scheme === "light"
                    ? { preset: "mujin-light" as const }
                    : {}),
              })
            }
            options={[
              {
                value: "system",
                label: "Match system",
                icon: <Monitor className="h-3.5 w-3.5" />,
                disabled: darkOnly,
              },
              {
                value: "dark",
                label: "Dark",
                icon: <Moon className="h-3.5 w-3.5" />,
              },
              {
                value: "light",
                label: "Light",
                icon: <Sun className="h-3.5 w-3.5" />,
                disabled: darkOnly,
              },
            ]}
            value={darkOnly ? "dark" : appearance.color_scheme}
          />
          <span className="text-xs text-mujin-text-tertiary">
            Showing {theme.label}, {theme.mode}
            {appearance.color_scheme === "system" && !darkOnly
              ? " (follows your system)"
              : ""}
          </span>
        </div>
      </Section>

      <Section
        description="One accent colours Play, selection and the HI-RES marks. Mujin Music checks every colour against WCAG and moves its tone until text stays readable."
        id="appearance-colour"
        title="Colour"
      >
        <div className="flex flex-wrap items-center gap-2.5">
          {ACCENT_SWATCHES.map((swatch) => {
            const active =
              !appearance.accent_from_artwork &&
              (appearance.accent ?? null) === swatch.value;
            return (
              <button
                aria-label={swatch.name}
                aria-pressed={active}
                className={cn(
                  "relative grid h-9 w-9 place-items-center rounded-full ring-offset-2 ring-offset-[var(--color-mujin-base)] transition-shadow",
                  active
                    ? "ring-2 ring-mujin-text"
                    : "hover:ring-1 hover:ring-white/40",
                )}
                key={swatch.name}
                onClick={() =>
                  update({ accent: swatch.value, accent_from_artwork: false })
                }
                style={{ background: swatch.value ?? "#d98b5f" }}
                title={swatch.name}
                type="button"
              >
                {active && <Check className="h-4 w-4 text-black/70" />}
              </button>
            );
          })}
          <label className="relative inline-flex h-9 items-center gap-2 rounded-full border border-white/12 bg-mujin-surface-1 pl-1 pr-3 text-xs text-mujin-text-secondary">
            <input
              aria-label="Custom accent colour"
              className="h-7 w-7 cursor-pointer appearance-none rounded-full border-0 bg-transparent p-0 [&::-webkit-color-swatch]:rounded-full [&::-webkit-color-swatch]:border-0 [&::-webkit-color-swatch-wrapper]:p-0"
              onChange={(event) =>
                update({
                  accent: event.target.value,
                  accent_from_artwork: false,
                })
              }
              type="color"
              value={accentValue}
            />
            <input
              aria-label="Accent colour as hex"
              className="w-[4.6rem] bg-transparent font-mono text-xs text-mujin-text outline-none"
              onBlur={() => {
                if (hexDraft && /^#?[0-9a-f]{6}$/i.test(hexDraft.trim())) {
                  const value = hexDraft.trim().replace(/^#?/, "#");
                  update({ accent: value, accent_from_artwork: false });
                }
                setHexDraft(null);
              }}
              onChange={(event) => setHexDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              value={hexDraft ?? accentValue}
            />
          </label>
        </div>
        <div className="mt-4 divide-y divide-white/[0.06]">
          <ToggleRow
            checked={appearance.accent_from_artwork}
            description={
              appearance.preset === "material-you"
                ? "Material You builds its whole scheme (surfaces, containers and accent) from the playing album's cover, using Material's HCT tonal palettes."
                : "The accent follows the playing album's cover; surfaces stay as the theme draws them."
            }
            label="Colour from album art"
            onChange={(accent_from_artwork) => update({ accent_from_artwork })}
          />
        </div>
        {appearance.accent_from_artwork && (
          <div className="mt-2 flex items-center gap-3 text-xs text-mujin-text-tertiary">
            {imageSrc ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                alt=""
                className="h-9 w-9 rounded-md object-cover"
                src={imageSrc}
              />
            ) : (
              <Sparkles className="h-4 w-4" />
            )}
            {environment.artworkColor != null ? (
              <span className="inline-flex items-center gap-2">
                <span
                  className="h-4 w-4 rounded-full"
                  style={{ background: hexFromArgb(environment.artworkColor) }}
                />
                Source colour {hexFromArgb(environment.artworkColor)} from the
                playing cover
              </span>
            ) : (
              <span>
                Play an album to take its colour; until then the accent above is
                used.
              </span>
            )}
          </div>
        )}
        <div
          className="mt-5 grid gap-2 rounded-xl border border-white/[0.08] bg-mujin-surface-1 p-4 sm:grid-cols-2"
          data-contrast-report
        >
          <p className="text-xs font-medium text-mujin-text-secondary sm:col-span-2">
            Contrast (WCAG 2.2){" "}
            {adjusted && (
              <span className="text-mujin-text-tertiary">
                · the accent's tone was adjusted to stay readable
              </span>
            )}
          </p>
          {checks.map((check) => (
            <div
              className="flex items-center justify-between gap-3 text-xs"
              key={check.name}
            >
              <span className="inline-flex min-w-0 items-center gap-2 text-mujin-text-secondary">
                <span
                  aria-hidden="true"
                  className="grid h-5 w-8 shrink-0 place-items-center rounded font-semibold"
                  style={{
                    background: check.background,
                    color: check.foreground,
                  }}
                >
                  Aa
                </span>
                <span className="truncate">{check.name}</span>
              </span>
              <span className="mj-tnum shrink-0 font-mono text-mujin-text">
                {check.ratio.toFixed(2)}:1{" "}
                <span
                  className={cn(
                    "ml-1 rounded px-1.5 py-0.5 text-[0.65rem] font-semibold",
                    contrastGrade(check.ratio) === "AAA"
                      ? "bg-quality-high/20 text-mujin-text"
                      : "bg-white/[0.08] text-mujin-text-secondary",
                  )}
                >
                  {contrastGrade(check.ratio)}
                </span>
              </span>
            </div>
          ))}
        </div>
      </Section>

      <Section
        description="Rounder or squarer corners, larger text and a tighter layout, applied everywhere at once."
        id="appearance-shape"
        title="Shape and text"
      >
        <div className="grid gap-6 sm:grid-cols-2">
          <label className="block">
            <span className="flex items-center justify-between text-sm text-mujin-text">
              Corner radius
              <span className="mj-tnum font-mono text-xs text-mujin-text-tertiary">
                {appearance.corner_radius}px
              </span>
            </span>
            <input
              aria-label="Corner radius"
              className="mt-3 w-full accent-[var(--color-mujin-copper)]"
              max={CORNER_RADIUS_RANGE.max}
              min={CORNER_RADIUS_RANGE.min}
              onChange={(event) =>
                update({ corner_radius: Number(event.target.value) })
              }
              step={1}
              type="range"
              value={appearance.corner_radius}
            />
            <span className="mt-1 flex justify-between text-[0.7rem] text-mujin-text-tertiary">
              <span>Square</span>
              <span>Default {DEFAULT_CORNER_RADIUS}px</span>
              <span>Round</span>
            </span>
          </label>
          <label className="block">
            <span className="flex items-center justify-between text-sm text-mujin-text">
              Text size
              <span className="mj-tnum font-mono text-xs text-mujin-text-tertiary">
                {Math.round(appearance.text_scale * 100)}%
              </span>
            </span>
            <input
              aria-label="Text size"
              className="mt-3 w-full accent-[var(--color-mujin-copper)]"
              max={TEXT_SCALE_RANGE.max}
              min={TEXT_SCALE_RANGE.min}
              onChange={(event) =>
                update({ text_scale: Number(event.target.value) })
              }
              step={0.05}
              type="range"
              value={appearance.text_scale}
            />
            <span className="mt-1 flex justify-between text-[0.7rem] text-mujin-text-tertiary">
              <span>Smaller</span>
              <span>100%</span>
              <span>Larger</span>
            </span>
          </label>
        </div>
        <div className="mt-6">
          <p className="mb-2 text-sm text-mujin-text">Density</p>
          <Segmented
            label="Density"
            onChange={(density) => update({ density })}
            options={[
              { value: "comfortable", label: "Comfortable" },
              { value: "compact", label: "Compact" },
            ]}
            value={appearance.density}
          />
        </div>
        <div className="mt-6">
          <p className="mb-2 text-sm text-mujin-text">Typeface</p>
          <p className="mb-3 text-xs text-mujin-text-tertiary">
            All bundled with Mujin Music under the SIL Open Font License; none
            is fetched from the internet.
          </p>
          <div className="grid gap-2 sm:grid-cols-3">
            {FONT_CHOICES.map((font) => (
              <button
                aria-pressed={appearance.font === font}
                className={cn(
                  "rounded-xl border p-3 text-left transition-colors",
                  appearance.font === font
                    ? "border-mujin-copper bg-mujin-copper/[0.08]"
                    : "border-white/[0.08] bg-mujin-surface-1 hover:border-white/25",
                )}
                data-font-choice={font}
                key={font}
                onClick={() => update({ font })}
                type="button"
              >
                <span
                  className="block text-[1.35rem] leading-tight text-mujin-text"
                  style={{ fontFamily: FONT_SAMPLE_STACKS[font] }}
                >
                  Night Swim
                </span>
                <span className="mt-1 block text-xs text-mujin-text-tertiary">
                  {font === "mujin" && appearance.preset === "material-you"
                    ? "Preset default (Roboto Flex)"
                    : FONT_LABELS[font]}
                </span>
              </button>
            ))}
          </div>
        </div>
      </Section>

      <Section
        description="How much room the sidebar takes, and how Now Playing frames the album."
        id="appearance-layout"
        title="Layout"
      >
        <div>
          <p className="mb-2 text-sm text-mujin-text">Sidebar</p>
          <Segmented
            label="Sidebar"
            onChange={(sidebar) => update({ sidebar })}
            options={[
              { value: "expanded", label: "Expanded" },
              { value: "compact", label: "Compact icons" },
            ]}
            value={appearance.sidebar}
          />
        </div>
        <div className="mt-6">
          <p className="mb-2 text-sm text-mujin-text">Now Playing</p>
          <div className="grid gap-3 sm:grid-cols-3">
            {NOW_PLAYING_LAYOUTS.map((layout) => (
              <button
                aria-pressed={appearance.now_playing_layout === layout}
                className={cn(
                  "rounded-xl border p-2 text-left transition-colors",
                  appearance.now_playing_layout === layout
                    ? "border-mujin-copper"
                    : "border-white/[0.08] hover:border-white/25",
                )}
                data-np-layout-choice={layout}
                key={layout}
                onClick={() => update({ now_playing_layout: layout })}
                type="button"
              >
                <NowPlayingLayoutPicture layout={layout} />
                <span className="mt-2 block px-1 text-sm text-mujin-text">
                  {NOW_PLAYING_LAYOUT_LABELS[layout]}
                </span>
              </button>
            ))}
          </div>
        </div>
        <div className="mt-6 divide-y divide-white/[0.06]">
          <ToggleRow
            checked={appearance.motion === "reduced"}
            description="Stops hover lifts, cross-fades and drifting artwork. Mujin Music also follows your system's reduced-motion setting."
            label="Reduce motion"
            onChange={(on) => update({ motion: on ? "reduced" : "system" })}
          />
          <ToggleRow
            checked={appearance.transparency === "reduced"}
            description="Frosted Glass becomes solid panels. Your system's reduced-transparency setting does the same."
            label="Reduce transparency"
            onChange={(on) =>
              update({ transparency: on ? "reduced" : "system" })
            }
          />
        </div>
        {appearance.preset === "frosted-glass" && (
          <div className="mt-4">
            <p className="mb-2 text-sm text-mujin-text">
              Window material (Windows app)
            </p>
            <Segmented
              label="Window material"
              onChange={(glass_material) => update({ glass_material })}
              options={[
                { value: "mica", label: "Mica" },
                { value: "acrylic", label: "Acrylic" },
              ]}
              value={appearance.glass_material}
            />
            <p className="mt-2 text-xs leading-5 text-mujin-text-tertiary">
              {theme.glass.native
                ? `This window shows the system's ${theme.glass.native === "vibrancy" ? "vibrancy" : theme.glass.native === "mica" ? "Mica" : "Acrylic"} material behind the glass.`
                : "In a browser, and where the system has no window material, Mujin Music blurs the playing album behind the glass instead."}
            </p>
          </div>
        )}
      </Section>

      <Section
        description="Choose which rows Home shows and in what order. Drag a row, or use the arrows."
        id="appearance-home"
        title="Home"
      >
        <HomeRowsEditor
          onChange={(home_rows) => update({ home_rows })}
          rows={appearance.home_rows}
        />
      </Section>
    </div>
  );
}
