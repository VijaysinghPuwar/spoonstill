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

#[cfg(test)]
mod tests {
    use super::*;

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
