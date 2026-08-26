//! The canonical segment profile, in one place, plus the assertion (D-040).
//!
//! # Why this exists in M1
//!
//! Nothing concatenates until M3. This is written now anyway, because
//! retrofitting a profile after segments exist means re-rendering every one of
//! them — and at n=500 that is the difference between a decision and an
//! incident.
//!
//! # Why the assertion exists at all
//!
//! Measured, in `ffmpeg-findings.md` §5: concatenating a SAR 30007:30000
//! segment between two SAR 1:1 segments with `-c copy` produced **exit code 0,
//! no error, no warning**, and an output declaring SAR 1:1 for the whole file.
//! Sixty frames rendered with the wrong geometry, silently.
//!
//! So "FFmpeg didn't complain" is not evidence of a valid join (D-041). This
//! module is the thing that complains.

use std::fmt;

use spoonstill_core::{OutputSpec, SAMPLE_RATE};

use crate::probe::{ProbeResult, StreamKind};

/// Container format list, as ffprobe spells it for MP4.
pub const CONTAINER: &str = "mov,mp4,m4a,3gp,3g2,mj2";
/// Video codec (D-036: libx264, always; hardware encode is an opt-in draft).
pub const VIDEO_CODEC: &str = "h264";
/// H.264 profile.
pub const VIDEO_PROFILE: &str = "High";
/// Pixel format. `yuvj420p` here would be a mismatch, not a synonym (D-037).
pub const PIX_FMT: &str = "yuv420p";
/// Colour range. Pinned because a JPEG source is full-range and would
/// otherwise carry `pc` into the segment (D-037).
pub const COLOR_RANGE: &str = "tv";
/// Matrix coefficients, primaries and transfer — all bt709.
pub const COLOR_SPACE: &str = "bt709";
/// Sample aspect ratio. The whole point of D-033.
pub const SAR: &str = "1:1";
/// Video track timescale, forced with `-video_track_timescale`.
///
/// 90 kHz is the classic MPEG timescale and divides evenly by 24, 25, 30, 50
/// and 60, so every frame rate an operator can pick lands on an exact tick.
/// Left to itself the MP4 muxer picks a per-frame-rate timescale — 1/15360 at
/// 30 fps — which would make the time base a function of the project settings
/// rather than a constant.
pub const VIDEO_TIMESCALE: u32 = 90_000;
/// Audio codec.
pub const AUDIO_CODEC: &str = "aac";
/// AAC profile.
pub const AUDIO_PROFILE: &str = "LC";
/// Audio sample format as the encoder emits it.
pub const SAMPLE_FMT: &str = "fltp";
/// Channel count.
pub const CHANNELS: u32 = 2;
/// Channel layout.
pub const CHANNEL_LAYOUT: &str = "stereo";
/// Audio bitrate, in the form passed to `-b:a`.
pub const AUDIO_BITRATE: &str = "192k";

/// The floor for the H.264 level tag.
///
/// Every V1 output — the three aspects at 1080 and below, at 60 fps and below
/// — fits inside level 4.0, so pinning the floor here means an entire project
/// carries one level even when its scenes differ in nothing that matters. A
/// level is an upper bound, so tagging a 360x640 segment 4.0 is legal and
/// plays everywhere. Geometry that genuinely needs more raises it (see
/// [`h264_level`]).
pub const LEVEL_FLOOR: i64 = 40;

/// The complete profile every segment in a project must match.
///
/// Partly fixed and partly derived: codec, colour and audio are the same in
/// every project spoonstill will ever render, while dimensions, frame rate and
/// level come from the project's own output spec. Both halves are pinned in
/// this one struct, which is what D-040 asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentProfile {
    /// Container format list.
    pub container: &'static str,
    /// Video codec short name.
    pub video_codec: &'static str,
    /// H.264 profile name.
    pub video_profile: &'static str,
    /// H.264 level as an integer (40 == level 4.0).
    pub video_level: i64,
    /// Pixel format.
    pub pix_fmt: &'static str,
    /// Colour range.
    pub color_range: &'static str,
    /// Matrix coefficients.
    pub color_space: &'static str,
    /// Colour primaries.
    pub color_primaries: &'static str,
    /// Transfer characteristics.
    pub color_transfer: &'static str,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Sample aspect ratio.
    pub sample_aspect_ratio: &'static str,
    /// Nominal frame rate, as `num/den`.
    pub r_frame_rate: String,
    /// Video time base, as `num/den`.
    pub video_time_base: String,
    /// Audio codec short name.
    pub audio_codec: &'static str,
    /// AAC profile.
    pub audio_profile: &'static str,
    /// Audio sample format.
    pub sample_fmt: &'static str,
    /// Audio sample rate.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
    /// Channel layout.
    pub channel_layout: &'static str,
    /// Audio time base, as `num/den`.
    pub audio_time_base: String,
    /// Required stream order.
    pub stream_order: [StreamKind; 2],
}

impl SegmentProfile {
    /// The profile implied by a project's output spec.
    #[must_use]
    pub fn for_output(output: OutputSpec) -> Self {
        Self {
            container: CONTAINER,
            video_codec: VIDEO_CODEC,
            video_profile: VIDEO_PROFILE,
            video_level: h264_level(output.width(), output.height(), output.fps()),
            pix_fmt: PIX_FMT,
            color_range: COLOR_RANGE,
            color_space: COLOR_SPACE,
            color_primaries: COLOR_SPACE,
            color_transfer: COLOR_SPACE,
            width: output.width(),
            height: output.height(),
            sample_aspect_ratio: SAR,
            r_frame_rate: format!("{}/1", output.fps()),
            video_time_base: format!("1/{VIDEO_TIMESCALE}"),
            audio_codec: AUDIO_CODEC,
            audio_profile: AUDIO_PROFILE,
            sample_fmt: SAMPLE_FMT,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            channel_layout: CHANNEL_LAYOUT,
            audio_time_base: format!("1/{SAMPLE_RATE}"),
            stream_order: [StreamKind::Video, StreamKind::Audio],
        }
    }
}

/// One field that differs, named.
///
/// "The segment is wrong" is not actionable at scene 147 of 500. The field, the
/// expectation and the reality are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// The profile field that differs.
    pub field: &'static str,
    /// What the profile requires.
    pub expected: String,
    /// What the file actually declares.
    pub actual: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: expected {:?}, found {:?}",
            self.field, self.expected, self.actual
        )
    }
}

/// Check a probed file against the profile, naming **every** field that differs.
///
/// Every field, not the first: an operator who fixes one mismatch and re-renders
/// 500 scenes only to hit the next one has been failed by the error message.
///
/// # Errors
///
/// Returns the list of mismatches. An empty list is impossible — success is
/// `Ok(())`.
pub fn assert_matches_profile(
    profile: &SegmentProfile,
    probe: &ProbeResult,
) -> Result<(), Vec<Mismatch>> {
    let mut out = Vec::new();

    compare("container", profile.container, &probe.format_name, &mut out);

    // Stream order is part of the profile: a segment whose audio precedes its
    // video concatenates without complaint and maps wrongly downstream.
    let kinds: Vec<StreamKind> = probe.streams.iter().map(|s| s.kind).collect();
    if kinds != profile.stream_order.to_vec() {
        out.push(Mismatch {
            field: "stream_order",
            expected: describe_kinds(&profile.stream_order),
            actual: describe_kinds(&kinds),
        });
    }

    match probe.video() {
        None => out.push(Mismatch {
            field: "video_stream",
            expected: "present".into(),
            actual: "absent".into(),
        }),
        Some(v) => {
            compare("video_codec", profile.video_codec, &v.codec_name, &mut out);
            compare_opt(
                "video_profile",
                profile.video_profile,
                v.profile.as_deref(),
                &mut out,
            );
            compare_num("video_level", profile.video_level, v.level, &mut out);
            compare_opt("pix_fmt", profile.pix_fmt, v.pix_fmt.as_deref(), &mut out);
            compare_opt(
                "color_range",
                profile.color_range,
                v.color_range.as_deref(),
                &mut out,
            );
            compare_opt(
                "color_space",
                profile.color_space,
                v.color_space.as_deref(),
                &mut out,
            );
            compare_opt(
                "color_primaries",
                profile.color_primaries,
                v.color_primaries.as_deref(),
                &mut out,
            );
            compare_opt(
                "color_transfer",
                profile.color_transfer,
                v.color_transfer.as_deref(),
                &mut out,
            );
            compare_num(
                "width",
                i64::from(profile.width),
                v.width.map(i64::from),
                &mut out,
            );
            compare_num(
                "height",
                i64::from(profile.height),
                v.height.map(i64::from),
                &mut out,
            );
            compare_opt(
                "sample_aspect_ratio",
                profile.sample_aspect_ratio,
                v.sample_aspect_ratio.as_deref(),
                &mut out,
            );
            compare_opt(
                "r_frame_rate",
                &profile.r_frame_rate,
                v.r_frame_rate.as_deref(),
                &mut out,
            );
            compare_opt(
                "video_time_base",
                &profile.video_time_base,
                v.time_base.as_deref(),
                &mut out,
            );
        }
    }

    match probe.audio() {
        None => out.push(Mismatch {
            field: "audio_stream",
            expected: "present".into(),
            actual: "absent".into(),
        }),
        Some(a) => {
            compare("audio_codec", profile.audio_codec, &a.codec_name, &mut out);
            compare_opt(
                "audio_profile",
                profile.audio_profile,
                a.profile.as_deref(),
                &mut out,
            );
            compare_opt(
                "sample_fmt",
                profile.sample_fmt,
                a.sample_fmt.as_deref(),
                &mut out,
            );
            compare_num(
                "sample_rate",
                i64::from(profile.sample_rate),
                a.sample_rate.map(i64::from),
                &mut out,
            );
            compare_num(
                "channels",
                i64::from(profile.channels),
                a.channels.map(i64::from),
                &mut out,
            );
            compare_opt(
                "channel_layout",
                profile.channel_layout,
                a.channel_layout.as_deref(),
                &mut out,
            );
            compare_opt(
                "audio_time_base",
                &profile.audio_time_base,
                a.time_base.as_deref(),
                &mut out,
            );
        }
    }

    if out.is_empty() { Ok(()) } else { Err(out) }
}

fn describe_kinds(kinds: &[StreamKind]) -> String {
    if kinds.is_empty() {
        return "no streams".to_string();
    }
    kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn compare(field: &'static str, expected: &str, actual: &str, out: &mut Vec<Mismatch>) {
    if expected != actual {
        out.push(Mismatch {
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
}

fn compare_opt(field: &'static str, expected: &str, actual: Option<&str>, out: &mut Vec<Mismatch>) {
    let actual = actual.unwrap_or("<absent>");
    compare(field, expected, actual, out);
}

fn compare_num(field: &'static str, expected: i64, actual: Option<i64>, out: &mut Vec<Mismatch>) {
    let matches = actual == Some(expected);
    if !matches {
        out.push(Mismatch {
            field,
            expected: expected.to_string(),
            actual: actual.map_or_else(|| "<absent>".to_string(), |v| v.to_string()),
        });
    }
}

/// The lowest standard H.264 level that can carry this geometry, floored at
/// [`LEVEL_FLOOR`].
///
/// From Annex A's `MaxFS` (frame size in macroblocks) and `MaxMBPS`
/// (macroblocks per second). Derived rather than guessed, because the level is
/// a pinned profile field: if the encoder chose it for us, the profile would be
/// asserting whatever the encoder felt like doing.
#[must_use]
pub fn h264_level(width: u32, height: u32, fps: u32) -> i64 {
    /// `(level_idc, MaxFS, MaxMBPS)`, in ascending order.
    const LEVELS: [(i64, u64, u64); 16] = [
        (10, 99, 1_485),
        (11, 396, 3_000),
        (12, 396, 6_000),
        (13, 396, 11_880),
        (20, 396, 11_880),
        (21, 792, 19_800),
        (22, 1_620, 20_250),
        (30, 1_620, 40_500),
        (31, 3_600, 108_000),
        (32, 5_120, 216_000),
        (40, 8_192, 245_760),
        (41, 8_192, 245_760),
        (42, 8_704, 522_240),
        (50, 22_080, 589_824),
        (51, 36_864, 983_040),
        (52, 36_864, 2_073_600),
    ];

    let macroblocks = u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16));
    let per_second = macroblocks * u64::from(fps);

    let required = LEVELS
        .iter()
        .find(|(_, max_fs, max_mbps)| macroblocks <= *max_fs && per_second <= *max_mbps)
        .map_or(52, |(level, _, _)| *level);

    required.max(LEVEL_FLOOR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::parse_probe_json;
    use spoonstill_core::Aspect;
    use std::path::Path;

    const GOOD: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"h264","profile":"High","codec_type":"video",
         "width":1920,"height":1080,"sample_aspect_ratio":"1:1","pix_fmt":"yuv420p",
         "level":40,"color_range":"tv","color_space":"bt709","color_primaries":"bt709",
         "color_transfer":"bt709","r_frame_rate":"30/1","time_base":"1/90000"},
        {"index":1,"codec_name":"aac","profile":"LC","codec_type":"audio",
         "sample_fmt":"fltp","sample_rate":"48000","channels":2,
         "channel_layout":"stereo","time_base":"1/48000"}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"3.733333"}
    }"#;

    fn probe_of(json: &str) -> ProbeResult {
        parse_probe_json(Path::new("seg.mp4"), json.as_bytes()).unwrap()
    }

    fn profile_1080p30() -> SegmentProfile {
        SegmentProfile::for_output(OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap())
    }

    #[test]
    fn a_conforming_segment_passes() {
        assert_eq!(
            assert_matches_profile(&profile_1080p30(), &probe_of(GOOD)),
            Ok(())
        );
    }

    /// The D-041 case itself: the SAR that concatenates silently.
    #[test]
    fn the_silent_sar_mismatch_is_caught() {
        let bad = GOOD.replace(
            r#""sample_aspect_ratio":"1:1""#,
            r#""sample_aspect_ratio":"30007:30000""#,
        );
        let errs = assert_matches_profile(&profile_1080p30(), &probe_of(&bad)).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "sample_aspect_ratio");
        assert_eq!(errs[0].actual, "30007:30000");
    }

    /// D-037's hazard, as a profile field: `yuvj420p` is a mismatch, not a
    /// synonym for `yuv420p`.
    #[test]
    fn a_full_range_pixel_format_is_a_mismatch() {
        let bad = GOOD.replace(r#""pix_fmt":"yuv420p""#, r#""pix_fmt":"yuvj420p""#);
        let errs = assert_matches_profile(&profile_1080p30(), &probe_of(&bad)).unwrap_err();
        assert!(errs.iter().any(|m| m.field == "pix_fmt"));
    }

    #[test]
    fn a_full_range_colour_flag_is_a_mismatch() {
        let bad = GOOD.replace(r#""color_range":"tv""#, r#""color_range":"pc""#);
        let errs = assert_matches_profile(&profile_1080p30(), &probe_of(&bad)).unwrap_err();
        assert!(errs.iter().any(|m| m.field == "color_range"));
    }

    /// Every differing field is named at once. Reporting only the first would
    /// cost an operator one 500-scene re-render per mismatch.
    #[test]
    fn every_differing_field_is_reported_together() {
        let bad = GOOD
            .replace(r#""width":1920"#, r#""width":1280"#)
            .replace(r#""height":1080"#, r#""height":720"#)
            .replace(r#""r_frame_rate":"30/1""#, r#""r_frame_rate":"25/1""#)
            .replace(r#""channels":2"#, r#""channels":1"#);
        let errs = assert_matches_profile(&profile_1080p30(), &probe_of(&bad)).unwrap_err();
        let fields: Vec<&str> = errs.iter().map(|m| m.field).collect();
        for expected in ["width", "height", "r_frame_rate", "channels"] {
            assert!(
                fields.contains(&expected),
                "{expected} missing from {fields:?}"
            );
        }
    }

    /// A segment whose audio precedes its video joins without complaint and
    /// maps wrongly. Order is a profile field.
    #[test]
    fn stream_order_is_asserted() {
        let mut p = probe_of(GOOD);
        p.streams.reverse();
        let errs = assert_matches_profile(&profile_1080p30(), &p).unwrap_err();
        assert!(errs.iter().any(|m| m.field == "stream_order"), "{errs:?}");
    }

    /// A video-only segment is not a valid scene: every scene has narration.
    #[test]
    fn a_missing_stream_is_named_rather_than_skipped() {
        let mut p = probe_of(GOOD);
        p.streams.retain(|s| s.kind == StreamKind::Video);
        let errs = assert_matches_profile(&profile_1080p30(), &p).unwrap_err();
        assert!(errs.iter().any(|m| m.field == "audio_stream"));
    }

    /// An absent field must read as absent, never silently satisfy the profile.
    #[test]
    fn an_absent_field_is_a_mismatch_not_a_pass() {
        let bad = GOOD.replace(r#""color_primaries":"bt709","#, "");
        let errs = assert_matches_profile(&profile_1080p30(), &probe_of(&bad)).unwrap_err();
        let m = errs.iter().find(|m| m.field == "color_primaries").unwrap();
        assert_eq!(m.actual, "<absent>");
    }

    /// The profile follows the project's output spec, in every aspect.
    #[test]
    fn the_profile_tracks_the_output_spec() {
        for aspect in Aspect::ALL {
            let out = OutputSpec::new(aspect, 1080, 30).unwrap();
            let p = SegmentProfile::for_output(out);
            assert_eq!((p.width, p.height), (out.width(), out.height()));
            assert_eq!(p.r_frame_rate, "30/1");
            // Fixed halves stay fixed.
            assert_eq!(p.sample_aspect_ratio, "1:1");
            assert_eq!(p.pix_fmt, "yuv420p");
            assert_eq!(p.video_time_base, "1/90000");
            assert_eq!(p.audio_time_base, "1/48000");
        }
    }

    /// The time base must not become a function of the frame rate — that is
    /// exactly what `-video_track_timescale` is there to prevent.
    #[test]
    fn the_time_base_is_constant_across_frame_rates() {
        for fps in [24_u32, 25, 30, 50, 60] {
            let p = SegmentProfile::for_output(
                OutputSpec::new(Aspect::Landscape16x9, 1080, fps).unwrap(),
            );
            assert_eq!(p.video_time_base, "1/90000", "at {fps}fps");
        }
    }

    /// Every ordinary V1 output shares one level, so a project's segments are
    /// uniform in the field without anyone having to think about it. The floor
    /// is what does the work here: a 360x640 segment needs only level 3.0, and
    /// tagging it 4.0 keeps it identical to its 1080p siblings.
    #[test]
    fn every_ordinary_v1_output_is_level_four() {
        for aspect in Aspect::ALL {
            for short_edge in [360_u32, 540, 720, 1080] {
                for fps in [24_u32, 25, 30] {
                    let out = OutputSpec::new(aspect, short_edge, fps).unwrap();
                    assert_eq!(
                        h264_level(out.width(), out.height(), fps),
                        40,
                        "{aspect} {short_edge} @ {fps}fps"
                    );
                }
            }
        }
    }

    /// A 16:9 or 9:16 frame at 1080 genuinely exceeds level 4.0 above 30 fps —
    /// 8,160 macroblocks at 50 fps is 408,000/s against a 245,760 ceiling — so
    /// the level rises rather than the stream being mistagged.
    ///
    /// 1:1 at the same size does not: 1080x1080 is 4,624 macroblocks, so even
    /// at 60 fps it fits. That difference is precisely why the level is derived
    /// from the geometry instead of pinned to a constant.
    ///
    /// A project stays internally uniform either way, which is all D-040
    /// requires: the level is a function of the output spec, and every scene in
    /// one project shares one spec.
    #[test]
    fn high_frame_rates_raise_the_level_only_where_the_geometry_demands_it() {
        for aspect in [Aspect::Landscape16x9, Aspect::Portrait9x16] {
            for fps in [50_u32, 60] {
                let out = OutputSpec::new(aspect, 1080, fps).unwrap();
                assert_eq!(
                    h264_level(out.width(), out.height(), fps),
                    42,
                    "{aspect} 1080 @ {fps}fps"
                );
            }
        }

        // 1:1 at 1080 crosses the same ceiling later, between 50 and 60 fps:
        // 4,624 macroblocks is 231,200/s at 50 and 277,440/s at 60.
        let square = OutputSpec::new(Aspect::Square1x1, 1080, 50).unwrap();
        assert_eq!(h264_level(square.width(), square.height(), 50), 40);
        assert_eq!(h264_level(square.width(), square.height(), 60), 42);

        // A smaller 16:9 frame fits at 60 fps with room to spare.
        let small = OutputSpec::new(Aspect::Landscape16x9, 540, 60).unwrap();
        assert_eq!(h264_level(small.width(), small.height(), 60), 40);
    }

    /// Geometry that genuinely exceeds level 4.0 raises the tag rather than
    /// emitting an illegal stream.
    #[test]
    fn geometry_beyond_level_four_raises_the_level() {
        // 4K30 needs more frame size than 4.0 allows.
        assert!(h264_level(3840, 2160, 30) > 40);
        // 1080p120 needs more macroblock throughput than 4.0 allows.
        assert!(h264_level(1920, 1080, 120) > 40);
        // And the table is ordered, so the answer is the *lowest* that fits.
        assert_eq!(h264_level(1920, 1080, 60), 42);
    }
}
