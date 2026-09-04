//! Single-scene rendering, as the CLI and the future Tauri shell both see it.
//!
//! D-010: the CLI is an adapter over this crate and owns no business logic, so
//! everything `still render-scene` needs to decide is decided here. When the
//! shell arrives at M4 it calls this same function and gets the same answers.
//!
//! M1 renders one scene. The queues of D-044 and the resume logic of D-042
//! arrive at M3 and will wrap this, not replace it.

use std::path::{Path, PathBuf};

use spoonstill_core::diagnostics::{Diagnostics, Event};
use spoonstill_core::{Aspect, MotionSpec, OutputSpec};
use spoonstill_media::scene::{Cancel, EncodeSettings, RenderedScene, SceneRequest};
use spoonstill_media::{MediaError, Progress, Tools};

/// What the caller asked for, before any of it has been resolved.
#[derive(Debug, Clone)]
pub struct RenderSceneOptions {
    /// The still.
    pub image: PathBuf,
    /// The narration.
    pub audio: PathBuf,
    /// Where the segment goes.
    pub out: PathBuf,
    /// Output aspect ratio.
    pub aspect: Aspect,
    /// Output short edge in pixels.
    pub short_edge: u32,
    /// Frame rate.
    pub fps: u32,
    /// An explicit move, or `None` to derive one from scene identity (D-035).
    pub motion: Option<MotionSpec>,
    /// Encoder settings.
    pub encode: EncodeSettings,
    /// Stable project identity, for the motion seed and the cache key.
    pub project_id: String,
    /// Scene index within the project.
    pub scene_index: u32,
}

impl Default for RenderSceneOptions {
    fn default() -> Self {
        Self {
            image: PathBuf::new(),
            audio: PathBuf::new(),
            out: PathBuf::new(),
            // D-070's recorded default is all three aspects in V1; 16:9 is the
            // one an operator gets without asking.
            aspect: Aspect::Landscape16x9,
            short_edge: 1080,
            fps: 30,
            motion: None,
            encode: EncodeSettings::default(),
            project_id: "single-scene".to_string(),
            scene_index: 0,
        }
    }
}

/// Everything that can stop a render before FFmpeg is involved.
#[derive(Debug)]
pub enum RenderError {
    /// Output geometry we will not render.
    Geometry(spoonstill_core::GeometryError),
    /// An input file that is not there.
    MissingInput {
        /// What it was for.
        role: &'static str,
        /// Where we looked.
        path: PathBuf,
    },
    /// Anything from the process boundary.
    Media(Box<MediaError>),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Geometry(e) => write!(f, "{e}"),
            RenderError::MissingInput { role, path } => {
                write!(f, "the {role} does not exist: {}", path.display())
            }
            RenderError::Media(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<spoonstill_core::GeometryError> for RenderError {
    fn from(e: spoonstill_core::GeometryError) -> Self {
        RenderError::Geometry(e)
    }
}
impl From<MediaError> for RenderError {
    fn from(e: MediaError) -> Self {
        RenderError::Media(Box::new(e))
    }
}

/// Where a project's machine-owned state — including the log — lives.
///
/// For a single-scene render there is no project directory, so the segment's
/// own directory stands in. That keeps the D-013 rule intact: machine state
/// sits beside the work, never inside the operator's manifest.
#[must_use]
pub fn state_root_for(out: &Path) -> PathBuf {
    out.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Render one scene.
///
/// # Errors
///
/// [`RenderError`] describing what stopped it. Every failure is also written to
/// the project's diagnostics log before it is returned, so a bundle exported
/// afterwards explains it (see [`crate::diagnostics`]).
pub fn render_scene(
    options: &RenderSceneOptions,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<RenderedScene, RenderError> {
    // Refuse a missing input here rather than letting FFmpeg say it, because
    // FFmpeg's message names the file without saying which of two roles it was
    // playing — and at 500 scenes that distinction is the whole diagnosis.
    for (role, path) in [("image", &options.image), ("narration", &options.audio)] {
        if !path.exists() {
            return Err(RenderError::MissingInput {
                role,
                path: path.clone(),
            });
        }
    }

    let output = OutputSpec::new(options.aspect, options.short_edge, options.fps)?;
    let tools = Tools::from_env();

    // The log is opened before the render, not after a failure — the whole
    // point is that the machine wrote things down while it was going wrong.
    // Both sinks, not just the folder's own (D-148). `still render-scene` used
    // to write only into the project it wrote its segment beside, so a scene
    // that failed here was invisible in the file D-093 says to open.
    let root = state_root_for(&options.out);
    let journal = spoonstill_state::Journal::for_project(&root);
    let sink: &dyn Diagnostics = &journal;

    sink.record(
        &Event::info("cli", "render-scene invoked")
            .with("image", options.image.display().to_string())
            .with("audio", options.audio.display().to_string())
            .with("out", options.out.display().to_string())
            .with("aspect", options.aspect.as_str())
            .with("short_edge", options.short_edge.to_string())
            .with("fps", options.fps.to_string())
            .with("preset", options.encode.preset.clone())
            .with("crf", options.encode.crf.to_string()),
    );

    let mut request = SceneRequest::new(
        options.image.clone(),
        options.audio.clone(),
        options.out.clone(),
        output,
    );
    request.motion = options.motion;
    request.project_id = options.project_id.clone();
    request.scene_index = options.scene_index;
    request.encode = options.encode.clone();

    let rendered = spoonstill_media::render_scene(&tools, &request, cancel, sink, on_progress)?;

    sink.record(
        &Event::info("render", "scene complete")
            .with("out", rendered.path.display().to_string())
            .with("frames", rendered.frames.to_string())
            .with("duration_s", format!("{:.6}", rendered.duration))
            .with("motion", rendered.motion.descriptor()),
    );

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that certainly exists, for the checks that run before anything
    /// is read. `file!()` is workspace-relative while tests run from the crate
    /// directory, so it is not the right answer here.
    fn existing_file() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
    }

    /// A missing input is named by role, because "no such file" about one of
    /// two paths is half a diagnosis.
    #[test]
    fn a_missing_input_is_named_by_role() {
        let mut options = RenderSceneOptions {
            image: PathBuf::from("/nonexistent/still.jpg"),
            audio: PathBuf::from("/nonexistent/narration.wav"),
            out: PathBuf::from("/tmp/spoonstill-test-out.mp4"),
            ..Default::default()
        };
        let error = render_scene(&options, &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("image"), "{error}");
        assert!(error.to_string().contains("still.jpg"), "{error}");

        // With the image present, the narration is the one reported. Any real
        // file will do — the existence check runs before anything reads it.
        options.image = existing_file();
        let error = render_scene(&options, &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("narration"), "{error}");
    }

    /// Geometry is refused before a process is spawned.
    #[test]
    fn unrenderable_geometry_is_refused_early() {
        let here = existing_file();
        let options = RenderSceneOptions {
            image: here.clone(),
            audio: here,
            out: PathBuf::from("/tmp/spoonstill-test-out.mp4"),
            short_edge: 1001, // odd: H.264 4:2:0 cannot represent it
            ..Default::default()
        };
        let error = render_scene(&options, &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(matches!(error, RenderError::Geometry(_)), "{error}");
    }

    /// Machine state sits beside the work, never in the operator's directory
    /// tree by surprise (D-013).
    #[test]
    fn the_state_root_is_the_output_directory() {
        assert_eq!(
            state_root_for(Path::new("/renders/proj/seg.mp4")),
            PathBuf::from("/renders/proj")
        );
        assert_eq!(state_root_for(Path::new("seg.mp4")), PathBuf::from("."));
    }

    /// D-070's recorded default, made visible: all three aspects exist, and
    /// 16:9 is what an operator gets without asking.
    #[test]
    fn the_default_output_is_1080p30_landscape() {
        let d = RenderSceneOptions::default();
        assert_eq!(d.aspect, Aspect::Landscape16x9);
        assert_eq!((d.short_edge, d.fps), (1080, 30));
        let spec = OutputSpec::new(d.aspect, d.short_edge, d.fps).unwrap();
        assert_eq!((spec.width(), spec.height()), (1920, 1080));
    }
}
