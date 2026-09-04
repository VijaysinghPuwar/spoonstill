//! Failures at the process boundary, each naming what an operator can act on.
//!
//! D-052 treats hostile input as the normal case, which means "it failed" is
//! never an acceptable message. Every variant here carries the specific path,
//! the specific binary, or the specific field that went wrong — because the
//! operator reading it has 500 scenes and needs to know which one.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::profile::Mismatch;

/// Everything that can go wrong between us and FFmpeg.
#[derive(Debug)]
pub enum MediaError {
    /// A configured binary is not where we were told it would be.
    ///
    /// Fails fast with the path we actually tried, because "ffmpeg not found"
    /// when a bundled binary was expected is a packaging bug, and the path is
    /// the only thing that distinguishes it from a missing `PATH` entry.
    BinaryMissing {
        /// What we were looking for.
        tool: &'static str,
        /// The exact path or program name we tried to execute.
        tried: PathBuf,
        /// The underlying OS error.
        source: std::io::Error,
    },
    /// The child could not be spawned, or its pipes could not be taken.
    Spawn {
        /// A paste-ready form of the command.
        command: String,
        /// The underlying OS error.
        source: std::io::Error,
    },
    /// The child ran and exited non-zero.
    Exit {
        /// Which program it was — `ffmpeg`, `ffprobe`, `edge-tts` (D-150).
        ///
        /// Carried, because this message used to say *"ffmpeg exited 1"* over
        /// an `ffprobe` command line on the very next line: a sentence
        /// contradicted by its own evidence.
        program: String,
        /// A paste-ready form of the command.
        command: String,
        /// Exit status as reported by the OS, or `None` if killed by a signal.
        code: Option<i32>,
        /// Retained raw stderr — never a parser's summary of it.
        stderr: String,
    },
    /// The child was still running when its deadline expired.
    Timeout {
        /// A paste-ready form of the command.
        command: String,
        /// How long we waited.
        waited: Duration,
    },
    /// The child was cancelled by us (D-045), gracefully or otherwise.
    Cancelled {
        /// A paste-ready form of the command.
        command: String,
    },
    /// `ffprobe` returned something that is not the JSON we asked for.
    UnreadableProbe {
        /// The file being probed.
        path: PathBuf,
        /// What was wrong with the output.
        detail: String,
    },
    /// The file probed fine but is not the media we require.
    UnusableInput {
        /// The file in question.
        path: PathBuf,
        /// What is unusable about it.
        detail: String,
    },
    /// A rendered segment does not match [`crate::profile::SegmentProfile`].
    ///
    /// This is the D-041 gate. FFmpeg will concatenate a mismatched segment
    /// with exit code 0 and no warning, so this error is the only thing
    /// standing between a wrong SAR and a silently corrupt final render.
    ProfileMismatch {
        /// The segment that failed the assertion.
        path: PathBuf,
        /// Every field that differs, named individually.
        mismatches: Vec<Mismatch>,
    },
    /// Filesystem trouble around a segment, with the path attached.
    Io {
        /// What we were doing.
        doing: &'static str,
        /// The path it was being done to.
        path: PathBuf,
        /// The underlying OS error.
        source: std::io::Error,
    },
    /// Geometry we will not render, from the domain layer.
    Geometry(spoonstill_core::GeometryError),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::BinaryMissing {
                tool,
                tried,
                source,
            } => write!(
                f,
                "{tool} could not be executed at {}: {source}\n\
                 spoonstill does not fall back to a different binary — a render made with an \
                 unknown build is not reproducible.",
                tried.display()
            ),
            MediaError::Spawn { command, source } => {
                write!(
                    f,
                    "could not start the child process: {source}\n  {command}"
                )
            }
            MediaError::Exit {
                program,
                command,
                code,
                stderr,
            } => {
                let code = code.map_or_else(|| "killed by signal".to_string(), |c| c.to_string());
                write!(
                    f,
                    "{program} exited {code}\n  {command}\n{}",
                    indent(stderr)
                )
            }
            MediaError::Timeout { command, waited } => write!(
                f,
                "no response after {:.1}s\n  {command}",
                waited.as_secs_f64()
            ),
            MediaError::Cancelled { command } => write!(f, "cancelled\n  {command}"),
            MediaError::UnreadableProbe { path, detail } => {
                write!(
                    f,
                    "ffprobe output for {} is unusable: {detail}",
                    path.display()
                )
            }
            MediaError::UnusableInput { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
            MediaError::ProfileMismatch { path, mismatches } => {
                writeln!(
                    f,
                    "{} does not match the segment profile ({} field{}):",
                    path.display(),
                    mismatches.len(),
                    if mismatches.len() == 1 { "" } else { "s" }
                )?;
                for m in mismatches {
                    writeln!(f, "  {m}")?;
                }
                write!(
                    f,
                    "FFmpeg would concatenate this segment with exit code 0 and no warning \
                     (D-041). That is why this is checked here."
                )
            }
            MediaError::Io {
                doing,
                path,
                source,
            } => {
                write!(f, "{doing} {}: {source}", path.display())
            }
            MediaError::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MediaError::BinaryMissing { source, .. }
            | MediaError::Spawn { source, .. }
            | MediaError::Io { source, .. } => Some(source),
            MediaError::Geometry(e) => Some(e),
            _ => None,
        }
    }
}

impl From<spoonstill_core::GeometryError> for MediaError {
    fn from(e: spoonstill_core::GeometryError) -> Self {
        MediaError::Geometry(e)
    }
}

/// Indent captured stderr so it is visibly the child's output, not ours.
fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  | {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every message must name the thing the operator can act on. A render of
    /// 500 scenes that says only "ffmpeg failed" is unactionable.
    #[test]
    fn errors_name_the_specific_thing_that_failed() {
        let e = MediaError::BinaryMissing {
            tool: "ffmpeg",
            tried: PathBuf::from("/opt/spoonstill/bin/ffmpeg"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert!(e.to_string().contains("/opt/spoonstill/bin/ffmpeg"));

        let e = MediaError::UnusableInput {
            path: PathBuf::from("scene-147.jpg"),
            detail: "0 bytes".into(),
        };
        assert!(e.to_string().contains("scene-147.jpg"));
    }

    /// A profile mismatch must explain why the check exists at all — the whole
    /// hazard is that FFmpeg itself stays silent.
    #[test]
    fn a_profile_mismatch_explains_why_it_is_checked_here() {
        let e = MediaError::ProfileMismatch {
            path: PathBuf::from("seg-003.mp4"),
            mismatches: vec![Mismatch {
                field: "sample_aspect_ratio",
                expected: "1:1".into(),
                actual: "30007:30000".into(),
            }],
        };
        let text = e.to_string();
        assert!(text.contains("sample_aspect_ratio"));
        assert!(text.contains("30007:30000"));
        assert!(text.contains("D-041"));
    }
}
