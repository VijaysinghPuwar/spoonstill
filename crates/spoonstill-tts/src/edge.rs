//! Edge TTS — Microsoft's neural voices, through the maintained `edge-tts`
//! command line tool.
//!
//! ## Why a subprocess rather than a Rust client
//!
//! D-023 already decided what Edge TTS *is* here: the internal and development
//! provider, free, never load-bearing in a sold build, because it speaks to a
//! reverse-engineered endpoint. That endpoint moves — it has an anti-abuse
//! token derived from a clock skew and a shared secret, and it has changed
//! more than once. Reimplementing it in Rust would mean owning a protocol whose
//! specification is "whatever Edge does this month", and shipping a build that
//! stops working on a Tuesday.
//!
//! `edge-tts` is the Python implementation that tracks those changes and is
//! already installed on the machines that want this provider. So this module
//! spawns it — through `spoonstill_media::command`, the one place a process is
//! spawned (D-011), which means argument vectors, a timeout, retained stderr
//! and a paste-ready command line for the diagnostics bundle, all for free.
//!
//! A provider that speaks HTTP itself, as ElevenLabs will, implements the same
//! trait and needs none of this.
//!
//! ## The text goes in a file, never in an argument
//!
//! `edge-tts` takes either `--text` or `--file`. This module always uses
//! `--file`, for three reasons, in increasing order of importance:
//!
//! 1. A paragraph can exceed the operating system's argument length limit.
//! 2. Arguments are visible to every process on the machine through `ps`.
//! 3. **The command line is logged** (D-016) and lands in the bundle the
//!    operator sends us. Their script is their content; it does not belong in
//!    our diagnostics, and the only reliable way to keep it out is for it never
//!    to be an argument.
//!
//! ## A network call in the middle of a 500-line batch (D-094)
//!
//! Everything else this program does is local and deterministic. This is the
//! one step that crosses a network, and it is the step that will fail while
//! nobody is watching. So the failure is **classified before it is reported**:
//! a dropped socket is tried again with a growing pause, and a line the service
//! will never speak — a bad voice, a line of only punctuation — is reported
//! immediately rather than three times slowly. `still render` at n=500 makes
//! that difference twenty-five minutes wide.
//!
//! The three facts the behaviour here is built on, all measured on this machine
//! on 2026-08-26 against `edge-tts 7.2.8`:
//!
//! - A short line takes about 0.6 s; **5 980 characters took 37.6 s** — about
//!   6.3 ms per character. A fixed 90 s ceiling therefore refuses a long
//!   paragraph on a slow link while calling it "the network is gone", which is
//!   why the ceiling is now derived from the length of the line.
//! - Every failure mode puts a **Python exception on the last line of stderr**
//!   and exits non-zero. `NoAudioReceived` and `ValueError: Invalid voice` are
//!   permanent; anything from `aiohttp` or a websocket is not.
//! - **Speaking one line twice does not produce the same bytes.** Two runs of
//!   the same text and voice gave files of identical length that differed in
//!   5 916 bytes. Nothing downstream depends on those bytes being stable —
//!   duration is measured, not assumed (D-021) — but it is why the raw speech
//!   cache of D-084 is load-bearing rather than an optimization: re-speaking a
//!   line is not a no-op.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_media::atomic;
use spoonstill_media::command::FfmpegCommand;

use crate::{Availability, Provider, Request, Spoken, TtsError, Voice, opening};

/// How long to let a package manager run. Installing pulls a dependency tree
/// over a network, so this is minutes rather than seconds.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// The package managers to try, in the order that works most often.
///
/// `pipx` first because it is the one that does not fight a system Python;
/// `brew` next on macOS; a `--user` pip last, which is the advice that fails on
/// a modern Homebrew Python and is therefore the fallback rather than the lead.
#[cfg(target_os = "macos")]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("pipx", &["install", "edge-tts"]),
    ("brew", &["install", "edge-tts"]),
    (
        "python3",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
];

/// Windows has no Homebrew, and its Python is not externally managed, so a
/// plain `pip --user` is the normal answer rather than the last resort.
#[cfg(not(target_os = "macos"))]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("pipx", &["install", "edge-tts"]),
    (
        "python",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
    (
        "python3",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
];

/// The last thing a failing tool said, which is the part that names the cause.
///
/// A Python traceback is twenty lines of our own irrelevance and one line of
/// diagnosis, and the diagnosis is always last. Package managers print pages;
/// an error box holds a sentence.
fn last_line(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_owned()
}

/// Environment override for the `edge-tts` binary. Development convenience,
/// and the hook a packaged build would use — same shape as
/// `spoonstill_media::tools` (D-012: no auto-download, no candidate search).
pub const EDGE_TTS_ENV: &str = "SPOONSTILL_EDGE_TTS";

/// The name used in `project.yaml`.
pub const ID: &str = "edge";

/// The sentence an operator needs when the tool is not there. One string, so
/// the pre-flight check, a failed render and the window all say the same thing.
fn how_to_install(program: &Path) -> String {
    format!(
        "`{}` is not on this machine. Install it with `pip install edge-tts` \
         (or `brew install edge-tts`), press Install in Settings, or point \
         {EDGE_TTS_ENV} at it.",
        program.display()
    )
}

/// What a line's ceiling starts at, before its length is considered.
///
/// This is the connection, the handshake and Python's own start-up — the part
/// that does not get bigger when the sentence does. Sixty seconds is not a
/// performance budget: it is the point past which "the network is gone" is a
/// better explanation than "this is taking a while".
const SPEAK_BASE: Duration = Duration::from_secs(60);

/// What each character of the line adds to that ceiling.
///
/// Measured at 6.3 ms/char (see the module note). Thirty is roughly five times
/// that, which is the headroom a hotel connection needs and still refuses a
/// wedged socket long before an operator gives up on it.
const SPEAK_PER_CHAR: Duration = Duration::from_millis(30);

/// No line waits longer than this, however long it is. A line this size is a
/// paragraph that should have been several scenes.
const SPEAK_CEILING: Duration = Duration::from_secs(900);

/// How long to give one line, given how long the line is.
fn speak_timeout(characters: usize) -> Duration {
    let scaled = SPEAK_PER_CHAR
        .checked_mul(u32::try_from(characters).unwrap_or(u32::MAX))
        .unwrap_or(SPEAK_CEILING);
    SPEAK_BASE.saturating_add(scaled).min(SPEAK_CEILING)
}

/// The largest slice of a line this provider sends in one request (D-095).
///
/// Long-form is where a single request stops being a good bet. Measured here:
/// 20 000 characters is one 128-second call producing 19 minutes of audio, and
/// **any failure anywhere in it throws away all 128 seconds**. At the scene cap
/// — an hour of narration, about 62 000 characters — that is a seven-minute
/// all-or-nothing request over a reverse-engineered endpoint.
///
/// Nine thousand characters is roughly 520 s of speech and 58 s of generation:
/// a unit of work worth retrying on its own. The number comes from the
/// author's own `setuptts`, which speaks to this same service in production
/// and settled on 9 200–10 500 after real use; the conservative end of that
/// range is the one to copy.
///
/// This is **not** a protocol limit. `edge-tts` splits its own websocket
/// payloads and knows the real byte ceiling far better than we do. This limit
/// exists to bound what one failure costs.
pub const CHUNK_CHARS: usize = 9_000;

/// A line at or under this length that comes back with no audio has nothing
/// speakable in it, and saying it again will not change that. Above it, the
/// same answer from the service is more likely to be about the payload.
///
/// Two hundred characters is a caption an operator can read at a glance — the
/// scale at which "there are no words in this" is a thing a human can verify.
const EXAMINABLE_CHARS: usize = 200;

/// Characters of text per second of speech, at `--rate +0%`.
///
/// **This is the fastest observed rate, on purpose.** Two measurements on this
/// machine:
///
/// | text | audio | rate |
/// |---|---|---|
/// | 20 000 chars of flowing prose | 1 153.2 s | 17.3 chars/s |
/// | 25 000 chars of short sentences | 2 099.7 s | 11.9 chars/s |
///
/// A 45% spread, because every full stop is a pause — the same voice reads
/// clipped narration far more slowly than it reads paragraphs. So there is no
/// single correct value here, and the only question is which end to use.
///
/// The fast end, because of what this number is *for*: refusing a line before
/// speaking it, on the grounds that no scene could hold the result. A limit
/// derived from the fast rate refuses only what **cannot** fit at any rate. A
/// limit derived from the slow rate would refuse lines that would have fitted,
/// which is a wrong answer given confidently — worse than the seven wasted
/// minutes it saves. Anything under the limit that still overruns is caught
/// downstream on its *measured* duration, which is the number that governs
/// (D-021).
///
/// Do not "correct" this to the slower figure.
const SPEECH_CHARS_PER_SECOND: f64 = 17.3;

/// The longest line that could fit in one scene.
///
/// A scene holds at most [`MAX_SCENE_SECONDS`] of narration (D-021), and
/// exceeding it is checked on the *measured* artifact — which today means a
/// 100 000-character script is spoken for eleven minutes, normalized by two
/// FFmpeg passes, and only then refused for being 1.6 hours long. Refusing it
/// before the first request costs nothing and saves all of that.
///
/// Deliberately generous: it is derived from the normal speaking rate, so a
/// line slowed with `--rate -50%` can still pass here and be caught downstream
/// on its measured duration. This limit is for the line nobody could have
/// meant, not for the borderline one.
fn max_line_chars() -> usize {
    (spoonstill_core::project::MAX_SCENE_SECONDS * SPEECH_CHARS_PER_SECOND) as usize
}

/// Split a line into pieces no longer than `limit`, at the most natural
/// boundary available.
///
/// Paragraph, then sentence, then word, then — only if a single word is longer
/// than the limit — a hard cut. The order matters for how the result *sounds*:
/// each piece is a separate request and the seam between two pieces is a seam
/// in the speech, so it belongs where a reader would have paused anyway.
///
/// Then the pieces are **packed**: consecutive units are merged while they
/// still fit. Cutting at every sentence would be correct and useless — an hour
/// of narration is fourteen hundred sentences, and fourteen hundred requests
/// is far worse than the one request this exists to avoid. The unit of work
/// wants to be as large as it can be while staying worth retrying.
///
/// Returns the whole line as one piece when it already fits, which is the case
/// for every line in a normal project.
fn split_line(text: &str, limit: usize) -> Vec<&str> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= limit {
        return vec![text];
    }

    // Atomic units, in order, as byte ranges into `text`. Ranges rather than
    // slices so that packing two of them keeps whatever separated them —
    // rejoining with a space of our own would edit the operator's line.
    let mut units: Vec<(usize, usize)> = Vec::new();
    for (from, to) in paragraphs(text) {
        if text[from..to].chars().count() <= limit {
            units.push((from, to));
            continue;
        }
        for (from, to) in sentences(text, from, to) {
            if text[from..to].chars().count() <= limit {
                units.push((from, to));
            } else {
                units.extend(words(text, from, to, limit));
            }
        }
    }

    let mut out: Vec<&str> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for (from, to) in units {
        current = match current {
            Some((start, end)) => {
                if text[start..to].chars().count() <= limit {
                    Some((start, to))
                } else {
                    out.push(text[start..end].trim());
                    Some((from, to))
                }
            }
            None => Some((from, to)),
        };
    }
    if let Some((start, end)) = current {
        out.push(text[start..end].trim());
    }
    out.retain(|piece| !piece.is_empty());
    out
}

/// Byte ranges of the paragraphs in `text`, split on a blank line.
fn paragraphs(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at + 1 < bytes.len() {
        if bytes[at] == b'\n' && bytes[at + 1] == b'\n' {
            let mut end = at + 2;
            while end < bytes.len() && bytes[end] == b'\n' {
                end += 1;
            }
            if at > start {
                out.push((start, at));
            }
            start = end;
            at = end;
        } else {
            at += 1;
        }
    }
    if start < text.len() {
        out.push((start, text.len()));
    }
    out
}

/// Byte ranges of the sentences in `text[from..to]`.
///
/// Cut after `.`, `!`, `?`, `…` or a full-width stop when whitespace follows,
/// keeping the punctuation — and any closing quote or bracket — with the
/// sentence it ends.
fn sentences(text: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let slice = &text[from..to];
    let mut out = Vec::new();
    let mut start = from;
    let chars: Vec<(usize, char)> = slice.char_indices().collect();

    for (position, (_, ch)) in chars.iter().enumerate() {
        if !matches!(ch, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            continue;
        }
        // Run past a cluster like "?!" or `."` so it stays with its sentence.
        let after = chars[position + 1..].iter().find(|(_, next)| {
            !matches!(next, '.' | '!' | '?' | '…' | '"' | '\'' | '”' | '’' | ')')
        });
        if let Some((at, next)) = after
            && next.is_whitespace()
        {
            let cut = from + at;
            if cut > start {
                out.push((start, cut));
            }
            start = cut;
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

/// Last resort: byte ranges that fill to `limit` on word boundaries, cutting a
/// word only when one word is longer than a whole chunk.
fn words(text: &str, from: usize, to: usize, limit: usize) -> Vec<(usize, usize)> {
    let slice = &text[from..to];
    let mut out = Vec::new();
    let mut start = from;
    let mut count = 0;
    let mut last_space: Option<usize> = None;

    for (offset, ch) in slice.char_indices() {
        let at = from + offset;
        if count == limit {
            let cut = last_space.filter(|space| *space > start).unwrap_or(at);
            out.push((start, cut));
            start = cut;
            count = text[start..at].chars().count();
            last_space = None;
        }
        if ch.is_whitespace() {
            last_space = Some(at);
        }
        count += 1;
    }
    if start < to {
        out.push((start, to));
    }
    out
}

/// Listing voices talks to the same service and returns 300-odd rows.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// `--version` should answer immediately or not at all.
const LIMIT_VERSION: Duration = Duration::from_secs(20);

/// The default voice, used when a project names none.
///
/// `edge-tts` has its own default and it changes between releases; naming ours
/// keeps a render reproducible across an upgrade of a tool we do not pin
/// (D-043: a cache key that hashes "default" must mean one voice).
pub const DEFAULT_VOICE: &str = "en-US-AvaNeural";

/// Whether one failure is worth trying again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The socket, the service or the machine was momentarily busy.
    Transient,
    /// Nothing about waiting will change the answer.
    Permanent,
}

/// What one attempt left behind.
enum Failure {
    /// Report this, now. The error is already shaped for the operator.
    Permanent(TtsError),
    /// Worth another attempt. The string is what the last one said, kept so
    /// that giving up can quote it.
    Transient(String),
}

/// How many times a line is attempted, and how long the pauses grow.
///
/// Three attempts rather than five: the failures worth retrying are a dropped
/// websocket and a rate limit, and both clear in seconds or not at all. Beyond
/// three, a batch is spending minutes proving something it already knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// Total attempts, including the first. One means never retry.
    pub attempts: u32,
    /// The pause before the second attempt; tripled for each one after.
    pub backoff: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            attempts: 3,
            backoff: Duration::from_millis(500),
        }
    }
}

impl Retry {
    /// Never retry — what a test uses, and what a caller that has its own
    /// supervision would ask for.
    #[must_use]
    pub const fn once() -> Self {
        Retry {
            attempts: 1,
            backoff: Duration::ZERO,
        }
    }

    /// The pause before attempt number `attempt` (1-based, so attempt 1 has
    /// none). Deliberately deterministic: a jittered backoff would make the
    /// slowest failing render un-reproducible for no gain at this scale, where
    /// the pool is four workers and not four hundred.
    fn pause_before(self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }
        self.backoff
            .checked_mul(3u32.saturating_pow(attempt - 2))
            .unwrap_or(self.backoff)
    }
}

/// Run `attempt` until it succeeds, fails permanently, or runs out of tries.
///
/// Separated from the process boundary on purpose: retry logic that can only be
/// exercised by unplugging a network cable is retry logic nobody tests, and
/// this is the code that runs when everything else has already gone wrong.
/// `rest` is the sleep, injected so a test costs nothing.
fn persevere<T>(
    policy: Retry,
    mut attempt: impl FnMut(u32) -> Result<T, Failure>,
    mut rest: impl FnMut(Duration),
) -> Result<T, (u32, Failure)> {
    let total = policy.attempts.max(1);
    let mut last = String::new();
    for number in 1..=total {
        let pause = policy.pause_before(number);
        if !pause.is_zero() {
            rest(pause);
        }
        match attempt(number) {
            Ok(value) => return Ok(value),
            Err(Failure::Permanent(error)) => return Err((number, Failure::Permanent(error))),
            Err(Failure::Transient(reason)) => last = reason,
        }
    }
    Err((total, Failure::Transient(last)))
}

/// Read a failing `edge-tts` run and decide whether waiting could help.
///
/// Classified on the **last** line of stderr, which is where Python puts the
/// exception, and matched on the exception's own class name rather than on
/// prose — the sentence after the colon is written for a human and changes;
/// `edge_tts.exceptions.NoAudioReceived` is an identifier and does not.
///
/// The default is [`Fault::Permanent`]. An unrecognised failure is far more
/// likely to be a wrong argument than a flaky socket, and retrying every
/// unrecognised failure three times turns one bad project into a batch that
/// takes half an hour to tell you so.
fn classify(stderr: &str) -> Fault {
    let line = last_line(stderr);

    // Permanent first: a `NoAudioReceived` traceback can pass through network
    // code on its way out, and the outermost exception is the true one.
    const PERMANENT: &[&str] = &[
        "NoAudioReceived",
        "ValueError",
        "TypeError",
        "usage: edge-tts",
        "unrecognized arguments",
        "No such file or directory",
        "Permission denied",
        "ModuleNotFoundError",
    ];
    if PERMANENT.iter().any(|marker| line.contains(marker)) {
        return Fault::Permanent;
    }

    // Everything the service and the socket can do to us. `WebSocketError`
    // covers the 403 that arrives when the anti-abuse token has moved and the
    // installed `edge-tts` has not caught up — worth one more attempt, because
    // it is also what a proxy returns when it is having a moment.
    const TRANSIENT: &[&str] = &[
        "WebSocketError",
        "UnexpectedResponse",
        "UnknownResponse",
        "SkewAdjustmentError",
        "aiohttp",
        "websockets",
        "ClientConnector",
        "ServerDisconnected",
        "ConnectionResetError",
        "ConnectionRefusedError",
        "TimeoutError",
        "socket.gaierror",
        "ssl.SSLError",
        "Temporary failure in name resolution",
        "429",
        "503",
    ];
    if TRANSIENT.iter().any(|marker| line.contains(marker)) {
        return Fault::Transient;
    }

    Fault::Permanent
}

/// Turn one failed run into a shaped error, or into a reason to try again.
///
/// The shaped errors are the point. `ValueError: Invalid voice 'en-GB-Ryan'.`
/// with twenty lines of Python above it is a stack trace; "en-GB-Ryan is not
/// one of this provider's voices — run `still voices`" is an instruction.
fn interpret(error: spoonstill_media::MediaError, voice: &str, text: &str) -> Failure {
    use spoonstill_media::MediaError;

    match error {
        // The tool is not there. Nothing to wait for, and the sentence is the
        // same one the pre-flight check and the window print.
        MediaError::BinaryMissing { ref tried, .. } => Failure::Permanent(TtsError::Unavailable {
            provider: ID.to_owned(),
            detail: how_to_install(tried),
        }),
        // We killed it. Either the line is enormous or the socket is wedged;
        // both are worth one more go with a fresh connection.
        MediaError::Timeout { waited, .. } => Failure::Transient(format!(
            "it did not finish within {:.0}s",
            waited.as_secs_f64()
        )),
        // Spawning failed for a reason other than "not found" — under a worker
        // pool that is a resource limit, which passes.
        MediaError::Spawn { ref source, .. } => {
            Failure::Transient(format!("could not start it: {source}"))
        }
        MediaError::Exit { ref stderr, .. } => {
            let said = last_line(stderr);
            match classify(stderr) {
                Fault::Transient => Failure::Transient(said),
                Fault::Permanent if said.contains("Invalid voice") => {
                    Failure::Permanent(TtsError::BadRequest {
                        provider: ID.to_owned(),
                        detail: format!(
                            "{voice:?} is not one of this provider's voices — \
                             run `still voices` to see the ones that are"
                        ),
                    })
                }
                // `NoAudioReceived` means two different things depending on
                // how much was asked for, and the author's own `setuptts`
                // learned the difference in production: it retries no-audio at
                // full size once and then re-splits smaller.
                //
                // On a line short enough to read at a glance, it is the truth
                // — there is nothing in it to say. On a long one it is also
                // what the service does when a payload does not suit it, and a
                // second request can succeed where the first did not.
                Fault::Permanent if said.contains("NoAudioReceived") => {
                    if text.chars().count() <= EXAMINABLE_CHARS {
                        Failure::Permanent(TtsError::NoAudio {
                            provider: ID.to_owned(),
                            text: opening(text),
                            detail: format!(
                                "the service accepted the request for {voice} and returned no \
                                 audio. A line with no speakable words in it — only punctuation, \
                                 digits or symbols — does this."
                            ),
                        })
                    } else {
                        Failure::Transient(format!(
                            "the service returned no audio for {} characters",
                            text.chars().count()
                        ))
                    }
                }
                Fault::Permanent => Failure::Permanent(TtsError::NoAudio {
                    provider: ID.to_owned(),
                    text: opening(text),
                    detail: said,
                }),
            }
        }
        other => Failure::Transient(other.to_string()),
    }
}

/// Whether these bytes could be an audio file at all.
///
/// `edge-tts` writes an MP3 today. This does not require one: it refuses only
/// what is not any container we could hand to FFmpeg — an HTML error page, a
/// JSON refusal, a fragment of a Python traceback. A truncated MP3 still passes
/// here and is caught downstream, where duration is measured (D-021); the point
/// of this check is to name the *provider* when the provider is what went
/// wrong, instead of surfacing it two steps later as "FFmpeg could not read
/// the narration".
fn looks_like_audio(head: &[u8]) -> bool {
    match head {
        [] => false,
        // MPEG frame sync, in the two shapes a bare MP3 starts with.
        [0xFF, b, ..] if b & 0xE0 == 0xE0 => true,
        _ => {
            const CONTAINERS: &[&[u8]] = &[
                b"ID3",              // MP3 with a tag
                b"RIFF",             // WAV
                b"OggS",             // Ogg / Opus
                b"fLaC",             // FLAC
                b"\x1A\x45\xDF\xA3", // Matroska / WebM
                b"\x00\x00\x00",     // ISO-BMFF: a box length, then `ftyp`
            ];
            CONTAINERS.iter().any(|magic| head.starts_with(magic))
        }
    }
}

/// Microsoft's neural voices, via `edge-tts`.
#[derive(Debug, Clone)]
pub struct Edge {
    program: PathBuf,
    retry: Retry,
}

impl Edge {
    /// Use whichever binary the environment names, falling back to `PATH`.
    #[must_use]
    pub fn from_env() -> Self {
        Edge {
            program: std::env::var_os(EDGE_TTS_ENV)
                .map_or_else(|| PathBuf::from("edge-tts"), PathBuf::from),
            retry: Retry::default(),
        }
    }

    /// Use an explicitly located binary — what a packaged build does, and what
    /// a test does.
    #[must_use]
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Edge {
            program: program.into(),
            retry: Retry::default(),
        }
    }

    /// Change how hard this provider tries before it gives up.
    #[must_use]
    pub const fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// The binary this provider will run.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// One attempt at one line: run the tool, then check what it wrote.
    ///
    /// The audio lands on `audio`, a temporary beside the destination. Moving
    /// it into place is the caller's last act, after the last thing that can
    /// fail has not.
    fn attempt_speak(
        &self,
        voice: &str,
        text: &str,
        knobs: &Knobs,
        script: &Path,
        audio: &Path,
    ) -> Result<Spoken, Failure> {
        let mut command = FfmpegCommand::new(&self.program);
        command
            .arg("--voice")
            .arg(voice)
            .arg("--rate")
            .arg(&knobs.rate)
            .arg("--volume")
            .arg(&knobs.volume)
            .arg("--pitch")
            .arg(&knobs.pitch)
            .arg("--file")
            .arg(script)
            .arg("--write-media")
            .arg(audio);

        let how = command.display();
        let outcome = command
            .spawn()
            .and_then(|child| child.wait_until(speak_timeout(text.chars().count())))
            .and_then(spoonstill_media::Finished::ok);

        // Whatever happened, a failed attempt leaves no half-file for the next
        // one to append to or for the caller to mistake for output.
        let finished = match outcome {
            Ok(finished) => finished,
            Err(e) => {
                let _ = std::fs::remove_file(audio);
                return Err(interpret(e, voice, text));
            }
        };

        // `edge-tts` reports some refusals by exiting zero and writing nothing
        // — the failure mode a bare exit-status check calls success.
        let bytes = std::fs::metadata(audio).map(|m| m.len()).unwrap_or(0);
        if bytes == 0 {
            let _ = std::fs::remove_file(audio);
            return Err(Failure::Permanent(TtsError::NoAudio {
                provider: ID.to_owned(),
                text: opening(text),
                detail: if finished.stderr.trim().is_empty() {
                    format!("the service accepted the request for {voice} and returned no audio")
                } else {
                    last_line(&finished.stderr)
                },
            }));
        }

        if !head_of(audio).is_some_and(|head| looks_like_audio(&head)) {
            let _ = std::fs::remove_file(audio);
            return Err(Failure::Transient(format!(
                "it wrote {bytes} bytes that are not an audio file"
            )));
        }

        Ok(Spoken { bytes, how })
    }

    /// One piece of a line: write it, say it, retry it if the service faltered.
    fn say_one(
        &self,
        voice: &str,
        text: &str,
        knobs: &Knobs,
        destination: &Path,
        audio: &Path,
    ) -> Result<Spoken, TtsError> {
        let script = atomic::partial_path(&destination.with_extension("txt"));
        write_script(&script, text)?;

        // Retries reuse the script — the words have not changed, and rewriting
        // them would be the one part of this loop that can fail for a reason
        // that has nothing to do with the service.
        let outcome = persevere(
            self.retry,
            |_attempt| self.attempt_speak(voice, text, knobs, &script, audio),
            std::thread::sleep,
        );

        // The script is the operator's words on our disk. It goes away whether
        // the run worked or not, and before the error is even shaped.
        let _ = std::fs::remove_file(&script);

        match outcome {
            Ok(spoken) => Ok(spoken),
            Err((_, Failure::Permanent(error))) => Err(error),
            Err((attempts, Failure::Transient(reason))) => Err(TtsError::NoAudio {
                provider: ID.to_owned(),
                text: opening(text),
                detail: format!(
                    "the voice service failed {attempts} time{} in a row. \
                     The last attempt said: {reason}",
                    if attempts == 1 { "" } else { "s" }
                ),
            }),
        }
    }
}

/// The first few bytes of a file, or nothing if it cannot be read.
fn head_of(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 8];
    let read = file.read(&mut head).ok()?;
    Some(head[..read].to_vec())
}

/// The three knobs, already validated into the forms `edge-tts` accepts.
struct Knobs {
    rate: String,
    volume: String,
    pitch: String,
}

impl Default for Edge {
    fn default() -> Self {
        Self::from_env()
    }
}

impl Provider for Edge {
    fn id(&self) -> &str {
        ID
    }

    fn availability(&self) -> Availability {
        let mut command = FfmpegCommand::new(&self.program);
        command.arg("--version");
        match command.spawn().and_then(|c| c.wait_until(LIMIT_VERSION)) {
            Ok(finished) if finished.status.success() => Availability::Ready,
            Ok(finished) => Availability::Missing(format!(
                "`{} --version` failed: {}",
                self.program.display(),
                last_line(&finished.stderr)
            )),
            Err(_) => Availability::Missing(how_to_install(&self.program)),
        }
    }

    fn default_voice(&self) -> &str {
        DEFAULT_VOICE
    }

    /// Fetch `edge-tts` through whichever package manager this machine has.
    ///
    /// Told to run `pip install edge-tts`, an operator on a Homebrew Python
    /// meets `error: externally-managed-environment` and stops. So the
    /// candidates are tried in the order that works most often, and the first
    /// one that both exists and succeeds wins. Nothing is downloaded from us
    /// (D-062, D-012): every candidate is the platform's own package manager,
    /// run because a button that says install was pressed.
    fn install(&self) -> Result<String, TtsError> {
        // Already there. Five minutes of a package manager re-resolving a
        // dependency tree is not what a button press asked for.
        if self.availability() == Availability::Ready {
            return Ok(format!("{} is already installed", self.program.display()));
        }

        let mut tried: Vec<String> = Vec::new();

        for (program, args) in INSTALLERS {
            let mut command = FfmpegCommand::new(*program);
            command.args(*args);
            let shown = command.display();

            match command.spawn().and_then(|c| c.wait_until(INSTALL_TIMEOUT)) {
                Ok(finished) if finished.status.success() => {
                    // Present is not the same as runnable: a `pip --user`
                    // install can land somewhere that is not on PATH, and
                    // reporting success there would be reporting a lie.
                    return match self.availability() {
                        Availability::Ready => Ok(shown),
                        Availability::Missing(detail) => Err(TtsError::Unavailable {
                            provider: ID.to_owned(),
                            detail: format!(
                                "`{shown}` succeeded but {} still cannot be run \
                                 — it is probably not on PATH. {detail}",
                                self.program.display()
                            ),
                        }),
                    };
                }
                Ok(finished) => tried.push(format!(
                    "`{shown}` exited {}: {}",
                    finished.status.code().unwrap_or(-1),
                    last_line(&finished.stderr)
                )),
                // Not installed on this machine — the next candidate is the
                // point of having a list.
                Err(_) => tried.push(format!("`{program}` is not on this machine")),
            }
        }

        Err(TtsError::Unavailable {
            provider: ID.to_owned(),
            detail: format!("could not install it. {}", tried.join("; ")),
        })
    }

    /// The whole catalogue, over the same network as a line of speech — so it
    /// is retried on the same terms, and an unreadable answer is an error
    /// rather than an empty list.
    ///
    /// An empty list is the worst possible outcome here: the window would draw
    /// three hundred voices as none, with nothing wrong on screen and nothing
    /// in the log. If `edge-tts` printed something this build cannot parse, the
    /// operator is told exactly that, and shown the line we could not read.
    fn voices(&self) -> Result<Vec<Voice>, TtsError> {
        let list = |_attempt: u32| -> Result<Vec<Voice>, Failure> {
            let mut command = FfmpegCommand::new(&self.program);
            command.arg("--list-voices");
            let finished = command
                .spawn()
                .and_then(|c| c.wait_until(LIST_TIMEOUT))
                .and_then(spoonstill_media::Finished::ok)
                .map_err(|e| interpret(e, "", ""))?;

            let table = String::from_utf8_lossy(&finished.stdout);
            let voices = parse_voices(&table);
            if voices.is_empty() {
                let first = table.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                return Err(Failure::Permanent(TtsError::Unavailable {
                    provider: ID.to_owned(),
                    detail: if first.is_empty() {
                        format!("`{} --list-voices` printed nothing", self.program.display())
                    } else {
                        format!(
                            "`{} --list-voices` printed a table this build cannot read, \
                             beginning {first:?}. A newer edge-tts may have changed its \
                             output.",
                            self.program.display()
                        )
                    },
                }));
            }
            Ok(voices)
        };

        match persevere(self.retry, list, std::thread::sleep) {
            Ok(voices) => Ok(voices),
            Err((_, Failure::Permanent(error))) => Err(error),
            Err((attempts, Failure::Transient(reason))) => Err(TtsError::Unavailable {
                provider: ID.to_owned(),
                detail: format!(
                    "could not list its voices after {attempts} attempt{}: {reason}",
                    if attempts == 1 { "" } else { "s" }
                ),
            }),
        }
    }

    fn speak(&self, request: &Request<'_>, destination: &Path) -> Result<Spoken, TtsError> {
        if request.text.trim().is_empty() {
            return Err(TtsError::BadRequest {
                provider: ID.to_owned(),
                detail: "the line is empty".to_owned(),
            });
        }

        let voice = if request.voice.is_empty() || request.voice == "default" {
            DEFAULT_VOICE
        } else {
            request.voice
        };

        // A line no scene could hold is refused before the first request
        // rather than after the last one (D-095). Speaking it would take
        // minutes, normalizing it two FFmpeg passes, and the measured duration
        // would refuse it anyway.
        let characters = request.text.chars().count();
        let limit = max_line_chars();
        if characters > limit {
            return Err(TtsError::BadRequest {
                provider: ID.to_owned(),
                detail: format!(
                    "this line is {characters} characters — about {:.0} minutes of speech, \
                     and one scene holds at most {:.0} (D-021). Split it across scenes.",
                    characters as f64 / SPEECH_CHARS_PER_SECOND / 60.0,
                    spoonstill_core::project::MAX_SCENE_SECONDS / 60.0,
                ),
            });
        }

        // Every knob is validated before anything is spawned or written: a
        // typo in `project.yaml` should not cost a process, a temporary file
        // and a network round trip to discover, five hundred times.
        let knobs = Knobs {
            rate: percent(request.setting("rate"), "rate")?,
            volume: percent(request.setting("volume"), "volume")?,
            pitch: hertz(request.setting("pitch"))?,
        };

        // One piece for every normal line; several for long-form, so that one
        // dropped connection costs a minute rather than the whole narration.
        let pieces = split_line(request.text, CHUNK_CHARS);
        if pieces.is_empty() {
            return Err(TtsError::BadRequest {
                provider: ID.to_owned(),
                detail: "the line is empty".to_owned(),
            });
        }

        // The temporary lives beside the destination: a rename within one
        // filesystem is atomic, one across two is a copy (D-042).
        let audio = atomic::partial_path(destination);
        let mut parts: Vec<PathBuf> = Vec::new();
        let mut spoken = Spoken {
            bytes: 0,
            how: String::new(),
        };

        for (index, piece) in pieces.iter().enumerate() {
            let part = if pieces.len() == 1 {
                audio.clone()
            } else {
                atomic::partial_path(&destination.with_extension(format!("part{index}.mp3")))
            };
            match self.say_one(voice, piece, &knobs, destination, &part) {
                Ok(said) => {
                    spoken.bytes += said.bytes;
                    // The command of the first piece, which is the one an
                    // operator would paste. The others differ only in which
                    // temporary file held the words.
                    if spoken.how.is_empty() {
                        spoken.how = said.how;
                    }
                    parts.push(part);
                }
                Err(error) => {
                    for part in &parts {
                        let _ = std::fs::remove_file(part);
                    }
                    let _ = std::fs::remove_file(&part);
                    return Err(match error {
                        // Name the piece, or an operator reading "no audio for
                        // 'The harbour…'" cannot tell which of eleven requests
                        // failed.
                        TtsError::NoAudio {
                            provider,
                            text,
                            detail,
                        } if pieces.len() > 1 => TtsError::NoAudio {
                            provider,
                            text,
                            detail: format!("{detail} (part {} of {})", index + 1, pieces.len()),
                        },
                        other => other,
                    });
                }
            }
        }

        if pieces.len() > 1 {
            join_mp3(&parts, &audio)?;
            for part in &parts {
                let _ = std::fs::remove_file(part);
            }
            spoken.bytes = std::fs::metadata(&audio)
                .map(|m| m.len())
                .unwrap_or(spoken.bytes);
            spoken.how = format!("{} (and {} more parts)", spoken.how, pieces.len() - 1);
        }

        atomic::move_into_place(&audio, destination)?;
        Ok(spoken)
    }
}

/// Join the pieces of one line into a single MP3, in order.
///
/// **By concatenating the bytes**, which for MPEG audio is a real join: the
/// format is a stream of self-describing frames with no container around them
/// and no index to fix up, which is why `cat a.mp3 b.mp3` has always worked.
/// No FFmpeg process, no transcode, no second copy of the audio in memory.
///
/// The one artifact is that each piece after the first begins with its
/// encoder's info frame, which decodes as about 26 ms of silence. Measured
/// against a seam in a sentence, that is a breath; against the alternative —
/// re-encoding an hour of speech to remove it — it is not worth paying for.
/// Nothing downstream is misled by it either, because the duration that
/// reaches the renderer is *measured* on the normalized artifact and never
/// added up from parts (D-021).
fn join_mp3(parts: &[PathBuf], out: &Path) -> Result<(), TtsError> {
    use std::io::{BufWriter, copy};

    let file = std::fs::File::create(out).map_err(|source| TtsError::Io {
        path: out.to_path_buf(),
        source,
    })?;
    let mut writer = BufWriter::new(file);
    for part in parts {
        let mut reader = std::fs::File::open(part).map_err(|source| TtsError::Io {
            path: part.clone(),
            source,
        })?;
        copy(&mut reader, &mut writer).map_err(|source| TtsError::Io {
            path: out.to_path_buf(),
            source,
        })?;
    }
    writer.flush().map_err(|source| TtsError::Io {
        path: out.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Put the line in a file for `edge-tts --file` to read.
///
/// Not `sync_all`: this file is deleted before the function that made it
/// returns, so durability across a power cut is a guarantee nobody needs, and
/// at n=500 it is five hundred fsyncs bought for nothing. Closing the handle
/// before the child opens it is the part that matters, and that is what
/// dropping it does.
fn write_script(path: &Path, text: &str) -> Result<(), TtsError> {
    let attempt = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(text.as_bytes())?;
        file.flush()
    };
    attempt().map_err(|source| {
        // A half-written script is worse than none: `edge-tts` would speak it.
        let _ = std::fs::remove_file(path);
        TtsError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Validate a percentage knob into the `+N%` form `edge-tts` requires.
///
/// Absent means `+0%`. A value the tool would reject is caught here, with the
/// knob's name attached, rather than as a stderr line from a subprocess in the
/// middle of a 500-line batch.
fn percent(value: Option<&str>, knob: &str) -> Result<String, TtsError> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok("+0%".to_owned());
    };
    let number = raw.trim_end_matches('%');
    let parsed: i32 = number.parse().map_err(|_| TtsError::BadRequest {
        provider: ID.to_owned(),
        detail: format!("{knob} {raw:?} is not a percentage like -10% or +25%"),
    })?;
    if !(-100..=200).contains(&parsed) {
        return Err(TtsError::BadRequest {
            provider: ID.to_owned(),
            detail: format!("{knob} {parsed}% is outside the range -100% to +200%"),
        });
    }
    Ok(format!("{parsed:+}%"))
}

/// The same, for pitch, which Edge states in hertz.
fn hertz(value: Option<&str>) -> Result<String, TtsError> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok("+0Hz".to_owned());
    };
    let number = raw
        .trim_end_matches(|c: char| c.eq_ignore_ascii_case(&'z') || c.eq_ignore_ascii_case(&'h'));
    let parsed: i32 = number.trim().parse().map_err(|_| TtsError::BadRequest {
        provider: ID.to_owned(),
        detail: format!("pitch {raw:?} is not a shift like -20Hz or +50Hz"),
    })?;
    if !(-100..=100).contains(&parsed) {
        return Err(TtsError::BadRequest {
            provider: ID.to_owned(),
            detail: format!("pitch {parsed}Hz is outside the range -100Hz to +100Hz"),
        });
    }
    Ok(format!("{parsed:+}Hz"))
}

/// The locale part of a voice id, as a tag the platform can name (D-086).
///
/// Microsoft spells a voice `<locale>-<Name>Neural`, and for 316 of the 322
/// voices in the recorded catalogue the locale is the first two segments. The
/// other six are why this is a function:
///
/// - `iu-Cans-CA-SiqiniqNeural` — language, **script**, region. Taking two
///   segments gives `iu-Cans`, which is not a locale, which the window's
///   `Intl.DisplayNames` cannot name, and which leaves the voice listed as
///   "CA-Siqiniq".
/// - `zh-CN-liaoning-XiaobeiNeural` — language, region, and a **dialect** that
///   is not a BCP-47 subtag at all. Taking three segments gives `zh-CN-liaoning`,
///   which is equally unnameable.
///
/// So the segments are read positionally — language, then an optional
/// four-letter script, then an optional region — and anything that does not fit
/// one of those slots ends the tag rather than joining it.
fn locale_of(id: &str) -> String {
    let parts: Vec<&str> = id.split('-').collect();
    // The last segment is the voice's own name, never part of the locale.
    let Some((_, head)) = parts.split_last() else {
        return String::new();
    };
    let mut out: Vec<&str> = Vec::new();
    let mut segments = head.iter();

    // Language: two or three letters, and required.
    match segments.next() {
        Some(part) if (2..=3).contains(&part.len()) && part.chars().all(char::is_alphabetic) => {
            out.push(part);
        }
        _ => return String::new(),
    }

    let mut next = segments.next();
    // Script: exactly four letters, optional.
    if let Some(part) = next
        && part.len() == 4
        && part.chars().all(char::is_alphabetic)
    {
        out.push(part);
        next = segments.next();
    }
    // Region: two letters or three digits, optional.
    if let Some(part) = next
        && (part.len() == 2 && part.chars().all(char::is_alphabetic)
            || part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
    {
        out.push(part);
    }

    out.join("-")
}

/// Read `edge-tts --list-voices`, which prints a fixed-width table.
///
/// Parsed by splitting on runs of two or more spaces rather than by column
/// offset: the widths follow the longest value in each column, so they differ
/// between releases and between locales. A row that does not have at least a
/// name is skipped rather than guessed at.
///
/// The `----` rule is found rather than assumed. A table with no rule — which
/// is what a future `edge-tts` that drops it would print — falls back to
/// skipping a header line whose first word is `Name`, so that a cosmetic
/// change to the tool costs an operator nothing.
fn parse_voices(table: &str) -> Vec<Voice> {
    let lines: Vec<&str> = table.lines().collect();
    let body = match lines.iter().position(|line| line.starts_with("---")) {
        Some(rule) => &lines[rule + 1..],
        None => match lines.first() {
            Some(first) if first.trim_start().starts_with("Name") => &lines[1..],
            _ => &lines[..],
        },
    };

    body.iter()
        .filter_map(|line| {
            let mut columns = line.split("  ").filter(|c| !c.trim().is_empty());
            let id = columns.next()?.trim().to_owned();
            // A name with a space in it is a wrapped line or a message, not a
            // voice — Microsoft's ids have never contained one.
            if id.is_empty() || id.contains(char::is_whitespace) {
                return None;
            }
            let locale = locale_of(&id);
            Some(Voice {
                id,
                locale,
                gender: columns.next().unwrap_or_default().trim().to_owned(),
                note: columns
                    .map(|c| c.trim())
                    .filter(|c| !c.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real output of `edge-tts --list-voices` on this machine, 2026-08-26,
    /// `edge-tts 7.2.8` — 322 voices, captured rather than typed. plan.md §M2
    /// asks for a recorded fixture from the first commit, and the reason is
    /// this file: a change in the tool's output is a test failure here instead
    /// of an empty voice list in the window with nothing wrong on screen.
    const CATALOGUE: &str = include_str!("../fixtures/edge-list-voices-7.2.8.txt");

    /// The first rows of that same capture, kept inline because most tests want
    /// four voices rather than three hundred.
    const RECORDED: &str = "\
Name                               Gender    ContentCategories      VoicePersonalities
---------------------------------  --------  ---------------------  --------------------------------------
af-ZA-AdriNeural                   Female    General                Friendly, Positive
af-ZA-WillemNeural                 Male      General                Friendly, Positive
en-US-AvaNeural                    Female    Conversation, Copilot  Expressive, Caring, Pleasant, Friendly
en-GB-RyanNeural                   Male      General                Friendly, Positive
";

    #[test]
    fn the_recorded_voice_table_parses_into_voices() {
        let voices = parse_voices(RECORDED);

        assert_eq!(voices.len(), 4);
        assert_eq!(voices[2].id, "en-US-AvaNeural");
        assert_eq!(voices[2].locale, "en-US");
        assert_eq!(voices[2].gender, "Female");
        assert!(voices[2].note.contains("Expressive"));
        assert!(
            voices.iter().any(|v| v.id == DEFAULT_VOICE),
            "the voice we default to must exist in the service's own list"
        );
    }

    /// Every row of the real catalogue, not a hand-picked four. A parser that
    /// drops one row in three hundred passes a four-row test and loses a voice
    /// an operator was using.
    #[test]
    fn every_voice_in_the_recorded_catalogue_parses() {
        let voices = parse_voices(CATALOGUE);
        let rows = CATALOGUE.lines().count() - 2; // header, rule

        assert_eq!(voices.len(), rows, "every row is a voice");
        assert!(
            voices
                .iter()
                .all(|v| !v.id.is_empty() && !v.gender.is_empty()),
            "no row loses its name or its gender"
        );
        assert!(
            voices.iter().all(|v| !v.locale.is_empty()),
            "every voice can be filed under a language: {:?}",
            voices
                .iter()
                .filter(|v| v.locale.is_empty())
                .map(|v| &v.id)
                .collect::<Vec<_>>()
        );
        assert!(
            voices.iter().all(|v| v.id.starts_with(&v.locale)),
            "a locale that is not the head of its own id is a parse, not a fact"
        );
    }

    /// The six voices in the catalogue whose ids are not `xx-YY-Name`. These
    /// are the whole reason `locale_of` exists; if a future catalogue drops
    /// them the rule still has to be right for the next odd shape.
    #[test]
    fn a_script_subtag_stays_and_a_dialect_does_not() {
        assert_eq!(locale_of("af-ZA-AdriNeural"), "af-ZA");
        assert_eq!(locale_of("en-US-AvaNeural"), "en-US");
        // Language, script, region: the script belongs to the locale.
        assert_eq!(locale_of("iu-Cans-CA-SiqiniqNeural"), "iu-Cans-CA");
        assert_eq!(locale_of("iu-Latn-CA-TaqqiqNeural"), "iu-Latn-CA");
        // Language, region, dialect: the dialect is not a BCP-47 subtag, so it
        // is not smuggled into one.
        assert_eq!(locale_of("zh-CN-liaoning-XiaobeiNeural"), "zh-CN");
        assert_eq!(locale_of("zh-CN-shaanxi-XiaoniNeural"), "zh-CN");
        // Nonsense keeps an empty locale rather than a wrong one.
        assert_eq!(locale_of("Nonsense"), "");
        assert_eq!(locale_of(""), "");
    }

    #[test]
    fn a_table_with_no_rows_is_empty_rather_than_a_row_of_dashes() {
        let header = RECORDED.lines().take(2).collect::<Vec<_>>().join("\n");
        assert!(parse_voices(&header).is_empty());
        assert!(parse_voices("").is_empty());
    }

    /// A future `edge-tts` that stops printing the rule must not silently
    /// produce zero voices — that is the failure that looks like nothing.
    #[test]
    fn a_table_without_its_rule_still_parses() {
        let ruleless: String = RECORDED
            .lines()
            .filter(|line| !line.starts_with("---"))
            .collect::<Vec<_>>()
            .join("\n");
        let voices = parse_voices(&ruleless);
        assert_eq!(voices.len(), 4, "{voices:?}");
        assert_eq!(voices[0].id, "af-ZA-AdriNeural");
    }

    #[test]
    fn knobs_default_to_no_change_and_normalize_their_sign() {
        assert_eq!(percent(None, "rate").expect("absent is fine"), "+0%");
        assert_eq!(percent(Some(""), "rate").expect("blank is absent"), "+0%");
        assert_eq!(percent(Some("10"), "rate").expect("bare number"), "+10%");
        assert_eq!(
            percent(Some("+10%"), "rate").expect("already signed"),
            "+10%"
        );
        assert_eq!(percent(Some("-25%"), "rate").expect("negative"), "-25%");
        assert_eq!(hertz(None).expect("absent is fine"), "+0Hz");
        assert_eq!(hertz(Some("-20Hz")).expect("negative"), "-20Hz");
        assert_eq!(hertz(Some("5")).expect("bare number"), "+5Hz");
    }

    #[test]
    fn a_knob_that_is_not_a_number_is_refused_before_the_process_starts() {
        let error = percent(Some("fast"), "rate").expect_err("not a percentage");
        assert!(error.to_string().contains("fast"), "{error}");
        assert!(
            error.to_string().contains("rate"),
            "the knob is named: {error}"
        );
        assert!(percent(Some("400%"), "rate").is_err(), "outside the range");
        assert!(hertz(Some("+900Hz")).is_err(), "outside the range");
    }

    #[test]
    fn an_empty_line_is_refused_without_spawning_anything() {
        // The program name is deliberately nonsense: reaching the process
        // boundary at all would fail this test rather than pass it slowly.
        let edge = Edge::at("/nonexistent/edge-tts");
        let error = edge
            .speak(
                &Request {
                    text: "   ",
                    voice: "en-US-AvaNeural",
                    settings: &[],
                },
                Path::new("/nonexistent/out.mp3"),
            )
            .expect_err("an empty line");
        assert!(matches!(error, TtsError::BadRequest { .. }), "{error}");
    }

    /// A bad knob must be caught before a file is created, not after — the
    /// temporary would be left behind on a path this function never returns to.
    #[test]
    fn a_bad_knob_is_refused_before_the_script_is_written() {
        let directory = std::env::temp_dir().join(format!(
            "spoonstill-edge-knob-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let settings = vec![("rate".to_owned(), "quickly".to_owned())];
        let error = Edge::at("/nonexistent/edge-tts")
            .speak(
                &Request {
                    text: "A line worth saying.",
                    voice: "en-US-AvaNeural",
                    settings: &settings,
                },
                &directory.join("out.mp3"),
            )
            .expect_err("a knob that is not a number");

        assert!(matches!(error, TtsError::BadRequest { .. }), "{error}");
        let left = std::fs::read_dir(&directory)
            .expect("readable")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect::<Vec<_>>();
        assert!(left.is_empty(), "nothing is left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_binary_reports_how_to_install_it() {
        let edge = Edge::at("/nonexistent/edge-tts");
        match edge.availability() {
            Availability::Missing(detail) => {
                assert!(detail.contains("pip install edge-tts"), "{detail}");
                assert!(detail.contains(EDGE_TTS_ENV), "{detail}");
            }
            Availability::Ready => panic!("a binary that is not there cannot be ready"),
        }
    }

    /// A missing binary in the middle of a render says the same sentence the
    /// pre-flight check does — and says it once, without three attempts and
    /// six seconds of pauses first.
    #[test]
    fn a_missing_binary_is_not_retried_and_names_the_fix() {
        let directory = std::env::temp_dir().join(format!(
            "spoonstill-edge-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let started = std::time::Instant::now();
        let error = Edge::at("/nonexistent/edge-tts")
            .speak(
                &Request {
                    text: "A line worth saying.",
                    voice: "en-US-AvaNeural",
                    settings: &[],
                },
                &directory.join("out.mp3"),
            )
            .expect_err("no binary");

        assert!(matches!(error, TtsError::Unavailable { .. }), "{error}");
        assert!(
            error.to_string().contains("pip install edge-tts"),
            "{error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a permanent failure must not sit through the backoff"
        );

        let left = std::fs::read_dir(&directory)
            .expect("readable")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect::<Vec<_>>();
        assert!(left.is_empty(), "the script is removed too: {left:?}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    // ---- classification -------------------------------------------------
    //
    // Every string below is real stderr from `edge-tts 7.2.8`, captured on
    // this machine on 2026-08-26. Described failures are the ones a classifier
    // gets wrong.

    /// A line of only punctuation. The service accepts the request, sends no
    /// audio, and the tool raises. Saying it again will do the same thing.
    const NO_AUDIO: &str = "\
Traceback (most recent call last):
  File \"/opt/homebrew/lib/python3.14/site-packages/edge_tts/communicate.py\", line 562, in __stream
    raise NoAudioReceived(
        \"No audio was received. Please verify that your parameters are correct.\"
    )
edge_tts.exceptions.NoAudioReceived: No audio was received. Please verify that your parameters are correct.
";

    /// A voice that does not exist. Caught by the tool before any network.
    const BAD_VOICE: &str = "\
  File \"/opt/homebrew/lib/python3.14/site-packages/edge_tts/data_classes.py\", line 40, in validate_string_param
    raise ValueError(f\"Invalid {param_name} '{param_value}'.\")
ValueError: Invalid voice 'not-a-voice'.
";

    /// The network is not there. Captured through an unreachable proxy, which
    /// is the same exception a dropped connection raises.
    const NO_NETWORK: &str = "\
  File \"/opt/homebrew/lib/python3.14/site-packages/aiohttp/connector.py\", line 1321, in _wrap_create_connection
    raise client_error(req.connection_key, exc) from exc
aiohttp.client_exceptions.ClientProxyConnectionError: Cannot connect to host 127.0.0.1:9 ssl:<ssl.SSLContext object at 0x10b01b790> [Connect call failed ('127.0.0.1', 9)]
";

    #[test]
    fn a_failure_the_service_will_repeat_is_not_retried() {
        assert_eq!(classify(NO_AUDIO), Fault::Permanent);
        assert_eq!(classify(BAD_VOICE), Fault::Permanent);
        assert_eq!(
            classify(""),
            Fault::Permanent,
            "an unrecognised failure is not retried three times at n=500"
        );
    }

    #[test]
    fn a_failure_the_network_caused_is_retried() {
        assert_eq!(classify(NO_NETWORK), Fault::Transient);
        assert_eq!(
            classify("edge_tts.exceptions.WebSocketError: Received 403"),
            Fault::Transient,
            "the token moved, or a proxy is having a moment"
        );
        assert_eq!(
            classify("edge_tts.exceptions.UnknownResponse: unexpected message"),
            Fault::Transient
        );
    }

    /// The trap this classifier is shaped around: a permanent failure whose
    /// traceback passes through the network stack on its way out. The
    /// outermost exception is the true one, and it is on the last line.
    #[test]
    fn a_permanent_failure_wrapped_in_network_frames_is_still_permanent() {
        let mixed = format!(
            "  File \"aiohttp/client.py\", line 1, in request\n{}",
            NO_AUDIO
        );
        assert_eq!(classify(&mixed), Fault::Permanent);
    }

    #[test]
    fn a_bad_voice_is_reported_as_a_bad_voice_and_not_as_a_traceback() {
        let error = spoonstill_media::MediaError::Exit {
            command: "edge-tts".to_owned(),
            code: Some(1),
            stderr: BAD_VOICE.to_owned(),
        };
        match interpret(error, "not-a-voice", "A line.") {
            Failure::Permanent(TtsError::BadRequest { detail, .. }) => {
                assert!(detail.contains("not-a-voice"), "{detail}");
                assert!(
                    detail.contains("still voices"),
                    "names the way out: {detail}"
                );
                assert!(!detail.contains("Traceback"), "{detail}");
            }
            Failure::Permanent(other) => panic!("wrong error: {other}"),
            Failure::Transient(reason) => panic!("a bad voice is not transient: {reason}"),
        }
    }

    #[test]
    fn a_line_with_nothing_speakable_in_it_names_the_line_and_the_cause() {
        let error = spoonstill_media::MediaError::Exit {
            command: "edge-tts".to_owned(),
            code: Some(1),
            stderr: NO_AUDIO.to_owned(),
        };
        match interpret(error, "en-US-AvaNeural", "...") {
            Failure::Permanent(TtsError::NoAudio { text, detail, .. }) => {
                assert_eq!(text, "...");
                assert!(detail.contains("punctuation"), "{detail}");
            }
            Failure::Permanent(other) => panic!("wrong error: {other}"),
            Failure::Transient(reason) => panic!("this one repeats: {reason}"),
        }
    }

    /// Our own timeout is always worth another go: a wedged socket and a very
    /// long line look identical from here, and the second attempt gets a fresh
    /// connection.
    #[test]
    fn a_timeout_is_transient() {
        let error = spoonstill_media::MediaError::Timeout {
            command: "edge-tts".to_owned(),
            waited: Duration::from_secs(90),
        };
        match interpret(error, "en-US-AvaNeural", "A line.") {
            Failure::Transient(reason) => assert!(reason.contains("90"), "{reason}"),
            Failure::Permanent(error) => panic!("a timeout is worth retrying: {error}"),
        }
    }

    // ---- long form (D-095) ----------------------------------------------

    /// The normal case, and the one that must not change: a line that fits is
    /// one request, unsplit, exactly as it was written.
    #[test]
    fn a_line_that_fits_is_never_split() {
        let line = "The harbour was empty by the time we arrived.";
        assert_eq!(split_line(line, CHUNK_CHARS), vec![line]);
        assert_eq!(split_line("  padded  ", CHUNK_CHARS), vec!["padded"]);
        assert!(split_line("   ", CHUNK_CHARS).is_empty());
    }

    /// A seam between two requests is a seam in the speech, so it goes where a
    /// reader would have paused: a paragraph break first, then a sentence end.
    #[test]
    fn a_long_line_is_cut_where_a_reader_would_pause() {
        let a = "First sentence here. Second sentence here. Third one here.";
        let pieces = split_line(a, 30);
        assert!(
            pieces.iter().all(|p| p.ends_with('.')),
            "every cut landed on a sentence end: {pieces:?}"
        );
        assert_eq!(pieces.concat().replace(' ', ""), a.replace(' ', ""));

        let paragraphs = "One two three.\n\nFour five six.";
        assert_eq!(
            split_line(paragraphs, 20),
            vec!["One two three.", "Four five six."],
            "a paragraph break is the first choice"
        );
    }

    /// No piece may exceed the limit, whatever the text is made of — that is
    /// the whole contract, and a sentence longer than a chunk is the case that
    /// breaks a naive splitter.
    #[test]
    fn no_piece_is_ever_longer_than_the_limit() {
        let unbroken = "word ".repeat(4_000);
        let no_spaces = "x".repeat(5_000);
        let mixed = format!("Short. {unbroken} Also short.\n\n{no_spaces}");

        for (name, text) in [
            ("unbroken", unbroken.as_str()),
            ("no spaces", no_spaces.as_str()),
            ("mixed", mixed.as_str()),
        ] {
            for limit in [50, 500, 9_000] {
                let pieces = split_line(text, limit);
                assert!(
                    pieces.iter().all(|p| p.chars().count() <= limit),
                    "{name} at limit {limit}: {:?}",
                    pieces
                        .iter()
                        .map(|p| p.chars().count())
                        .filter(|n| *n > limit)
                        .collect::<Vec<_>>()
                );
                assert!(
                    !pieces.is_empty(),
                    "{name} at limit {limit} produced nothing"
                );
            }
        }
    }

    /// Nothing may be dropped and nothing invented. Whitespace at the seams is
    /// the provider's to normalize; the words are not.
    #[test]
    fn splitting_loses_no_words() {
        let text = "Alpha bravo charlie. Delta echo foxtrot! Golf hotel india? Juliet.";
        for limit in [10, 25, 40, 1_000] {
            let pieces = split_line(text, limit);
            let rejoined: String = pieces
                .iter()
                .flat_map(|p| p.split_whitespace())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(
                rejoined,
                text.split_whitespace().collect::<Vec<_>>().join(" "),
                "limit {limit} lost or invented words: {pieces:?}"
            );
        }
    }

    /// An hour of narration — the most a scene can hold — becomes a handful of
    /// requests rather than one seven-minute bet.
    #[test]
    fn an_hour_of_narration_becomes_several_requests() {
        let hour = "The harbour was empty by the time we arrived. ".repeat(1_400);
        assert!(
            hour.chars().count() > 60_000,
            "{} characters",
            hour.chars().count()
        );

        let pieces = split_line(&hour, CHUNK_CHARS);
        assert!(
            (5..=12).contains(&pieces.len()),
            "an hour is a handful of requests, not {} of them",
            pieces.len()
        );
        for piece in &pieces {
            let ceiling = speak_timeout(piece.chars().count());
            assert!(
                ceiling < SPEAK_CEILING,
                "no piece may be big enough to need the hard ceiling: {ceiling:?}"
            );
        }
    }

    /// The bug this found: a script far longer than a scene can hold used to be
    /// spoken for eleven minutes, normalized by two FFmpeg passes, and *then*
    /// refused for its measured duration. It is refused before the first
    /// request now, and the refusal says what to do.
    #[test]
    fn a_line_no_scene_could_hold_is_refused_before_anything_is_spoken() {
        let far_too_long = "The harbour was empty by the time we arrived. ".repeat(3_000);
        assert!(far_too_long.chars().count() > max_line_chars());

        let started = std::time::Instant::now();
        let error = Edge::at("/nonexistent/edge-tts")
            .speak(
                &Request {
                    text: &far_too_long,
                    voice: DEFAULT_VOICE,
                    settings: &[],
                },
                Path::new("/nonexistent/out.mp3"),
            )
            .expect_err("longer than a scene");

        assert!(matches!(error, TtsError::BadRequest { .. }), "{error}");
        let said = error.to_string();
        assert!(said.contains("minutes of speech"), "{said}");
        assert!(said.contains("Split it"), "it says what to do: {said}");
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    /// The rate this is built on is the *fastest* observed, so the limit
    /// refuses only what cannot fit at any speaking rate. Measured on this
    /// machine: 17.3 chars/s for flowing prose, 11.9 for clipped sentences.
    /// Deriving the limit from the slow end would refuse lines that fit.
    #[test]
    fn the_limit_refuses_only_what_could_never_fit() {
        const SLOWEST_SEEN: f64 = 11.9;
        assert!(
            SPEECH_CHARS_PER_SECOND > SLOWEST_SEEN,
            "the limit must come from the fastest rate, or it refuses lines that would have fitted"
        );

        // A line just under the limit is refusable downstream if it happens to
        // be slow prose — and that is correct, because the duration that
        // governs is the measured one (D-021).
        let cap = spoonstill_core::project::MAX_SCENE_SECONDS;
        let at_the_limit = max_line_chars() as f64;
        assert!(
            at_the_limit / SPEECH_CHARS_PER_SECOND <= cap,
            "a line at the limit fits when read quickly"
        );
        assert!(
            at_the_limit / SLOWEST_SEEN > cap,
            "and may not when read slowly — which downstream catches, not us"
        );
    }

    /// The limit is derived from the scene cap, not typed in beside it. If
    /// D-021's hour ever changes, this moves with it.
    #[test]
    fn the_line_limit_tracks_the_scene_cap() {
        let expected =
            (spoonstill_core::project::MAX_SCENE_SECONDS * SPEECH_CHARS_PER_SECOND) as usize;
        assert_eq!(max_line_chars(), expected);
        assert!(
            max_line_chars() > CHUNK_CHARS * 5,
            "a scene's worth of narration must be several chunks"
        );
    }

    /// A long line that comes back empty is worth asking again; `...` is not.
    /// Corrected from the author's own `setuptts`, which retries no-audio at
    /// full size before re-splitting.
    #[test]
    fn no_audio_is_permanent_for_a_caption_and_transient_for_a_paragraph() {
        let error = || spoonstill_media::MediaError::Exit {
            command: "edge-tts".to_owned(),
            code: Some(1),
            stderr: NO_AUDIO.to_owned(),
        };

        assert!(
            matches!(
                interpret(error(), DEFAULT_VOICE, "..."),
                Failure::Permanent(_)
            ),
            "a caption with no words in it will never have any"
        );

        let paragraph = "The harbour was empty. ".repeat(500);
        match interpret(error(), DEFAULT_VOICE, &paragraph) {
            Failure::Transient(reason) => {
                assert!(
                    reason.contains(&paragraph.chars().count().to_string()),
                    "{reason}"
                );
            }
            Failure::Permanent(error) => {
                panic!("a long payload that returns nothing is worth one more go: {error}")
            }
        }
    }

    // ---- the retry loop itself ------------------------------------------

    #[test]
    fn a_transient_failure_is_tried_again_and_can_succeed() {
        let mut slept = Vec::new();
        let outcome = persevere(
            Retry::default(),
            |attempt| {
                if attempt < 3 {
                    Err(Failure::Transient(format!("attempt {attempt} dropped")))
                } else {
                    Ok("spoke")
                }
            },
            |pause| slept.push(pause),
        );

        assert_eq!(outcome.ok(), Some("spoke"));
        assert_eq!(
            slept,
            vec![Duration::from_millis(500), Duration::from_millis(1500)],
            "the pause grows, and there is none before the first attempt"
        );
    }

    #[test]
    fn a_permanent_failure_stops_at_the_first_attempt() {
        let mut attempts = 0;
        let mut slept = 0;
        let outcome: Result<(), _> = persevere(
            Retry::default(),
            |_| {
                attempts += 1;
                Err(Failure::Permanent(TtsError::BadRequest {
                    provider: ID.to_owned(),
                    detail: "no".to_owned(),
                }))
            },
            |_| slept += 1,
        );

        assert_eq!(attempts, 1);
        assert_eq!(slept, 0);
        assert!(matches!(outcome, Err((1, Failure::Permanent(_)))));
    }

    #[test]
    fn running_out_of_attempts_reports_the_last_thing_that_went_wrong() {
        let outcome: Result<(), _> = persevere(
            Retry::default(),
            |attempt| Err(Failure::Transient(format!("dropped on {attempt}"))),
            |_| {},
        );

        match outcome {
            Err((3, Failure::Transient(reason))) => assert_eq!(reason, "dropped on 3"),
            Err((n, _)) => panic!("three attempts, not {n}"),
            Ok(()) => panic!("nothing succeeded"),
        }
    }

    #[test]
    fn one_attempt_means_one_attempt() {
        let mut attempts = 0;
        let outcome: Result<(), _> = persevere(
            Retry::once(),
            |_| {
                attempts += 1;
                Err(Failure::Transient("dropped".to_owned()))
            },
            |_| panic!("nothing to wait for"),
        );
        assert_eq!(attempts, 1);
        assert!(outcome.is_err());
    }

    // ---- the ceiling on one line ----------------------------------------

    /// 5 980 characters took 37.6 s on this machine. The ceiling has to be
    /// above that with room for a bad connection, and the old fixed 90 s was
    /// not: it refused a paragraph twice this long on any link slower than the
    /// one it was measured on.
    #[test]
    fn a_long_line_gets_longer_than_a_short_one() {
        let short = speak_timeout(20);
        let measured = speak_timeout(5_980);

        assert!(short >= Duration::from_secs(60), "{short:?}");
        assert!(
            measured > Duration::from_secs(37 * 4),
            "37.6s measured, and a slow link is several times that: {measured:?}"
        );
        assert!(measured > short);
        assert_eq!(
            speak_timeout(usize::MAX),
            SPEAK_CEILING,
            "no line waits forever"
        );
    }

    // ---- what came back --------------------------------------------------

    #[test]
    fn the_first_bytes_of_a_real_mp3_are_recognised_as_audio() {
        // The opening of a file this provider actually produced, 2026-08-26.
        assert!(looks_like_audio(&[
            0xFF, 0xF3, 0x64, 0xC4, 0x00, 0x00, 0x00, 0x03
        ]));
        assert!(looks_like_audio(b"ID3\x04\x00\x00\x00"));
        assert!(looks_like_audio(b"RIFF\x24\x08\x00"));
        assert!(looks_like_audio(b"OggS\x00\x02\x00"));
    }

    #[test]
    fn an_error_page_written_to_the_media_file_is_not_audio() {
        assert!(!looks_like_audio(b"<!DOCTYPE html>"));
        assert!(!looks_like_audio(b"{\"error\":\"429\"}"));
        assert!(!looks_like_audio(b"Traceback (most"));
        assert!(!looks_like_audio(b""));
    }
}
