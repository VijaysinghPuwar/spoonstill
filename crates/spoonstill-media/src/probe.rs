//! `ffprobe`, typed and time-bounded.
//!
//! Modelled on `plan/lossless-cut/src/common/ffprobe.ts`, the most thorough
//! schema in the reference checkouts — read, not copied: lossless-cut is
//! GPL-2.0-only (D-062).
//!
//! Two rules shape this module:
//!
//! - **Every probe has a timeout, from the first call.** `ffprobe` on hostile
//!   media can sit forever; plan.md M1 lists it as a named risk. A timeout
//!   added "after the first hang" is a timeout added after a 500-scene batch
//!   has already stalled overnight.
//! - **Duration comes from the stream, cross-checked against the container**
//!   (D-021). `vbr_lying_header.mp3` is in the fixtures because container
//!   headers lie, and we measure the normalized artifact rather than trusting
//!   what a file claims about itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::command::FfmpegCommand;
use crate::error::MediaError;
use crate::tools::Tools;

/// Default ceiling for a single probe.
///
/// Generous against a slow network volume, short against a hang. A probe that
/// takes longer than this on a still or a narration clip is not slow, it is
/// stuck.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything we read back from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResult {
    /// The file this describes.
    pub path: PathBuf,
    /// Container format names, as the comma-joined list ffprobe reports.
    pub format_name: String,
    /// Container-level duration in seconds, when the container declares one.
    pub format_duration: Option<f64>,
    /// Every stream, in file order. Order is part of the segment profile
    /// (D-040), so it is preserved rather than sorted.
    pub streams: Vec<Stream>,
}

/// One stream, normalized into the fields the segment profile pins.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// Index within the file.
    pub index: u32,
    /// `video`, `audio`, or anything else the file happens to carry.
    pub kind: StreamKind,
    /// Codec short name, e.g. `h264`.
    pub codec_name: String,
    /// Codec profile, e.g. `High` or `LC`.
    pub profile: Option<String>,
    /// H.264 level as an integer, e.g. `40` for level 4.0.
    pub level: Option<i64>,
    /// Pixel format, video only.
    pub pix_fmt: Option<String>,
    /// Colour range: `tv` or `pc`.
    pub color_range: Option<String>,
    /// Matrix coefficients.
    pub color_space: Option<String>,
    /// Colour primaries.
    pub color_primaries: Option<String>,
    /// Transfer characteristics.
    pub color_transfer: Option<String>,
    /// Coded width, video only.
    pub width: Option<u32>,
    /// Coded height, video only.
    pub height: Option<u32>,
    /// Sample aspect ratio, normalized so "unspecified" reads as `1:1`.
    pub sample_aspect_ratio: Option<String>,
    /// Nominal frame rate as the `num/den` string ffprobe reports.
    pub r_frame_rate: Option<String>,
    /// Stream time base as `num/den`.
    pub time_base: Option<String>,
    /// Sample format, audio only.
    pub sample_fmt: Option<String>,
    /// Sample rate in Hz, audio only.
    pub sample_rate: Option<u32>,
    /// Channel count, audio only.
    pub channels: Option<u32>,
    /// Channel layout, audio only.
    pub channel_layout: Option<String>,
    /// Stream duration in seconds, when declared.
    pub duration: Option<f64>,
    /// Frames actually decoded — present only when counted.
    pub nb_read_frames: Option<u64>,
    /// Frames the container *declares*, without decoding.
    ///
    /// Metadata rather than evidence, so it never stands in for a counted
    /// frame check on a segment (D-041). It earns its place on a joined film,
    /// where decoding a 500-scene MP4 to count frames would cost minutes and
    /// the MP4 sample table is a faithful record of what the stream copy put
    /// there.
    pub nb_frames: Option<u64>,
}

/// What kind of stream this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// A video stream.
    Video,
    /// An audio stream.
    Audio,
    /// Anything else: subtitles, data, attached pictures.
    Other,
}

impl StreamKind {
    fn parse(text: &str) -> Self {
        match text {
            "video" => StreamKind::Video,
            "audio" => StreamKind::Audio,
            _ => StreamKind::Other,
        }
    }

    /// The name ffprobe uses, for error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
            StreamKind::Other => "other",
        }
    }
}

impl ProbeResult {
    /// The first video stream, if any.
    #[must_use]
    pub fn video(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.kind == StreamKind::Video)
    }

    /// The first audio stream, if any.
    #[must_use]
    pub fn audio(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.kind == StreamKind::Audio)
    }

    /// The authoritative duration of an audio artifact (D-021).
    ///
    /// The audio stream's own duration is preferred over the container's. When
    /// they disagree the container is the one lying — that is what
    /// `vbr_lying_header.mp3` demonstrates — and the stream is what actually
    /// gets decoded.
    #[must_use]
    pub fn audio_duration(&self) -> Option<f64> {
        self.audio()
            .and_then(|s| s.duration)
            .or(self.format_duration)
            .filter(|d| d.is_finite() && *d > 0.0)
    }

    /// Source geometry for a still, refusing a file that is not usable as one.
    ///
    /// # Errors
    ///
    /// [`MediaError::UnusableInput`] when the file carries no video stream, or
    /// when its dimensions are degenerate — `truncated.jpg` probes as `0x0`
    /// and must be named rather than divided by.
    pub fn source_geometry(&self) -> Result<spoonstill_core::SourceGeometry, MediaError> {
        let video = self.video().ok_or_else(|| MediaError::UnusableInput {
            path: self.path.clone(),
            detail: "carries no video stream, so it is not an image".into(),
        })?;
        let (num, den) = video
            .sample_aspect_ratio
            .as_deref()
            .and_then(parse_ratio)
            .unwrap_or((1, 1));
        spoonstill_core::SourceGeometry::new(
            video.width.unwrap_or(0),
            video.height.unwrap_or(0),
            num,
            den,
        )
        .map_err(|e| MediaError::UnusableInput {
            path: self.path.clone(),
            detail: e.to_string(),
        })
    }
}

/// Probe a file, with a deadline.
///
/// # Errors
///
/// [`MediaError::Timeout`] if `timeout` expires, [`MediaError::Exit`] if
/// `ffprobe` refuses the file, or [`MediaError::UnreadableProbe`] if the JSON
/// is not what we asked for.
pub fn probe(tools: &Tools, path: &Path, timeout: Duration) -> Result<ProbeResult, MediaError> {
    probe_inner(tools, path, timeout, false)
}

/// Probe a file and decode it fully to count frames.
///
/// Materially slower than [`probe`], because it decodes. Used where the frame
/// count is the assertion — which, per D-030, is every segment we render.
///
/// # Errors
///
/// As [`probe`].
pub fn probe_counting_frames(
    tools: &Tools,
    path: &Path,
    timeout: Duration,
) -> Result<ProbeResult, MediaError> {
    probe_inner(tools, path, timeout, true)
}

fn probe_inner(
    tools: &Tools,
    path: &Path,
    timeout: Duration,
    count_frames: bool,
) -> Result<ProbeResult, MediaError> {
    let mut command = FfmpegCommand::new(tools.ffprobe());
    command.args([
        "-hide_banner",
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]);
    if count_frames {
        command.args(["-count_frames"]);
    }
    // `--` would be ideal, but ffprobe has no such separator. `-i` is what
    // keeps a filename beginning with `-` from being read as an option.
    command.arg("-i").arg(path);

    let finished = command.spawn()?.wait_until(timeout)?.ok()?;
    parse_probe_json(path, &finished.stdout)
}

/// Parse ffprobe's JSON into the normalized form.
///
/// Split out from the process handling so it can be tested against captured
/// output without spawning anything.
///
/// # Errors
///
/// [`MediaError::UnreadableProbe`] when the payload is not the JSON we asked
/// for.
pub fn parse_probe_json(path: &Path, stdout: &[u8]) -> Result<ProbeResult, MediaError> {
    let raw: RawProbe = serde_json::from_slice(stdout).map_err(|e| {
        let preview: String = String::from_utf8_lossy(stdout).chars().take(200).collect();
        MediaError::UnreadableProbe {
            path: path.to_path_buf(),
            detail: format!("{e}; output began: {preview:?}"),
        }
    })?;

    let format = raw.format.unwrap_or_default();
    let streams = raw.streams.into_iter().map(Stream::from_raw).collect();

    Ok(ProbeResult {
        path: path.to_path_buf(),
        format_name: format.format_name.unwrap_or_default(),
        format_duration: format.duration.as_deref().and_then(parse_f64),
        streams,
    })
}

impl Stream {
    fn from_raw(raw: RawStream) -> Self {
        Self {
            index: raw.index.unwrap_or(0),
            kind: StreamKind::parse(raw.codec_type.as_deref().unwrap_or_default()),
            codec_name: raw.codec_name.unwrap_or_default(),
            profile: raw.profile,
            level: raw.level,
            pix_fmt: raw.pix_fmt,
            color_range: raw.color_range,
            color_space: raw.color_space,
            color_primaries: raw.color_primaries,
            color_transfer: raw.color_transfer,
            width: raw.width,
            height: raw.height,
            // ffprobe reports `0:1`, `N/A`, or nothing at all for a stream with
            // no aspect metadata. All three mean square pixels, and normalizing
            // here keeps three spellings of "1:1" out of the profile assertion.
            sample_aspect_ratio: Some(match raw.sample_aspect_ratio.as_deref() {
                None | Some("0:1") | Some("0:0") | Some("N/A") | Some("") => "1:1".to_string(),
                Some(other) => other.to_string(),
            }),
            r_frame_rate: raw.r_frame_rate,
            time_base: raw.time_base,
            sample_fmt: raw.sample_fmt,
            sample_rate: raw.sample_rate.as_deref().and_then(|s| s.parse().ok()),
            channels: raw.channels,
            channel_layout: raw.channel_layout,
            duration: raw.duration.as_deref().and_then(parse_f64),
            nb_read_frames: raw.nb_read_frames.as_deref().and_then(|s| s.parse().ok()),
            nb_frames: raw.nb_frames.as_deref().and_then(|s| s.parse().ok()),
        }
    }
}

/// Parse a `num:den` or `num/den` ratio.
fn parse_ratio(text: &str) -> Option<(u32, u32)> {
    let (a, b) = text.split_once([':', '/'])?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Parse a duration, rejecting the non-numbers ffprobe emits.
///
/// `"N/A"` is a real value in ffprobe output and parses to `None`, not to zero.
/// A zero here would become a one-frame segment that renders without complaint.
fn parse_f64(text: &str) -> Option<f64> {
    let value: f64 = text.trim().parse().ok()?;
    value.is_finite().then_some(value)
}

// --- the wire format, kept separate from the normalized form ----------------
//
// ffprobe is inconsistent about which numbers are JSON numbers and which are
// strings: `channels` is a number, `sample_rate` is a string, `duration` is a
// string. Deserializing into exactly what it sends and normalizing afterwards
// is less fragile than teaching serde each exception.

#[derive(Debug, Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Default, Deserialize)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    index: Option<u32>,
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    level: Option<i64>,
    pix_fmt: Option<String>,
    color_range: Option<String>,
    color_space: Option<String>,
    color_primaries: Option<String>,
    color_transfer: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    r_frame_rate: Option<String>,
    time_base: Option<String>,
    sample_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    duration: Option<String>,
    nb_read_frames: Option<String>,
    nb_frames: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real captured output, from the prototype render that validated the M1
    /// recipe on this machine.
    const SEGMENT_JSON: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"h264","profile":"High","codec_type":"video",
         "width":1920,"height":1080,"sample_aspect_ratio":"1:1","pix_fmt":"yuv420p",
         "level":40,"color_range":"tv","color_space":"bt709","color_primaries":"bt709",
         "color_transfer":"bt709","r_frame_rate":"30/1","time_base":"1/90000",
         "duration":"3.733333","nb_read_frames":"112"},
        {"index":1,"codec_name":"aac","profile":"LC","codec_type":"audio",
         "sample_fmt":"fltp","sample_rate":"48000","channels":2,
         "channel_layout":"stereo","time_base":"1/48000","duration":"3.733000"}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"3.733333"}
    }"#;

    fn parse(json: &str) -> ProbeResult {
        parse_probe_json(Path::new("seg.mp4"), json.as_bytes()).unwrap()
    }

    #[test]
    fn a_real_segment_probe_normalizes_completely() {
        let p = parse(SEGMENT_JSON);
        assert_eq!(p.format_name, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(p.streams.len(), 2);

        let v = p.video().unwrap();
        assert_eq!(v.codec_name, "h264");
        assert_eq!(v.profile.as_deref(), Some("High"));
        assert_eq!(v.level, Some(40));
        assert_eq!((v.width, v.height), (Some(1920), Some(1080)));
        assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));
        assert_eq!(v.sample_aspect_ratio.as_deref(), Some("1:1"));
        assert_eq!(v.time_base.as_deref(), Some("1/90000"));
        assert_eq!(v.nb_read_frames, Some(112));

        let a = p.audio().unwrap();
        assert_eq!(a.sample_rate, Some(48_000));
        assert_eq!(a.channels, Some(2));
        assert_eq!(a.channel_layout.as_deref(), Some("stereo"));
    }

    /// Stream order is part of the profile (D-040), so it must not be sorted
    /// or deduplicated on the way through.
    #[test]
    fn stream_order_is_preserved() {
        let p = parse(SEGMENT_JSON);
        assert_eq!(p.streams[0].kind, StreamKind::Video);
        assert_eq!(p.streams[1].kind, StreamKind::Audio);
    }

    /// All three of ffprobe's spellings of "no aspect metadata" mean square
    /// pixels, and must not reach the profile assertion as three values.
    #[test]
    fn unspecified_sar_normalizes_to_one_to_one() {
        for spelling in [r#""0:1""#, r#""0:0""#, r#""N/A""#, r#""""#] {
            let json = format!(
                r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"mjpeg",
                   "width":4000,"height":3000,"sample_aspect_ratio":{spelling}}}]}}"#
            );
            assert_eq!(
                parse(&json).video().unwrap().sample_aspect_ratio.as_deref(),
                Some("1:1"),
                "spelling {spelling}"
            );
        }
        // Absent entirely.
        let json = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"mjpeg",
                      "width":4000,"height":3000}]}"#;
        assert_eq!(
            parse(json).video().unwrap().sample_aspect_ratio.as_deref(),
            Some("1:1")
        );
    }

    /// A genuinely anamorphic source keeps its ratio — normalizing must not
    /// flatten a real value into 1:1.
    #[test]
    fn a_real_sar_survives_normalization() {
        let json = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"mjpeg",
                      "width":720,"height":576,"sample_aspect_ratio":"16:11"}]}"#;
        let p = parse(json);
        assert_eq!(
            p.video().unwrap().sample_aspect_ratio.as_deref(),
            Some("16:11")
        );
        let g = p.source_geometry().unwrap();
        assert_eq!(g.sar(), (16, 11));
        assert!(!g.has_square_pixels());
    }

    /// D-021: the stream's duration wins over the container's.
    #[test]
    fn the_audio_stream_duration_outranks_the_container() {
        let json = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"mp3",
                      "duration":"3.717021"}],
                      "format":{"format_name":"mp3","duration":"5.300000"}}"#;
        let p = parse(json);
        assert_eq!(p.audio_duration(), Some(3.717_021));
    }

    /// `"N/A"` is a real value in ffprobe output. Parsing it as zero would
    /// produce a one-frame segment that renders without complaint.
    #[test]
    fn na_durations_are_absent_not_zero() {
        let json = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"mp3",
                      "duration":"N/A"}],"format":{"format_name":"mp3","duration":"N/A"}}"#;
        let p = parse(json);
        assert_eq!(p.audio_duration(), None);
        assert_eq!(parse_f64("N/A"), None);
        assert_eq!(parse_f64("0.5"), Some(0.5));
    }

    /// `truncated.jpg` probes as 0x0. It must be refused by name.
    #[test]
    fn a_degenerate_still_is_refused_with_its_path() {
        let json = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"mjpeg",
                      "width":0,"height":0}]}"#;
        let err = parse(json).source_geometry().unwrap_err();
        assert!(err.to_string().contains("seg.mp4"), "{err}");
    }

    /// A file with no video stream is not an image, and must say so.
    #[test]
    fn audio_only_input_is_not_a_still() {
        let json = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"aac"}]}"#;
        let err = parse(json).source_geometry().unwrap_err();
        assert!(err.to_string().contains("no video stream"), "{err}");
    }

    /// Garbage on stdout must name the file and show what arrived, not panic.
    #[test]
    fn unparseable_output_is_reported_with_a_preview() {
        let err =
            parse_probe_json(Path::new("x.mp4"), b"ffprobe: command not found\n").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("x.mp4"), "{text}");
        assert!(text.contains("command not found"), "{text}");
    }

    /// An empty file yields an empty probe, not a crash.
    #[test]
    fn an_empty_stream_list_is_handled() {
        let p = parse(r#"{"streams":[],"format":{"format_name":"mp3"}}"#);
        assert!(p.video().is_none() && p.audio().is_none());
        assert_eq!(p.audio_duration(), None);
    }

    #[test]
    fn ratios_parse_in_both_spellings() {
        assert_eq!(parse_ratio("16:11"), Some((16, 11)));
        assert_eq!(parse_ratio("30/1"), Some((30, 1)));
        assert_eq!(parse_ratio("nonsense"), None);
    }
}
