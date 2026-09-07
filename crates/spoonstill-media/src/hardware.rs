//! Which hardware video encoders this machine can actually run (D-036, D-159).
//!
//! D-036 settled the encoder in M1 and left one clause of itself unbuilt:
//! *"probe availability at runtime, expose it as an explicit fast draft mode,
//! and always fall back to libx264."* The default was implemented, the fallback
//! was implemented by never leaving it, and **the probe was not** — so through
//! eight releases this project could not answer the first question an operator
//! with a graphics card asks, which is whether it is being used.
//!
//! This module answers it. It does **not** change what renders: the default is
//! still `libx264 -preset medium -crf 18`, for D-036's measured reason and for
//! D-144's, which timed the encoder at **14% of a 4K render** and put NVENC's
//! whole ceiling at a 1.17x speedup with **zero** memory saved. Detection is
//! worth having anyway, because "does it use my GPU" is currently answered by
//! guessing, and because the answer belongs in a diagnostics bundle from a
//! stranger's machine (D-016).
//!
//! # Listing an encoder is not detecting it
//!
//! The load-bearing measurement, Windows 2026-09-05, on the `Gyan.FFmpeg` build
//! that the README's own `winget` line installs (9.0.1, built with
//! `--enable-nvenc --enable-amf --enable-libvpl --enable-vaapi
//! --enable-vulkan`), on a machine carrying an RTX 3060 *and* an AMD Radeon:
//!
//! | encoder | `-encoders` says | actually encodes |
//! |---|---|---|
//! | `h264_nvenc` | present | **yes** |
//! | `h264_amf` | present | **yes** |
//! | `h264_mf` | present | **yes** |
//! | `h264_qsv` | present | no — `Error creating a MFX session: -9` |
//! | `h264_vaapi` | present | no — needs a hardware frames context |
//! | `h264_d3d12va` | present | no — needs a hardware frames context |
//! | `h264_vulkan` | present | no — needs a hardware frames context |
//!
//! Four of seven. `-encoders` reports **what the build was compiled with**, and
//! a general-purpose build is compiled with everything: `h264_qsv` is listed on
//! a machine with no Intel graphics at all, and `h264_vaapi` — a Linux API — is
//! listed on Windows. A detector that grepped that list would tell this
//! operator they have seven hardware encoders, which is worse than telling them
//! nothing, because it is a specific false answer they would then act on.
//!
//! So each candidate is **run**. That is the rule everywhere else here:
//! `Tools::ready` stats the binary because the spawn is the authority (D-103),
//! and a check that passes by finding nothing to check is not a check (D-125).
//!
//! # Why the probe feeds software frames
//!
//! The three that fail with *"Impossible to convert between the formats"* are
//! not broken drivers — they are encoders that will only accept frames already
//! on the GPU. Our filter graph produces **software** frames and always will:
//! D-030 through D-037 prescale, `zoompan`, `setparams` and `format` all run on
//! the CPU, and D-144 measured that this is where the time and all of the
//! memory go. An encoder that cannot take a software frame is therefore not a
//! drop-in for libx264 here, whatever the driver could do in some other
//! program — so reporting it unusable is not a limitation of the probe, it is
//! the correct answer to the question this project is asking.
//!
//! The probe is shaped like the real thing for exactly that reason: `yuv420p`
//! software frames, from a source with no file behind it, encoded and thrown
//! away.

use std::path::Path;
use std::time::Duration;

use crate::MediaError;
use crate::command::FfmpegCommand;
use crate::tools::Tools;

/// How long one encoder gets to prove itself.
///
/// Generous because this is cold hardware initialisation — a first NVENC or AMF
/// session loads a driver runtime — and because the cost of being wrong is
/// reporting "your graphics card does not work" to somebody whose graphics card
/// works. It is a ceiling on a wedged driver, not a performance budget.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// The frame size the probe encodes.
///
/// Small enough that the encode itself is free and the whole cost is session
/// setup, but not so small that an encoder refuses it: NVENC has a documented
/// 145x49 floor and several others round up to macroblocks, so this clears
/// every one of them comfortably.
const PROBE_SIZE: &str = "320x240";

/// One hardware H.264 encoder that might be available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// The name FFmpeg knows it by — what would follow `-c:v`.
    pub encoder: &'static str,
    /// The hardware it belongs to, in the operator's terms.
    ///
    /// "NVIDIA (NVENC)" rather than `h264_nvenc`, because the question being
    /// answered is about the card in the machine, not about a codec name.
    pub vendor: &'static str,
}

/// What this machine can do with a [`Candidate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// It encoded a frame here. This is the only status that means anything.
    Usable,
    /// FFmpeg has it compiled in, but it would not run on this machine.
    ///
    /// The reason is FFmpeg's own stderr, taken as its last meaningful line and
    /// never summarised by a parser of ours — D-016's rule, and the difference
    /// between *"Error creating a MFX session: -9"* (no Intel graphics) and a
    /// driver that needs updating.
    Unusable {
        /// Why it would not run, in FFmpeg's words.
        reason: String,
    },
    /// This FFmpeg build does not have it at all.
    ///
    /// Not a fault: a build without `--enable-nvenc` on a machine with no
    /// NVIDIA card is the correct pairing, and calling it "missing" would
    /// invent a problem.
    Absent,
}

impl Support {
    /// Whether this encoder could actually be used.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Usable)
    }
}

/// A [`Candidate`] and what this machine turned out to make of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// Which encoder was asked about.
    pub candidate: Candidate,
    /// What happened when it was asked.
    pub support: Support,
}

/// The hardware encoders worth asking about on macOS.
///
/// VideoToolbox is the whole list: it is the one hardware encoder Apple ships,
/// it covers both Apple silicon and the Intel Macs D-071 still puts in scope,
/// and there is no second vendor to ask about.
#[cfg(target_os = "macos")]
const CANDIDATES: &[Candidate] = &[Candidate {
    encoder: "h264_videotoolbox",
    vendor: "Apple (VideoToolbox)",
}];

/// The hardware encoders worth asking about on Windows.
///
/// Four, in the order an operator would rank them: the three GPU vendors, then
/// Media Foundation — which is Windows' own abstraction and is usually backed
/// by one of the first three, so it is listed last rather than presented as a
/// fourth piece of hardware.
///
/// The ones deliberately **not** here are `h264_vaapi`, `h264_d3d12va` and
/// `h264_vulkan`: all three were measured refusing software frames on this
/// exact build, so listing them could only ever add three lines of `Unusable`
/// noise under a heading about the operator's graphics card.
#[cfg(target_os = "windows")]
const CANDIDATES: &[Candidate] = &[
    Candidate {
        encoder: "h264_nvenc",
        vendor: "NVIDIA (NVENC)",
    },
    Candidate {
        encoder: "h264_amf",
        vendor: "AMD (AMF)",
    },
    Candidate {
        encoder: "h264_qsv",
        vendor: "Intel (Quick Sync)",
    },
    Candidate {
        encoder: "h264_mf",
        vendor: "Windows (Media Foundation)",
    },
];

/// Neither platform this project targets (D-071), but the module still builds.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const CANDIDATES: &[Candidate] = &[
    Candidate {
        encoder: "h264_nvenc",
        vendor: "NVIDIA (NVENC)",
    },
    Candidate {
        encoder: "h264_vaapi",
        vendor: "VA-API",
    },
];

/// Every hardware encoder this platform might have.
#[must_use]
pub fn candidates() -> &'static [Candidate] {
    CANDIDATES
}

/// Ask this machine which hardware encoders it can really run.
///
/// One `-encoders` call to find out what the build has, then one short encode
/// per surviving candidate. That is a handful of process spawns and a second or
/// two, which is why it belongs on `still doctor` and in a diagnostics bundle
/// and **not** on the render path — the same split D-151 draws for the version
/// probe, for the same reason: this is a person asking what is on this machine,
/// and that question gets asked once.
///
/// Never fails. An FFmpeg that will not run at all makes every candidate
/// [`Support::Absent`], because [`Tools::ready`] is the surface that reports a
/// missing FFmpeg (D-103) and two errors for one cause is D-103's own defect.
#[must_use]
pub fn detect(tools: &Tools) -> Vec<Detected> {
    let built_in = compiled_encoders(tools.ffmpeg());
    CANDIDATES
        .iter()
        .map(|candidate| {
            let support = if built_in.iter().any(|name| name == candidate.encoder) {
                probe(tools.ffmpeg(), candidate.encoder)
            } else {
                Support::Absent
            };
            Detected {
                candidate: *candidate,
                support,
            }
        })
        .collect()
}

/// The encoder names this FFmpeg build was compiled with.
///
/// Only ever used to skip a pointless spawn — a name that appears here still
/// has to encode a frame before [`detect`] believes it. Empty when FFmpeg will
/// not run, which makes every candidate [`Support::Absent`].
fn compiled_encoders(ffmpeg: &Path) -> Vec<String> {
    let mut command = FfmpegCommand::new(ffmpeg);
    command.args(["-hide_banner", "-encoders"]);
    let Ok(finished) = command
        .spawn()
        .and_then(|child| child.wait_until(PROBE_TIMEOUT))
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&finished.stdout)
        .lines()
        .filter_map(encoder_name)
        .collect()
}

/// The encoder name out of one line of `ffmpeg -encoders`, if it is one.
///
/// ` V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)`
///
/// The flags column is fixed width, so the name is the second field. Taken
/// positionally rather than by pattern, which keeps a description containing a
/// space from being mistaken for a name and skips the header block — whose
/// lines have no six-character flags column.
fn encoder_name(line: &str) -> Option<String> {
    let mut fields = line.split_whitespace();
    let flags = fields.next()?;
    if flags.len() != 6 || !flags.starts_with('V') {
        return None;
    }
    let name = fields.next()?;
    // The legend at the top of the listing reads ` V..... = Video`, whose first
    // field is also six characters beginning with `V` — so without this the
    // parser reports an encoder called `=`. Harmless downstream, since only
    // known names are ever looked up, and wrong, which the test caught first.
    if !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(name.to_owned())
}

/// Encode a few frames with `encoder` and see whether it works.
fn probe(ffmpeg: &Path, encoder: &str) -> Support {
    let mut command = FfmpegCommand::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=black:s={PROBE_SIZE}:r=25:d=0.2"),
        // Software frames, in the pixel format the real chain ends on (D-034).
        // This is the whole point of the probe: an encoder that needs the frame
        // to be on the GPU already cannot stand in for libx264 here.
        "-pix_fmt",
        "yuv420p",
        "-c:v",
        encoder,
        "-f",
        "null",
        "-",
    ]);

    match command
        .spawn()
        .and_then(|child| child.wait_until(PROBE_TIMEOUT))
        .and_then(crate::command::Finished::ok)
    {
        Ok(_) => Support::Usable,
        Err(error) => Support::Unusable {
            reason: reason_from(&error, encoder),
        },
    }
}

/// FFmpeg's own first word on why an encoder would not start.
///
/// **The first line that names the encoder**, not the last line of stderr. That
/// distinction was a defect in the first draft of this module, and running it is
/// what found it: asking for `h264_qsv` on a machine with no Intel graphics
/// produces eleven lines, of which the last is
///
/// ```text
/// [out#0/null @ ...] Nothing was written into output file, because at least
/// one of its streams received no packets.
/// ```
///
/// — the muxer complaining about a consequence three components downstream. An
/// operator would read that and go looking at their output settings. The cause
/// is the *first* line, `Error creating a MFX session: -9`, because FFmpeg
/// reports causes before cascades: the encoder-tagged lines after it are
/// *"Could not open encoder before EOF"* and *"Task finished with error code"*,
/// each true and each useless.
///
/// Falls back to the last non-empty line when nothing names the encoder, which
/// is better than nothing and is where a future FFmpeg's changed tagging would
/// land. A timeout has no stderr worth quoting and says so in its own words.
fn reason_from(error: &MediaError, encoder: &str) -> String {
    let MediaError::Exit { stderr, .. } = error else {
        return error.to_string();
    };
    let lines = || {
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    lines()
        .find(|line| line.contains(encoder))
        .or_else(|| lines().next_back())
        .unwrap_or("failed with no explanation")
        .to_owned()
}

/// Which encoder a render actually uses (D-162).
///
/// D-036 settled this in M1 in one sentence with three clauses — *"probe
/// availability at runtime, expose it as an explicit fast draft mode, and
/// always fall back to libx264"* — and D-159 recorded that only the third was
/// ever built. This is the second. [`VideoEncoder::Software`] is still the
/// default on every platform and nothing here changes what an unflagged render
/// produces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    /// D-036's default: `libx264` on the CPU, on macOS and Windows alike.
    ///
    /// Not a fallback in the apologetic sense — it is the encoder this content
    /// is rendered with unless somebody asks otherwise, because VideoToolbox
    /// and NVENC band visibly on slow pans across large smooth gradients, and
    /// that is exactly what a Ken Burns move over a photograph is.
    #[default]
    Software,
    /// An opt-in hardware encoder, named by the FFmpeg encoder id.
    ///
    /// Held as the id rather than a vendor enum because it is the id that
    /// reaches `-c:v`, seeds the cache key, and comes back out of [`detect`] —
    /// three places that must not disagree about spelling.
    Hardware(String),
}

impl VideoEncoder {
    /// The name that follows `-c:v`.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Software => SOFTWARE_ENCODER,
            Self::Hardware(id) => id,
        }
    }

    /// Whether this is D-036's default rather than an opt-in.
    #[must_use]
    pub fn is_software(&self) -> bool {
        matches!(self, Self::Software)
    }

    /// The quality arguments for this encoder, given the project's settings.
    ///
    /// **`-crf` is an x264 option and only x264 has it.** NVENC spells the same
    /// idea `-cq`, AMF `-qp_i`/`-qp_p`, Quick Sync `-global_quality`, and
    /// VideoToolbox not at all — it takes `-q:v` on a 1-100 scale that runs the
    /// *other way*. Handing `-crf 18` to any of them is either ignored or an
    /// error, so the mapping is written out per encoder.
    ///
    /// The preset is x264's alone for the same reason: NVENC's `p1`-`p7` are a
    /// different scale with different meanings, so `medium` is *translated*
    /// rather than passed through and hoped for.
    #[must_use]
    pub fn quality_args(&self, preset: &str, crf: u32) -> Vec<String> {
        let quantizer = crf.to_string();
        match self.name() {
            // x264: the project's own settings, untouched. This arm emits
            // exactly what every render made before D-162 emitted, argument for
            // argument — which is what keeps an existing film byte-identical.
            SOFTWARE_ENCODER => {
                vec!["-preset".into(), preset.into(), "-crf".into(), quantizer]
            }
            // NVENC: `-cq` is constant quality under VBR and is numerically
            // comparable to CRF, so the project's 18 stays 18 rather than being
            // rescaled into a number nobody could relate back to the setting.
            // `-b:v 0` is required or the rate control drifts toward a default
            // bitrate and ignores `-cq` entirely.
            "h264_nvenc" => vec![
                "-preset".into(),
                nvenc_preset(preset).into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                quantizer,
                "-b:v".into(),
                "0".into(),
            ],
            // AMF: constant QP, set on all three frame types. Leaving P and B
            // to the driver gives visibly softer motion than the I frames,
            // which on a slow pan is the whole picture.
            "h264_amf" => vec![
                "-quality".into(),
                "quality".into(),
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                quantizer.clone(),
                "-qp_p".into(),
                quantizer.clone(),
                "-qp_b".into(),
                quantizer,
            ],
            // Quick Sync: `-global_quality` is its CRF analogue.
            "h264_qsv" => vec![
                "-preset".into(),
                preset.into(),
                "-global_quality".into(),
                quantizer,
            ],
            // Media Foundation exposes no quantizer — only a quality scale.
            "h264_mf" => vec![
                "-rate_control".into(),
                "quality".into(),
                "-quality".into(),
                quantizer,
            ],
            // VideoToolbox: `-q:v` runs 1-100 and *upwards* is better, which is
            // the opposite direction to CRF. Inverted here so that lowering the
            // project's `crf` improves the picture on a Mac exactly as it does
            // on the CPU, instead of silently doing the reverse.
            "h264_videotoolbox" => vec!["-q:v".into(), videotoolbox_quality(crf).to_string()],
            // Unreachable while `CANDIDATES` and this match agree, which is
            // what `every_candidate_has_quality_flags` asserts. Falling back to
            // no quality flags at all is the safe arm: the encoder picks its own
            // default rather than being handed an option it may reject.
            _ => Vec::new(),
        }
    }
}

/// D-036's encoder, spelled once.
pub const SOFTWARE_ENCODER: &str = "libx264";

/// x264's preset ladder mapped onto NVENC's `p1`-`p7`.
///
/// Not a passthrough: NVENC accepts the legacy x264 names, but they mean
/// different things on its own scale, and `medium` there is markedly faster and
/// softer than x264's. `p5` is NVENC's own balanced point.
fn nvenc_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "p1",
        "faster" | "fast" => "p3",
        "slow" => "p6",
        "slower" | "veryslow" | "placebo" => "p7",
        // `medium`, and anything unrecognised. An unknown preset is a setting
        // this build does not know, and the balanced point is the honest answer
        // to it — refusing the render would be worse.
        _ => "p5",
    }
}

/// A CRF quantizer as a VideoToolbox `-q:v` value.
///
/// CRF is 0-51 and lower is better; `-q:v` is 1-100 and higher is better. The
/// linear inversion is deliberate and approximate — these are two encoders'
/// internal scales and no mapping between them is exact — but it gets the
/// *direction* right, which is the part that would otherwise be a silent
/// quality regression on the platform this must not damage.
fn videotoolbox_quality(crf: u32) -> u32 {
    let crf = crf.min(51);
    (100 - (crf * 100 / 51)).clamp(1, 100)
}

/// The best hardware encoder this machine can actually run, if any.
///
/// [`candidates`] is already in the order an operator would rank them — the
/// GPU vendors, then the platform's own abstraction — so "best" is "first
/// usable", and that ranking stays in one place rather than being restated
/// here.
///
/// `None` on a machine with no usable hardware encoder, which is the signal to
/// stay on [`VideoEncoder::Software`] rather than to fail: D-036's third clause
/// is that the fallback is always available.
#[must_use]
pub fn best_hardware(detected: &[Detected]) -> Option<VideoEncoder> {
    detected
        .iter()
        .find(|entry| entry.support.is_usable())
        .map(|entry| VideoEncoder::Hardware(entry.candidate.encoder.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_encoder_is_still_libx264_on_every_platform() {
        // D-162's first promise, and the one that keeps the Mac side — and any
        // machine with a graphics card — rendering exactly what it rendered
        // before. This is a `cfg`-free assertion on purpose: a `#[cfg]` here
        // would let one platform drift silently.
        assert_eq!(VideoEncoder::default(), VideoEncoder::Software);
        assert_eq!(VideoEncoder::default().name(), "libx264");
        assert!(VideoEncoder::default().is_software());
    }

    #[test]
    fn software_quality_flags_are_exactly_what_they_have_always_been() {
        // Pinned as a whole vector, not field by field, because what must not
        // change is the *argument list* FFmpeg receives — an extra flag, a
        // reordering, or a lost one all produce a different film from the same
        // project and would silently miss D-107's cache.
        assert_eq!(
            VideoEncoder::Software.quality_args("medium", 18),
            vec!["-preset", "medium", "-crf", "18"]
        );
        // And it still carries the project's own settings rather than the
        // defaults it happens to be tested with.
        assert_eq!(
            VideoEncoder::Software.quality_args("slow", 20),
            vec!["-preset", "slow", "-crf", "20"]
        );
    }

    #[test]
    fn every_candidate_has_quality_flags_of_its_own() {
        // `-crf` is x264's alone. A candidate added to `CANDIDATES` without an
        // arm in `quality_args` would fall through to the empty vector and
        // encode at whatever the driver's default happens to be — a quality
        // regression with no error and no log line. This is the test that makes
        // adding hardware a two-line change instead of a one-line one.
        for candidate in candidates() {
            let encoder = VideoEncoder::Hardware(candidate.encoder.to_string());
            let args = encoder.quality_args("medium", 18);
            assert!(
                !args.is_empty(),
                "{} has no quality flags — add an arm to `quality_args`",
                candidate.encoder
            );
            assert!(
                !args.iter().any(|arg| arg == "-crf"),
                "{} was handed `-crf`, which only libx264 has",
                candidate.encoder
            );
        }
    }

    #[test]
    fn videotoolbox_quality_runs_the_same_direction_as_crf() {
        // The one mapping that could invert. `-q:v` counts upwards and CRF
        // counts downwards, so a passthrough would mean that lowering `crf` in
        // `project.yaml` made a Mac render *worse* — a silent quality
        // regression on the platform D-162 must not damage.
        let better = VideoEncoder::Hardware("h264_videotoolbox".into()).quality_args("medium", 14);
        let worse = VideoEncoder::Hardware("h264_videotoolbox".into()).quality_args("medium", 30);

        let value = |args: &[String]| args[1].parse::<u32>().expect("a number");
        assert!(
            value(&better) > value(&worse),
            "a lower crf must give a higher -q:v: {better:?} against {worse:?}"
        );
        // And it stays inside the scale at both ends, including a crf nobody
        // should use.
        for crf in [0, 18, 51, 99] {
            let args = VideoEncoder::Hardware("h264_videotoolbox".into()).quality_args("x", crf);
            let q = value(&args);
            assert!((1..=100).contains(&q), "crf {crf} gave -q:v {q}");
        }
    }

    #[test]
    fn nvenc_gets_its_own_preset_scale_and_a_comparable_quantizer() {
        let args = VideoEncoder::Hardware("h264_nvenc".into()).quality_args("medium", 18);
        // Translated, not passed through: `medium` is a name NVENC accepts and
        // means something else by.
        assert!(
            args.iter().any(|arg| arg == "p5"),
            "medium should map onto NVENC's own ladder: {args:?}"
        );
        assert!(!args.iter().any(|arg| arg == "medium"), "{args:?}");
        // `-b:v 0` or the rate control ignores `-cq` and drifts to a bitrate.
        assert!(args.windows(2).any(|w| w[0] == "-cq" && w[1] == "18"));
        assert!(args.windows(2).any(|w| w[0] == "-b:v" && w[1] == "0"));
    }

    #[test]
    fn best_hardware_takes_the_first_usable_and_nothing_else() {
        let entry = |encoder: &'static str, support: Support| Detected {
            candidate: Candidate {
                encoder,
                vendor: "test",
            },
            support,
        };

        // Order is `CANDIDATES`' order, so an unusable first entry is skipped
        // rather than preferred.
        let found = vec![
            entry(
                "h264_qsv",
                Support::Unusable {
                    reason: "no Intel graphics".into(),
                },
            ),
            entry("h264_amf", Support::Usable),
            entry("h264_mf", Support::Usable),
        ];
        assert_eq!(
            best_hardware(&found),
            Some(VideoEncoder::Hardware("h264_amf".into()))
        );

        // Nothing usable is not an error — it is the signal to stay on D-036's
        // default, which is always available.
        let none = vec![entry("h264_nvenc", Support::Absent)];
        assert_eq!(best_hardware(&none), None);
        assert_eq!(best_hardware(&[]), None);
    }
    #[test]
    fn every_candidate_names_an_h264_encoder() {
        // The whole module is about standing in for libx264 in one specific
        // filter chain, so a candidate for another codec would be a category
        // error rather than a nice extra.
        for candidate in candidates() {
            assert!(
                candidate.encoder.starts_with("h264_"),
                "{} is not an H.264 encoder",
                candidate.encoder
            );
            assert!(!candidate.vendor.is_empty());
        }
    }

    #[test]
    fn candidates_are_distinct() {
        let mut names: Vec<&str> = candidates().iter().map(|c| c.encoder).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "a candidate is listed twice");
    }

    /// The parser has to survive the header block, which has no flags column,
    /// and has to take the name rather than the description.
    #[test]
    fn encoder_names_are_read_positionally() {
        let listing = concat!(
            "Encoders:\n",
            " V..... = Video\n",
            " ------\n",
            " V....D libx264              libx264 H.264 / AVC (codec h264)\n",
            " V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n",
            " A....D aac                  AAC (Advanced Audio Coding)\n",
        );
        let names: Vec<String> = listing.lines().filter_map(encoder_name).collect();
        assert_eq!(names, vec!["libx264", "h264_nvenc"]);
    }

    /// An encoder that is not in the build is never spawned, and is never
    /// reported as broken hardware.
    #[test]
    fn an_absent_encoder_is_not_a_fault() {
        assert!(!Support::Absent.is_usable());
        assert!(
            !Support::Unusable {
                reason: "no device".into()
            }
            .is_usable()
        );
        assert!(Support::Usable.is_usable());
    }

    /// The reason is the cause, not the cascade.
    ///
    /// Verbatim stderr from `h264_qsv` on a machine with no Intel graphics
    /// (Windows, 2026-09-05). The last line is the muxer noticing that nothing
    /// arrived — true, and three components away from the problem. This test
    /// fails against the "last non-empty line" rule this module was first
    /// written with, which is how that rule was found to be wrong.
    #[test]
    fn the_reason_is_the_cause_and_not_the_cascade() {
        let error = MediaError::Exit {
            program: "ffmpeg".into(),
            command: "ffmpeg ...".into(),
            code: Some(1),
            stderr: concat!(
                "[h264_qsv @ 0x1] Error creating a MFX session: -9.
",
                "[h264_qsv @ 0x1] The current mfx implementation is not supported
",
                "[vost#0:0/h264_qsv @ 0x2] Could not open encoder before EOF
",
                "[out#0/null @ 0x3] Nothing was written into output file, because at ",
                "least one of its streams received no packets.

",
            )
            .into(),
        };
        assert_eq!(
            reason_from(&error, "h264_qsv"),
            "[h264_qsv @ 0x1] Error creating a MFX session: -9."
        );
    }

    /// Nothing naming the encoder still produces a sentence rather than an
    /// empty string — where a future FFmpeg's changed tagging would land.
    #[test]
    fn an_untagged_failure_falls_back_to_the_last_line() {
        let error = MediaError::Exit {
            program: "ffmpeg".into(),
            command: "ffmpeg ...".into(),
            code: Some(1),
            stderr: "something went wrong
and this was the last of it

"
            .into(),
        };
        assert_eq!(
            reason_from(&error, "h264_nvenc"),
            "and this was the last of it"
        );
    }

    /// Detection runs against whatever FFmpeg this machine has and must answer
    /// for every candidate without panicking — including on a machine with no
    /// FFmpeg at all, where every answer is `Absent`.
    #[test]
    fn detect_answers_for_every_candidate() {
        let detected = detect(&Tools::from_env());
        assert_eq!(detected.len(), candidates().len());
        for one in &detected {
            if let Support::Unusable { reason } = &one.support {
                assert!(
                    !reason.is_empty(),
                    "{} gave no reason",
                    one.candidate.encoder
                );
            }
        }
    }
}
