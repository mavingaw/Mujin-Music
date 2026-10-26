//! What an opened ALSA PCM really is, from libasound's own `snd_pcm_dump`.
//!
//! An ALSA name can stand for a chain of plugins: `plug` may insert a
//! format or rate converter, `dmix` mixes other programs in, `softvol`
//! scales the samples. libasound prints the chain it built when the PCM was
//! opened, one plugin per header line, each followed by the setup it
//! negotiated, down to the bottom of the chain (the hardware, or a file
//! for the `file` plugin). Parsing that dump lets Parson say which plugins
//! the samples passed through and what the bottom of the chain really
//! runs at, instead of trusting the name it asked for.
//!
//! The parser is plain text processing, so it is tested on every platform.

use serde::Serialize;

use crate::procfs::{HwParams, parse_hw_params};

/// The plugin chain behind an opened PCM.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PcmChain {
    /// Plugins from the top (what Parson opened) to the bottom, e.g.
    /// `["Hardware PCM card 1 'USB Audio' device 0 subdevice 0"]` or
    /// `["File PCM", "Null PCM"]`.
    pub plugins: Vec<String>,
    /// No plugin in the chain can change a sample: every one is the
    /// hardware itself, a recorder, a discard or a plain copy.
    pub bit_exact: bool,
    /// The plugins that convert, mix or scale, when there are any.
    pub converting: Vec<String>,
    /// The setup the bottom of the chain negotiated (for a `hw` PCM, what
    /// the hardware runs).
    pub bottom: Option<HwParams>,
}

/// Plugins that pass every sample through unchanged. Anything else (rate,
/// linear, route, softvol, dmix, ioplug for PulseAudio or PipeWire and so
/// on) may change the samples, so it is not bit-exact.
const PASS_THROUGH: [&str; 5] = [
    "Hardware PCM",
    "File PCM",
    "Null PCM",
    "Hooks PCM",
    "Copy conversion PCM",
];

/// A plugin's short name: `File PCM (file=/tmp/x.raw)` is `File PCM`.
fn plugin_name(line: &str) -> String {
    let line = line.trim();
    if line.starts_with("Hardware PCM") {
        return line.to_string();
    }
    match line.split_once(" (") {
        Some((name, _)) => name.trim().to_string(),
        None => line.to_string(),
    }
}

fn passes_through(plugin: &str) -> bool {
    PASS_THROUGH.iter().any(|name| plugin.starts_with(name))
}

/// Parses the text `snd_pcm_dump` wrote for an opened, configured PCM.
pub fn parse_pcm_dump(text: &str) -> PcmChain {
    let mut plugins = Vec::new();
    let mut setups: Vec<String> = Vec::new();
    let mut in_setup = false;
    let mut expect_header = true;
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if in_setup && let Some(setup) = setups.last_mut() {
                setup.push_str(line);
                setup.push('\n');
            }
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "Its setup is:" {
            in_setup = true;
            setups.push(String::new());
            continue;
        }
        in_setup = false;
        // The first line names the top plugin; each `Slave:` (or
        // `Slave #0:` for `multi`) names the next one down. Other lines
        // (`Converter: …`, `Control: …`, the file plugin's `Final file
        // PCM`) are a plugin's own details.
        let header = if expect_header {
            Some(line)
        } else if let Some(rest) = line.strip_prefix("Slave") {
            rest.split_once(':').map(|(_, slave)| slave)
        } else {
            None
        };
        expect_header = false;
        let Some(header) = header else {
            continue;
        };
        // A wrapper that adds nothing prints `<Name> PCM: <slave>`; `plug`
        // that converts nothing prints `Plug PCM: <slave>` and is dropped.
        for part in header.split(": ") {
            let name = plugin_name(part);
            if !name.is_empty() && name != "Plug PCM" {
                plugins.push(name);
            }
        }
    }
    let converting: Vec<String> = plugins
        .iter()
        .filter(|plugin| !passes_through(plugin))
        .cloned()
        .collect();
    PcmChain {
        bit_exact: !plugins.is_empty() && converting.is_empty(),
        converting,
        bottom: setups.last().and_then(|setup| parse_hw_params(setup)),
        plugins,
    }
}

/// `File PCM → Null PCM`.
pub fn chain_label(chain: &PcmChain) -> String {
    chain.plugins.join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETUP_16: &str = "Its setup is:
  stream       : PLAYBACK
  access       : RW_INTERLEAVED
  format       : S16_LE
  subformat    : STD
  channels     : 2
  rate         : 44100
  exact rate   : 44100 (44100/1)
  msbits       : 16
  buffer_size  : 8820
  period_size  : 2205
";

    #[test]
    fn a_file_plugin_over_null_is_bit_exact() {
        let text = format!(
            "File PCM (file=/tmp/capture.raw)\nFinal file PCM (file=/tmp/capture.raw)\n{SETUP_16}Slave: Null PCM\n{SETUP_16}"
        );
        let chain = parse_pcm_dump(&text);
        assert_eq!(chain.plugins, ["File PCM", "Null PCM"]);
        assert!(chain.bit_exact);
        assert!(chain.converting.is_empty());
        let bottom = chain.bottom.unwrap();
        assert_eq!(
            (bottom.format.as_str(), bottom.rate, bottom.channels),
            ("S16_LE", 44_100, 2)
        );
        assert_eq!(bottom.buffer_size, Some(8_820));
        assert_eq!(chain_label(&parse_pcm_dump(&text)), "File PCM → Null PCM");
    }

    #[test]
    fn a_hardware_pcm_is_bit_exact_and_reports_the_hardware_setup() {
        let text = format!(
            "Hardware PCM card 1 'E30' device 0 subdevice 0\n{}",
            SETUP_16
                .replace("S16_LE", "S24_3LE")
                .replace("44100", "96000")
        );
        let chain = parse_pcm_dump(&text);
        assert_eq!(
            chain.plugins,
            ["Hardware PCM card 1 'E30' device 0 subdevice 0"]
        );
        assert!(chain.bit_exact);
        let bottom = chain.bottom.unwrap();
        assert_eq!((bottom.format.as_str(), bottom.rate), ("S24_3LE", 96_000));
    }

    #[test]
    fn plug_that_converts_nothing_collapses_into_its_slave() {
        let text = format!(
            "Plug PCM: File PCM (file=/tmp/c.raw)\nFinal file PCM (file=/tmp/c.raw)\n{SETUP_16}Slave: Null PCM\n{SETUP_16}"
        );
        let chain = parse_pcm_dump(&text);
        assert_eq!(chain.plugins, ["File PCM", "Null PCM"]);
        assert!(chain.bit_exact);
    }

    #[test]
    fn converters_mixers_and_sound_servers_are_not_bit_exact() {
        let setup_32 = SETUP_16.replace("S16_LE", "S32_LE");
        let text = format!(
            "Plug PCM: Linear conversion PCM (S32_LE)\n{SETUP_16}Slave: File PCM (file=/tmp/c.raw)\nFinal file PCM (file=/tmp/c.raw)\n{setup_32}Slave: Null PCM\n{setup_32}"
        );
        let chain = parse_pcm_dump(&text);
        assert_eq!(
            chain.plugins,
            ["Linear conversion PCM", "File PCM", "Null PCM"]
        );
        assert!(!chain.bit_exact);
        assert_eq!(chain.converting, ["Linear conversion PCM"]);
        // The bottom runs 32-bit although Parson opened 16-bit.
        assert_eq!(chain.bottom.unwrap().format, "S32_LE");

        let dmix = parse_pcm_dump(&format!(
            "Direct Stream Mixing PCM\n{SETUP_16}Slave: Hardware PCM card 0 'PCH' device 0 subdevice 0\n{SETUP_16}"
        ));
        assert_eq!(dmix.converting, ["Direct Stream Mixing PCM"]);
        assert!(!dmix.bit_exact);
        let pipewire = parse_pcm_dump(&format!("ALSA <-> PipeWire PCM I/O Plugin\n{SETUP_16}"));
        assert!(!pipewire.bit_exact);
        let softvol = parse_pcm_dump(&format!(
            "Soft volume PCM\nControl: PCM Playback Volume\n{SETUP_16}Slave: Hardware PCM card 1 'USB' device 0 subdevice 0\n{SETUP_16}"
        ));
        assert!(!softvol.bit_exact);
        assert_eq!(
            softvol.plugins,
            [
                "Soft volume PCM",
                "Hardware PCM card 1 'USB' device 0 subdevice 0"
            ]
        );
    }

    #[test]
    fn an_empty_dump_proves_nothing() {
        let chain = parse_pcm_dump("");
        assert!(!chain.bit_exact);
        assert!(chain.bottom.is_none());
    }
}
