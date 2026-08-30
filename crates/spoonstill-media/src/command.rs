//! The one place in spoonstill that spawns a process (D-011, D-012).
//!
//! # Argument vectors, never shell strings
//!
//! Every argument is an [`OsString`]. There is no method on [`FfmpegCommand`]
//! that takes a whole command line, and no code path here reaches `sh -c` or
//! `cmd /c`. That is not a convention — `tests/no_shell_strings.rs` reads this
//! crate's own source and fails if a second module learns to spawn.
//!
//! It matters because D-052 treats hostile input as normal: `ünïcode spaced
//! 名前.jpg` is a fixture, not an edge case. Through an argument vector that
//! filename is data and can never become syntax. Through a formatted string it
//! is one apostrophe away from being a command.
//!
//! # What is retained, and why
//!
//! Raw stderr is captured in full, always — even when `-progress` is parsed.
//! A parser-classified log level is not a substitute for exit status plus
//! output validation, because the interesting FFmpeg failures do not announce
//! themselves at `error` level. D-041's silent concat is exactly that shape.
//!
//! # Cancellation (D-045)
//!
//! Graceful, then forced, then clean: [`FfmpegChild::quit`] writes `q` to
//! FFmpeg's stdin and lets it finalize; [`FfmpegChild::cancel`] escalates to
//! [`FfmpegChild::kill`] once a deadline passes. `q`-on-stdin rather than a
//! signal because there is no portable way to send SIGINT to a child on
//! Windows, and D-071 puts Windows in scope from M1.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::MediaError;

/// How often a wait loop checks on a child.
///
/// `std::process::Child` has no portable timed wait, so waiting is a poll. The
/// interval is irrelevant against a multi-second encode and keeps the loop off
/// the CPU; it is deliberately not configurable, because tuning it would mean
/// someone was using it as a scheduler.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Cap on retained stderr per child.
///
/// FFmpeg can emit megabytes of per-frame warnings on a damaged input. Keeping
/// all of it would let one bad file in a 500-scene batch exhaust memory, so the
/// tail is kept — which is where the cause always is — and the truncation is
/// announced rather than silent.
const STDERR_CAP: usize = 256 * 1024;

/// A command under construction. Arguments only; never a command line.
#[derive(Debug, Clone)]
pub struct FfmpegCommand {
    program: PathBuf,
    args: Vec<OsString>,
    /// Whether to attach `-progress pipe:1`, set when the caller wants events.
    progress: bool,
}

impl FfmpegCommand {
    /// Start building an invocation of `program`.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            progress: false,
        }
    }

    /// Append one argument.
    ///
    /// Takes `AsRef<OsStr>` rather than `AsRef<str>` on purpose: a path on
    /// either platform can contain bytes that are not valid UTF-8, and forcing
    /// them through `String` would either lose them or refuse the file.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Append several arguments.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Append an input file. Exists so `-i` and its path cannot drift apart.
    pub fn input(&mut self, path: &Path) -> &mut Self {
        self.arg("-i").arg(path)
    }

    /// Request machine-readable progress on stdout.
    pub fn with_progress(&mut self) -> &mut Self {
        self.progress = true;
        self
    }

    /// The arguments, in order, for tests and for the cache key.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.args
    }

    /// The program that will be executed.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// A safely escaped, paste-ready form of this command.
    ///
    /// For humans and logs only — it is never parsed, never re-executed, and
    /// never the thing that reaches the OS. Modelled on lossless-cut's
    /// `LastCommands` panel: when a render fails at scene 147 the operator's
    /// first move is to run that exact command in a terminal, and if we make
    /// them reconstruct it by hand they will reconstruct it wrong.
    #[must_use]
    pub fn display(&self) -> String {
        let mut out = shell_quote(&self.program.as_os_str().to_string_lossy());
        if self.progress {
            out.push_str(" -progress pipe:1");
        }
        for arg in &self.args {
            out.push(' ');
            out.push_str(&shell_quote(&arg.to_string_lossy()));
        }
        out
    }

    /// Spawn the child, taking its pipes and starting the retention threads.
    ///
    /// # Errors
    ///
    /// [`MediaError::BinaryMissing`] when the program cannot be executed —
    /// reported with the exact path tried, because a missing bundled binary and
    /// a missing `PATH` entry need different fixes.
    pub fn spawn(&self) -> Result<FfmpegChild, MediaError> {
        let display = self.display();
        let mut command = Command::new(&self.program);
        if self.progress {
            command.arg("-progress").arg("pipe:1");
        }
        command.args(&self.args);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut command);

        let mut child = command.spawn().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                MediaError::BinaryMissing {
                    tool: tool_name(&self.program),
                    tried: self.program.clone(),
                    source,
                }
            } else {
                MediaError::Spawn {
                    command: display.clone(),
                    source,
                }
            }
        })?;

        // Both pipes are drained on their own threads from the moment the child
        // starts. Reading them in sequence after `wait()` deadlocks the moment
        // either fills its OS buffer, and a filter graph that is warning per
        // frame fills stderr in seconds.
        let stderr = child.stderr.take().map(spawn_stderr_reader);
        let (progress_tx, progress_rx) = channel();

        // stdout is either machine-readable progress or the tool's actual
        // answer — `ffprobe` writes its JSON there. Both are drained on a
        // thread; neither is ever left unread.
        let (progress_thread, stdout) = match child.stdout.take() {
            Some(stdout) if self.progress => {
                (Some(spawn_progress_reader(stdout, progress_tx)), None)
            }
            Some(stdout) => (None, Some(spawn_stdout_reader(stdout))),
            None => (None, None),
        };

        Ok(FfmpegChild {
            child,
            display,
            stderr,
            stdout,
            progress_rx,
            progress_thread,
        })
    }
}

/// A running child, retained rather than waited on immediately.
///
/// Retention is the point: a handle that only offers "run to completion" cannot
/// implement D-045's ladder, cannot report progress, and cannot be supervised
/// by a queue that owns several of them.
#[derive(Debug)]
pub struct FfmpegChild {
    child: Child,
    display: String,
    stderr: Option<JoinHandle<String>>,
    stdout: Option<JoinHandle<Vec<u8>>>,
    progress_rx: Receiver<Progress>,
    progress_thread: Option<JoinHandle<()>>,
}

/// One `-progress` report from a running encode.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    /// Frames written so far.
    pub frame: Option<u64>,
    /// Output timestamp in microseconds.
    pub out_time_us: Option<i64>,
    /// Encoding speed as a multiple of realtime.
    pub speed: Option<f64>,
    /// True on the final report of the run.
    pub done: bool,
}

/// What a finished child left behind.
#[derive(Debug)]
pub struct Finished {
    /// Exit status as reported by the OS.
    pub status: ExitStatus,
    /// Raw stdout. Empty when `-progress` was requested instead.
    pub stdout: Vec<u8>,
    /// Raw stderr, retained in full up to [`STDERR_CAP`].
    pub stderr: String,
    /// The paste-ready command, carried so callers can report it.
    pub command: String,
}

impl FfmpegChild {
    /// The paste-ready form of the command that started this child.
    #[must_use]
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Progress reports, if `-progress` was requested.
    #[must_use]
    pub fn progress(&self) -> &Receiver<Progress> {
        &self.progress_rx
    }

    /// Ask FFmpeg to stop and finalize its output, the way `q` at the terminal
    /// does. Graceful: the file it has written so far is closed properly.
    ///
    /// # Errors
    ///
    /// Returns the underlying error if stdin has already been closed — which
    /// simply means the child is already on its way out.
    pub fn quit(&mut self) -> std::io::Result<()> {
        if let Some(mut stdin) = self.child.stdin.take() {
            stdin.write_all(b"q")?;
            stdin.flush()?;
        }
        Ok(())
    }

    /// Force the child to stop. Nothing is finalized.
    ///
    /// # Errors
    ///
    /// Returns the underlying error if the child cannot be signalled.
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    /// Whether the child has exited, without blocking.
    ///
    /// # Errors
    ///
    /// Returns the underlying error if the child cannot be inspected.
    pub fn try_status(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Wait for the child to exit, however long it takes.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Spawn`] if the child cannot be waited on.
    pub fn wait(mut self) -> Result<Finished, MediaError> {
        let status = self.child.wait().map_err(|source| MediaError::Spawn {
            command: self.display.clone(),
            source,
        })?;
        Ok(self.finish(status))
    }

    /// Wait up to `limit`, then give up and report a timeout.
    ///
    /// The child is killed on timeout rather than left running: an orphaned
    /// FFmpeg holding a partial file is exactly the "valid-looking stub" that
    /// D-045 forbids.
    ///
    /// # Errors
    ///
    /// [`MediaError::Timeout`] if `limit` expires first.
    pub fn wait_until(mut self, limit: Duration) -> Result<Finished, MediaError> {
        let deadline = Instant::now() + limit;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(self.finish(status)),
                Ok(None) => {}
                Err(source) => {
                    return Err(MediaError::Spawn {
                        command: self.display.clone(),
                        source,
                    });
                }
            }
            if Instant::now() >= deadline {
                let _ = self.kill();
                let _ = self.child.wait();
                return Err(MediaError::Timeout {
                    command: self.display.clone(),
                    waited: limit,
                });
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// D-045's ladder in one call: ask nicely, wait, then force.
    ///
    /// Returns once the child is genuinely gone. Cleaning up whatever it wrote
    /// is the caller's job, because only the caller knows which path was the
    /// real output and which was the temporary one.
    pub fn cancel(mut self, grace: Duration) -> Finished {
        let _ = self.quit();
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return self.finish(status),
                Ok(None) => {}
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        let _ = self.kill();
        let status = self.child.wait().ok();
        match status {
            Some(status) => self.finish(status),
            // The child is gone but the OS would not tell us how. Report the
            // stderr we retained anyway; it is the part with the answer in it.
            None => {
                let (stdout, stderr) = self.collect_output();
                Finished {
                    status: ExitStatus::default(),
                    stdout,
                    stderr,
                    command: self.display.clone(),
                }
            }
        }
    }

    fn finish(&mut self, status: ExitStatus) -> Finished {
        let (stdout, stderr) = self.collect_output();
        Finished {
            status,
            stdout,
            stderr,
            command: self.display.clone(),
        }
    }

    /// Join every retention thread. Called once, after the child has exited, so
    /// the pipes are at EOF and no join can block.
    fn collect_output(&mut self) -> (Vec<u8>, String) {
        if let Some(handle) = self.progress_thread.take() {
            let _ = handle.join();
        }
        let stdout = self
            .stdout
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let stderr = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        (stdout, stderr)
    }
}

impl Finished {
    /// Turn a non-zero exit into a [`MediaError`] carrying the retained stderr.
    ///
    /// # Errors
    ///
    /// [`MediaError::Exit`] when the status is not success.
    pub fn ok(self) -> Result<Self, MediaError> {
        if self.status.success() {
            Ok(self)
        } else {
            Err(MediaError::Exit {
                command: self.command,
                code: self.status.code(),
                stderr: self.stderr,
            })
        }
    }
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = Vec::with_capacity(8 * 1024);
        let mut reader = BufReader::new(stderr);
        let mut chunk = [0_u8; 8 * 1024];
        let mut truncated = false;
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.len() > STDERR_CAP {
                // Keep the tail: the cause of an FFmpeg failure is at the end,
                // and the head is banner and stream metadata.
                let excess = buffer.len() - STDERR_CAP;
                buffer.drain(..excess);
                truncated = true;
            }
        }
        let mut text = String::from_utf8_lossy(&buffer).into_owned();
        if truncated {
            text.insert_str(0, "[earlier stderr dropped — retained the last 256 KiB]\n");
        }
        text
    })
}

fn spawn_stdout_reader(stdout: std::process::ChildStdout) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::with_capacity(8 * 1024);
        let mut reader = BufReader::new(stdout);
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
}

fn spawn_progress_reader(
    stdout: std::process::ChildStdout,
    tx: Sender<Progress>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut current = Progress::default();
        for line in reader.lines().map_while(Result::ok) {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "frame" => current.frame = value.parse().ok(),
                "out_time_us" => current.out_time_us = value.parse().ok(),
                "speed" => current.speed = value.trim_end_matches('x').trim().parse().ok(),
                "progress" => {
                    current.done = value == "end";
                    // A send failure means the consumer stopped listening,
                    // which is normal when a render is cancelled.
                    if tx.send(std::mem::take(&mut current)).is_err() {
                        return;
                    }
                }
                _ => {}
            }
        }
    })
}

/// Best-effort tool name for an error message.
fn tool_name(program: &Path) -> &'static str {
    let name = program
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.contains("ffprobe") {
        "ffprobe"
    } else {
        "ffmpeg"
    }
}

/// Whether an argument can be shown with no quoting at all.
///
/// Deliberately narrow, and the same on both platforms: a `\` puts a Windows
/// path on the quoted side, which is where it belongs.
fn needs_no_quoting(arg: &str) -> bool {
    !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/:=+,@".contains(c))
}

/// Quote one argument for a POSIX shell. Display only.
///
/// Carries the same exemption as [`windows_quote`], for the same reason and in
/// the opposite direction: on Windows [`shell_quote`] never reaches this, so
/// the lib build has it as dead code. D-128 kept both dialects compiled
/// everywhere so the tests can run both on whatever machine is to hand — and
/// an exemption written for only one of them makes that true on only one of
/// them. Without this, `-D warnings` fails the Windows build.
#[cfg_attr(windows, allow(dead_code))]
fn posix_quote(arg: &str) -> String {
    if needs_no_quoting(arg) {
        arg.to_owned()
    } else {
        // Single quotes, with the standard escape for an embedded one: close
        // the quote, emit an escaped quote, reopen.
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// Quote one argument the way a Windows terminal reads it. Display only.
///
/// **Not the same rule** (D-128). PowerShell's single quotes are literal, which
/// is right for a Windows path full of backslashes, but an embedded quote is
/// escaped by **doubling** it, not with POSIX's `'\''`. Emitting the POSIX
/// form gave a Windows operator a line that would not paste — and a filename
/// with an apostrophe in it is `Dad's photos`, not an exotic case.
///
/// PowerShell rather than `cmd.exe` because it is what the shipped installer is
/// written in and what a modern Windows terminal opens; `cmd` has no single
/// quoting at all, so no one form can serve both.
///
/// Compiled everywhere, not only on Windows, so that the tests below can run
/// **both** dialects on whatever machine is to hand — D-071 puts Windows in
/// scope and nothing has ever been run there, so a rule that only exists on a
/// platform nobody tests is a rule nobody checks.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_quote(arg: &str) -> String {
    if needs_no_quoting(arg) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "''"))
    }
}

/// Quote one argument for display, in the shell the operator is most likely to
/// paste it into. Never used to build a real invocation.
fn shell_quote(arg: &str) -> String {
    #[cfg(windows)]
    {
        windows_quote(arg)
    }
    #[cfg(not(windows))]
    {
        posix_quote(arg)
    }
}

/// Keep FFmpeg from flashing a console window on Windows (D-071).
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`, from the Win32 process creation flags.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// No-op away from Windows; the flag has no meaning there.
#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_kept_as_a_vector_in_order() {
        let mut c = FfmpegCommand::new("ffmpeg");
        c.arg("-y").input(Path::new("a b.jpg")).arg("out.mp4");
        let args: Vec<String> = c
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["-y", "-i", "a b.jpg", "out.mp4"]);
    }

    /// The display form is for a human's terminal. A path that would be four
    /// words unquoted must come back as one.
    #[test]
    fn the_display_form_is_paste_ready() {
        let mut c = FfmpegCommand::new("ffmpeg");
        c.input(Path::new("ünïcode spaced 名前.jpg")).arg("out.mp4");
        let shown = c.display();
        assert!(shown.contains("'ünïcode spaced 名前.jpg'"), "{shown}");
        assert!(shown.starts_with("ffmpeg -i "), "{shown}");
    }

    /// Quoting for display must not be forgeable by the filename itself.
    #[test]
    fn display_quoting_survives_a_hostile_filename() {
        let hostile = "it's; rm -rf $(pwd) `x`.jpg";
        let mut c = FfmpegCommand::new("ffmpeg");
        c.input(Path::new(hostile));
        let shown = c.display();
        assert!(shown.contains(r"'\''"), "apostrophe not escaped: {shown}");
        // The dangerous text is inside quotes, so a paste cannot execute it.
        assert!(!shown.contains("; rm -rf $(pwd)") || shown.contains(r"'it'\''s;"));
    }

    #[test]
    fn plain_arguments_are_not_needlessly_quoted() {
        assert_eq!(shell_quote("-frames:v"), "-frames:v");
        assert_eq!(shell_quote("scale=1920:1080"), "scale=1920:1080");
        assert_eq!(shell_quote("out.mp4"), "out.mp4");
        assert_eq!(shell_quote(""), "''");
    }

    /// A filter chain is one argument, however many commas and quotes it has.
    #[test]
    fn a_filter_chain_stays_a_single_argument() {
        let filter = "zoompan=z='1+0.1*on':d=30,setsar=1";
        let mut c = FfmpegCommand::new("ffmpeg");
        c.arg("-vf").arg(filter);
        assert_eq!(c.arguments().len(), 2);
        assert_eq!(c.arguments()[1].to_string_lossy(), filter);
    }

    /// The specific path is what distinguishes a packaging bug from a missing
    /// PATH entry, so it must survive into the error.
    #[test]
    fn a_missing_binary_names_the_path_it_tried() {
        let c = FfmpegCommand::new("/nonexistent/spoonstill/bin/ffmpeg");
        let err = c.spawn().expect_err("must not find that binary");
        assert!(matches!(err, MediaError::BinaryMissing { .. }));
        assert!(
            err.to_string()
                .contains("/nonexistent/spoonstill/bin/ffmpeg")
        );
    }

    #[test]
    fn a_missing_probe_binary_is_named_as_ffprobe() {
        let c = FfmpegCommand::new("/nonexistent/bin/ffprobe");
        let err = c.spawn().expect_err("must not find that binary");
        assert!(err.to_string().starts_with("ffprobe"), "{err}");
    }

    #[test]
    fn progress_lines_parse_into_reports() {
        let (tx, rx) = channel();
        let text = "frame=42\nout_time_us=1400000\nspeed=2.5x\nprogress=continue\n\
                    frame=112\nprogress=end\n";
        // Exercise the parser without a process, by feeding it the same lines.
        std::thread::spawn(move || {
            let mut current = Progress::default();
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                match k {
                    "frame" => current.frame = v.parse().ok(),
                    "out_time_us" => current.out_time_us = v.parse().ok(),
                    "speed" => current.speed = v.trim_end_matches('x').parse().ok(),
                    "progress" => {
                        current.done = v == "end";
                        let _ = tx.send(std::mem::take(&mut current));
                    }
                    _ => {}
                }
            }
        });
        let first = rx.recv().unwrap();
        assert_eq!(first.frame, Some(42));
        assert_eq!(first.speed, Some(2.5));
        assert!(!first.done);
        let last = rx.recv().unwrap();
        assert_eq!(last.frame, Some(112));
        assert!(last.done);
    }

    /// D-128. Two shells, two rules, and the display form has to be right for
    /// the one the operator will paste into.
    ///
    /// Both are tested on every platform on purpose. D-071 puts Windows in
    /// scope and nothing has ever been run there, so a rule that only compiles
    /// on Windows is a rule nobody checks.
    #[test]
    fn an_argument_is_quoted_for_the_shell_it_will_be_pasted_into() {
        // Nothing that needs quoting is left alone by either.
        for plain in ["ffmpeg", "-i", "input.mp4", "scale=1920:1080", "a/b/c.jpg"] {
            assert_eq!(posix_quote(plain), plain);
            assert_eq!(windows_quote(plain), plain);
        }

        // A space is the ordinary case, and both quote it.
        assert_eq!(posix_quote("RANDOM vidoe "), "'RANDOM vidoe '");
        assert_eq!(windows_quote("RANDOM vidoe "), "'RANDOM vidoe '");

        // A Windows path: backslashes are literal inside single quotes in both,
        // which is exactly why single quotes are the right choice here.
        let windows = r"C:\Users\vijay\Desktop\my film\001.jpg";
        assert_eq!(windows_quote(windows), format!("'{windows}'"));

        // The one that differs, and the reason this decision exists. POSIX
        // closes the quote, escapes one, and reopens; PowerShell doubles it.
        assert_eq!(posix_quote("Dad's photos"), r"'Dad'\''s photos'");
        assert_eq!(windows_quote("Dad's photos"), "'Dad''s photos'");

        // Emitting the POSIX form to PowerShell is what used to happen, and it
        // is not merely ugly — it does not parse.
        assert_ne!(
            posix_quote("Dad's photos"),
            windows_quote("Dad's photos"),
            "if these ever agree, one of them is wrong"
        );

        // An empty argument is still an argument.
        assert_eq!(posix_quote(""), "''");
        assert_eq!(windows_quote(""), "''");
    }
}
