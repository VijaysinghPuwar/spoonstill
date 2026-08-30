//! Turning an operator-supplied path into a real path inside the project — or
//! refusing it without telling the caller what is on the disk (D-052, D-054).
//!
//! Every path in a manifest is untrusted input (D-052). A row can say
//! `../../../etc/passwd`, or name a symlink that leaves the project, or use a
//! spelling that only matches on a case-insensitive volume. One function
//! decides what any of that means, and it is this one.
//!
//! Two rules, and everything below follows from them:
//!
//! - **Containment is decided on canonical paths, component-wise.** Not on the
//!   string the operator typed, and not with a string prefix — `/proj-evil`
//!   has the string prefix `/proj` and must not resolve inside it.
//! - **Out of bounds is one answer, whether or not the file exists** (D-054).
//!   Otherwise the difference between two error messages is a probe of the
//!   host filesystem: ask for `/etc/passwd`, ask for `/etc/nope`, compare.
//!
//! ## No I/O lives here
//!
//! `spoonstill-core` models the *result* of I/O and puts the doing behind a
//! trait (D-010). Canonicalization is the one thing only the filesystem knows,
//! so it arrives through [`RealPath`]. Everything else — the walk, the lexical
//! finish, the containment decision — is pure, which is why the tests at the
//! bottom of this file run identically on macOS and Windows with no fixtures.
//!
//! ## Prior art
//!
//! Shape borrowed from `plan/MoneyPrinterTurbo/app/utils/file_security.py`
//! `resolve_path_within_directory()` and its caller
//! `app/services/task.py:359 resolve_custom_audio_file()`, as plan.md §M2
//! asks. Labelled:
//!
//! - **adopt** — realpath first, then containment, rather than a string test.
//!   Their comment is right that this is what covers symlinks, duplicate
//!   separators and relative paths in one step.
//! - **adopt** — the generic out-of-bounds error, precise in-bounds error.
//!   Their `resolve_custom_audio_file` is explicit about why.
//! - **modify** — `os.path.commonpath` becomes [`Path::starts_with`], which is
//!   already component-wise. Their `ValueError` for two Windows drive letters
//!   becomes a plain containment failure; different volumes are simply not
//!   inside each other.
//! - **modify** — they realpath a path that need not exist. Rust's
//!   `canonicalize` requires existence, so a missing file is resolved through
//!   [`deepest_real_ancestor`] instead. That turns out to matter for more than
//!   ergonomics; see its documentation.
//! - **reject** — `allow_server_file_input`, the flag that lets a trusted
//!   caller resolve outside the boundary. We have no second trust level to
//!   spend it on, and D-050's project folder is the boundary. If an operator
//!   ever needs a shared media library outside the project, that is a new
//!   decision with an explicit root list, not a boolean.

use core::fmt;
use std::path::{Component, Path, PathBuf};

/// The one filesystem fact this module cannot derive: what a path really is.
///
/// Implemented over `std::fs::canonicalize` by the layer that is allowed to
/// touch a disk, and over a fixed table by the tests below.
pub trait RealPath {
    /// Canonical form of an **existing** path: symlinks followed, `.` and `..`
    /// resolved, case as the volume actually stores it.
    ///
    /// `None` when the path does not exist or cannot be resolved. Callers must
    /// not distinguish those two — see [`PathError::Outside`].
    fn real_path(&self, path: &Path) -> Option<PathBuf>;
}

/// Why a path was refused.
///
/// Deliberately small and payload-free. The caller knows what it asked for and
/// which scene asked, and attaches both when it records the problem — that is
/// how the scene ID reaches the operator (D-052) without this type growing a
/// dependency on the project model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathError {
    /// The manifest cell was blank.
    Empty,
    /// The path resolves outside the project root.
    ///
    /// **This is also the answer for a path that does not exist outside the
    /// project root** (D-054). The two cases are indistinguishable on purpose:
    /// a caller that could tell them apart could enumerate the host
    /// filesystem one question at a time.
    Outside,
    /// The path is inside the project, and there is nothing there.
    ///
    /// Only ever produced for in-bounds paths, which is what makes it safe to
    /// be specific: it describes the operator's own folder back to them.
    Missing,
    /// The project root itself could not be resolved — it does not exist, or
    /// it is not readable. Nothing can be contained in it, so no path from
    /// this manifest can be judged.
    RootUnresolvable,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => f.write_str("path is empty"),
            PathError::Outside => f.write_str("path is outside the project folder"),
            PathError::Missing => f.write_str("no such file inside the project folder"),
            PathError::RootUnresolvable => {
                f.write_str("project folder does not exist or cannot be read")
            }
        }
    }
}

/// Resolve `requested` against `root`, or refuse it.
///
/// A relative path is joined onto the *canonical* root; an absolute path is
/// taken as given and then held to the same containment rule, so an absolute
/// path that happens to point inside the project is fine and one that does not
/// is [`PathError::Outside`].
///
/// On success the returned path is canonical and inside `root`. That is the
/// only path any later stage should use — the operator's spelling is a
/// request, not an address.
///
/// # Errors
///
/// See [`PathError`]. Note that a missing file is an error, so nothing that
/// does not exist is ever handed back for a caller to create or read through.
pub fn resolve_within(
    root: &Path,
    requested: &Path,
    fs: &dyn RealPath,
) -> Result<PathBuf, PathError> {
    match resolve_contained(root, requested, fs)? {
        (path, true) => Ok(path),
        (_, false) => Err(PathError::Missing),
    }
}

/// Where a file this program is about to **write** would land, held to exactly
/// the same containment rule (D-112).
///
/// [`resolve_within`] answers for an input, so it treats a path that does not
/// exist as an error. A destination does not exist yet — that is the normal
/// case — and the caller needs the resolved path in order to create it. The
/// containment decision is identical and is made by the same code; only the
/// verdict on absence differs.
///
/// The returned path is canonical as far as anything real exists, so a caller
/// writes to the place containment was actually decided about rather than to
/// the operator's spelling of it.
///
/// # Errors
///
/// [`PathError::Outside`] when the destination escapes the root — including
/// through a symlink, which is the case a lexical check on `..` cannot see.
/// Never [`PathError::Missing`]: absence is the expected state here.
pub fn resolve_destination_within(
    root: &Path,
    requested: &Path,
    fs: &dyn RealPath,
) -> Result<PathBuf, PathError> {
    resolve_contained(root, requested, fs).map(|(path, _)| path)
}

/// The containment decision both public functions rest on.
///
/// Returns the resolved path and whether it exists. Split out so that reading
/// a file and writing one cannot drift into two different ideas of what
/// "inside the project" means — the rule is one function, and the only thing
/// the callers disagree about is whether absence is a failure.
fn resolve_contained(
    root: &Path,
    requested: &Path,
    fs: &dyn RealPath,
) -> Result<(PathBuf, bool), PathError> {
    if requested.as_os_str().is_empty() {
        return Err(PathError::Empty);
    }

    let base = fs.real_path(root).ok_or(PathError::RootUnresolvable)?;

    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        base.join(requested)
    };

    // The path exists: the filesystem has already done the hard part. Every
    // symlink in it is followed, so this single check cannot be walked around.
    if let Some(real) = fs.real_path(&candidate) {
        return if real.starts_with(&base) {
            Ok((real, true))
        } else {
            Err(PathError::Outside)
        };
    }

    // The path does not exist, and `canonicalize` will not resolve it. Finish
    // the job by hand — but anchored on real ground, never lexically from the
    // top. Why that distinction is load-bearing: see `deepest_real_ancestor`.
    let Some((anchor, rest)) = deepest_real_ancestor(&candidate, fs) else {
        return Err(PathError::Outside);
    };
    if !anchor.starts_with(&base) {
        return Err(PathError::Outside);
    }
    let Some(resolved) = lexical_join(&anchor, &rest) else {
        return Err(PathError::Outside);
    };
    if resolved.starts_with(&base) {
        Ok((resolved, false))
    } else {
        Err(PathError::Outside)
    }
}

/// Longest leading run of `path` that exists, canonicalized, plus the
/// components left over.
///
/// This exists to close an information leak, not for tidiness. Suppose the
/// project contains `link -> /etc`. Then:
///
/// ```text
/// project/link/passwd   exists     -> canonicalize -> /etc/passwd -> Outside
/// project/link/nope     missing    -> ???
/// ```
///
/// Finish the missing case lexically and it stays "inside" `project/`, so it
/// answers `Missing` while its neighbour answers `Outside` — and the
/// difference between those two words is a working existence oracle for every
/// file on the host. Anchoring on the deepest *real* ancestor resolves `link`
/// to `/etc` first, so both answer `Outside`, which is D-054's rule.
///
/// Returns `None` only when no ancestor at all resolves, including the root of
/// the volume.
fn deepest_real_ancestor<'a>(
    path: &'a Path,
    fs: &dyn RealPath,
) -> Option<(PathBuf, Vec<Component<'a>>)> {
    let components: Vec<Component<'_>> = path.components().collect();

    for split in (1..=components.len()).rev() {
        let prefix: PathBuf = components[..split].iter().collect();
        if let Some(real) = fs.real_path(&prefix) {
            return Some((real, components[split..].to_vec()));
        }
    }
    None
}

/// Apply `rest` to `anchor` textually: `.` is dropped, `..` pops.
///
/// Safe precisely because every component in `rest` is known not to exist —
/// [`deepest_real_ancestor`] stopped where the filesystem stopped — so none of
/// them can be a symlink with an opinion about where it leads.
///
/// `None` when the walk cannot be completed: popping past the volume root, or
/// an absolute component in the tail, which a well-formed split cannot produce
/// and which would silently re-root the result if it were honoured.
fn lexical_join(anchor: &Path, rest: &[Component<'_>]) -> Option<PathBuf> {
    let mut out = anchor.to_path_buf();
    for component in rest {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(name) => out.push(name),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A filesystem that is a table: `path -> canonical path`. Anything absent
    /// does not exist. Symlinks are just entries whose value points elsewhere.
    ///
    /// Paths are written in POSIX form and rewritten per-platform on lookup,
    /// so these tests assert the same behaviour on Windows as on macOS —
    /// which is the point of keeping the logic pure (D-071).
    struct FakeFs {
        entries: BTreeMap<PathBuf, PathBuf>,
    }

    impl FakeFs {
        fn new(pairs: &[(&str, &str)]) -> Self {
            FakeFs {
                entries: pairs
                    .iter()
                    .map(|(from, to)| (native(from), native(to)))
                    .collect(),
            }
        }

        /// The shape every test below starts from: a project folder, one real
        /// image inside it, a secret outside it, and a symlink out.
        fn project() -> Self {
            FakeFs::new(&[
                ("/work", "/work"),
                ("/work/proj", "/work/proj"),
                ("/work/proj/img", "/work/proj/img"),
                ("/work/proj/img/001.jpg", "/work/proj/img/001.jpg"),
                ("/work/proj/link", "/etc"),
                ("/work/proj-evil", "/work/proj-evil"),
                ("/work/proj-evil/001.jpg", "/work/proj-evil/001.jpg"),
                ("/etc", "/etc"),
                ("/etc/passwd", "/etc/passwd"),
                ("/", "/"),
            ])
        }
    }

    impl RealPath for FakeFs {
        fn real_path(&self, path: &Path) -> Option<PathBuf> {
            self.entries.get(path).cloned()
        }
    }

    /// POSIX spelling -> whatever this platform calls it.
    fn native(posix: &str) -> PathBuf {
        if cfg!(windows) {
            let stripped = posix.strip_prefix('/').unwrap_or(posix);
            PathBuf::from(format!(r"C:\{}", stripped.replace('/', r"\")))
        } else {
            PathBuf::from(posix)
        }
    }

    fn resolve(requested: &str) -> Result<PathBuf, PathError> {
        let fs = FakeFs::project();
        resolve_within(&native("/work/proj"), Path::new(requested), &fs)
    }

    #[test]
    fn a_relative_path_inside_the_project_resolves() {
        assert_eq!(resolve("img/001.jpg"), Ok(native("/work/proj/img/001.jpg")));
    }

    #[test]
    fn an_absolute_path_inside_the_project_resolves() {
        let inside = native("/work/proj/img/001.jpg");
        assert_eq!(
            resolve(inside.to_str().expect("test path is UTF-8")),
            Ok(inside)
        );
    }

    #[test]
    fn a_blank_cell_is_not_the_project_root() {
        assert_eq!(resolve(""), Err(PathError::Empty));
    }

    #[test]
    fn path_safety_rejects_dotdot_traversal() {
        // Exists, and is outside. The classic.
        assert_eq!(resolve("../proj-evil/001.jpg"), Err(PathError::Outside));
        assert_eq!(
            resolve("img/../../proj-evil/001.jpg"),
            Err(PathError::Outside)
        );
        assert_eq!(resolve("../../etc/passwd"), Err(PathError::Outside));
    }

    #[test]
    fn path_safety_rejects_an_absolute_escape() {
        assert_eq!(
            resolve(native("/etc/passwd").to_str().expect("UTF-8")),
            Err(PathError::Outside)
        );
    }

    /// D-054, and the reason this module has a fake filesystem at all: the
    /// answer for something outside the project must not depend on whether it
    /// is there. Two files, one real and one not, one error.
    #[test]
    fn path_safety_leaks_no_existence_outside_the_project() {
        let real = resolve("../../etc/passwd");
        let absent = resolve("../../etc/definitely-not-here");

        assert_eq!(real, Err(PathError::Outside));
        assert_eq!(absent, Err(PathError::Outside));
        assert_eq!(real, absent, "D-054: out of bounds is one answer");
        assert_eq!(
            real.unwrap_err().to_string(),
            absent.unwrap_err().to_string(),
            "the rendered messages must match too — an operator reads those"
        );
    }

    /// The same test through a symlink, which is the case a lexical
    /// containment check gets wrong. `link -> /etc`.
    #[test]
    fn path_safety_leaks_no_existence_through_a_symlink() {
        let real = resolve("link/passwd");
        let absent = resolve("link/definitely-not-here");

        assert_eq!(real, Err(PathError::Outside));
        assert_eq!(
            absent,
            Err(PathError::Outside),
            "a missing file under an escaping symlink must not answer Missing — \
             that difference is an existence oracle for the whole host"
        );
    }

    /// A missing file the operator genuinely meant deserves the specific
    /// error; that is the half of the reference implementation's rule that is
    /// easy to forget.
    #[test]
    fn a_missing_file_inside_the_project_is_reported_precisely() {
        assert_eq!(resolve("img/002.jpg"), Err(PathError::Missing));
        assert_eq!(resolve("no-such-dir/002.jpg"), Err(PathError::Missing));
        assert_eq!(resolve("img/./sub/../002.jpg"), Err(PathError::Missing));
    }

    /// Containment is component-wise. A string prefix test passes this and is
    /// wrong: `/work/proj-evil` starts with the characters of `/work/proj`.
    #[test]
    fn a_sibling_with_a_shared_prefix_is_outside() {
        assert_eq!(resolve("../proj-evil/001.jpg"), Err(PathError::Outside));

        let sibling = native("/work/proj-evil/001.jpg");
        assert_eq!(
            resolve(sibling.to_str().expect("UTF-8")),
            Err(PathError::Outside)
        );
    }

    #[test]
    fn a_project_root_that_is_not_there_judges_nothing() {
        let fs = FakeFs::project();
        assert_eq!(
            resolve_within(&native("/work/gone"), Path::new("img/001.jpg"), &fs),
            Err(PathError::RootUnresolvable)
        );
    }

    /// The project folder itself is in bounds. Callers still have to decide
    /// whether a directory is an acceptable answer — that is a media question,
    /// not a safety one.
    #[test]
    fn the_root_is_contained_by_itself() {
        assert_eq!(resolve("."), Ok(native("/work/proj")));
    }

    #[test]
    fn popping_past_the_volume_root_is_refused_rather_than_clamped() {
        // `/` exists in the fake fs, so the walk anchors there and then pops.
        assert_eq!(
            resolve("../../../../../../nope.jpg"),
            Err(PathError::Outside)
        );
    }
}
