//! The two joints between the webview and Rust, asserted rather than clicked.
//!
//! D-088 shipped a title bar that looked draggable and was not, and closed with
//! the rule: *anything in the window a test cannot assert has to be clicked
//! before it is called done.* This file is the other half of that bargain —
//! it makes two things assertable that used to need a click, and both of them
//! fail **silently** in a webview, which is what makes them worth a test:
//!
//! - `el("gone").addEventListener(…)` throws `TypeError` on `null`. The
//!   listener is at the foot of `app.js`, so the throw takes every listener
//!   after it with it, and the window opens looking perfectly normal with half
//!   its buttons inert. There is no console anybody is watching.
//! - `invoke("renamed_command")` rejects a promise. `guard` turns that into one
//!   line of status text, so a command removed in Rust reads to an operator as
//!   a feature that stopped working for no reason.
//!
//! D-105 made both of these live risks in one change: it deleted the
//! `app-provider-install` and `app-provider-recheck` buttons in favour of the
//! shared fix component, and renamed `install_provider` to `install_tool`.
//! Every one of those is a dangling reference if the other file is not edited
//! in the same commit.
//!
//! A source scan is a blunt instrument and it is the right one here, for the
//! same reason `no_shell_strings.rs` gives: what it prevents is somebody
//! *removing* an element six months from now, which no test of today's
//! behaviour can catch.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

fn read(name: &str) -> String {
    let path = ui_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Every `"…"` that follows `prefix` in `text`, unescaped enough for ids and
/// command names, which are both plain ASCII by construction.
fn quoted_after(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        rest = &rest[at + prefix.len()..];
        let Some(end) = rest.find('"') else { break };
        let value = &rest[..end];
        if !value.is_empty() && !value.contains('\n') {
            found.insert(value.to_owned());
        }
        rest = &rest[end..];
    }
    found
}

/// Strip `//` line comments, so prose naming an id does not count as using it.
///
/// Borrowed wholesale from `no_shell_strings.rs`, including the reason: this
/// file's own header names several ids that no longer exist.
fn code_only(text: &str) -> String {
    text.lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip `/* … */` blocks, so a comment explaining a rule is not read as one.
///
/// The CSS counterpart of [`code_only`], and it earned its place immediately:
/// the comment above the platform rule names the negated selector it exists to
/// warn against, and without this the test that forbids that selector fails on
/// the prose forbidding it.
fn css_code_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = &rest[at + 2..];
        match rest.find("*/") {
            Some(end) => rest = &rest[end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every `el("literal")` in `text`, ignoring ids built by concatenation.
///
/// `el("pane-" + name)` names a whole family of elements and says nothing
/// checkable about any one of them, so it is skipped rather than reported as
/// an element called `pane-`. The literal lookups are the ones a rename
/// breaks, and they are the overwhelming majority.
fn literal_el_ids(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find("el(\"") {
        rest = &rest[at + 4..];
        let Some(end) = rest.find('"') else { break };
        let (value, after) = (&rest[..end], &rest[end + 1..]);
        if after.starts_with(')') && !value.is_empty() {
            found.insert(value.to_owned());
        }
        rest = after;
    }
    found
}

/// Every element the frontend reaches for is really in the document.
#[test]
fn every_id_the_frontend_asks_for_exists_in_the_markup() {
    let html = read("index.html");
    let present = quoted_after(&html, "id=\"");
    let asked = literal_el_ids(&code_only(&read("app.js")));

    assert!(
        !asked.is_empty(),
        "the scan found no el(\"…\") calls at all"
    );

    let missing: Vec<&String> = asked.difference(&present).collect();
    assert!(
        missing.is_empty(),
        "app.js reaches for elements index.html does not have: {missing:?}\n\
         `el()` answers null, and a listener attached to null throws — taking \
         every listener declared after it with it. The window then opens \
         looking correct with half its controls dead, and says nothing."
    );
}

/// Every command the frontend calls is really registered.
#[test]
fn every_command_the_frontend_calls_is_registered_in_rust() {
    // The dialog plugin is invoked by name because this shell has no bundler,
    // and it is registered by Tauri rather than by us (see the note in
    // app.js). Anything else namespaced the same way is somebody else's too.
    let called: BTreeSet<String> = quoted_after(&code_only(&read("app.js")), "invoke(\"")
        .into_iter()
        .filter(|name| !name.starts_with("plugin:"))
        .collect();

    let main = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("readable main.rs");
    let (_, handlers) = main
        .split_once("generate_handler![")
        .expect("main.rs registers its commands with generate_handler!");
    let (handlers, _) = handlers.split_once(']').expect("a closed handler list");
    let registered: BTreeSet<String> = handlers
        .split(',')
        .map(|entry| {
            entry
                .lines()
                .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|entry| !entry.is_empty())
        .collect();

    assert!(
        !called.is_empty(),
        "the scan found no invoke(\"…\") calls at all"
    );

    let missing: Vec<&String> = called.difference(&registered).collect();
    assert!(
        missing.is_empty(),
        "app.js invokes commands that are not in generate_handler!: {missing:?}\n\
         registered: {registered:?}\n\
         An unregistered command rejects its promise, `guard` turns that into \
         one line of status text, and the operator reads it as a feature that \
         stopped working."
    );
}

/// The fix component is on every screen where a tool can be missing (D-105).
///
/// Not decoration: the Voice screen shipped with the *report* and without the
/// button, and the button was one level up under Settings. That is the defect
/// this test exists to keep out — a future screen that reports a missing tool
/// and offers no way to end it.
#[test]
fn every_screen_that_reports_a_missing_tool_can_also_fix_it() {
    let html = read("index.html");
    for host in [
        "voice-fix",        // the Voice screen — the screen from the report
        "render-fix",       // before a render, not after it fails
        "app-provider-fix", // Settings: the voice service
        "app-ffmpeg-fix",   // Settings: FFmpeg, which every render needs
    ] {
        assert!(
            html.contains(&format!("id=\"{host}\"")),
            "{host} is gone — a screen that can report a missing tool has lost \
             the button that ends it"
        );
    }

    let js = code_only(&read("app.js"));
    assert!(
        js.contains("function drawFix("),
        "the one component that draws a remedy is gone"
    );
    assert!(
        js.contains("invoke(\"install_tool\""),
        "nothing installs anything any more"
    );
}

/// D-127. Every command that writes into the operator's folder still tells
/// Rust which project it means.
///
/// Those commands are bound to the project the window has open, and the value
/// the page sends is *compared* against it rather than ignored — so a page that
/// stopped sending `root` would have every edit refused. In a webview that
/// failure is silent, which is the whole reason `ui_contract.rs` exists.
#[test]
fn every_command_that_writes_names_the_project_it_means() {
    let js = code_only(&read("app.js"));

    for command in [
        "set_narration",
        "add_media",
        "remove_scene",
        "move_scene",
        "preview_voice",
    ] {
        let at = js
            .find(&format!("\"{command}\""))
            .unwrap_or_else(|| panic!("{command} is not invoked from the page at all"));

        // The arguments follow the name closely — they are one object literal,
        // written inline or handed to the `arrange` helper.
        let window = &js[at..js.len().min(at + 200)];
        assert!(
            window.contains("root:"),
            "{command} is called without a root, so Rust cannot tell which \
             project it means and will refuse it: {window}"
        );
    }
}

/// D-143. The Output screen's shape and size boxes *send* what they say.
///
/// This is D-106's own lesson, written down as a test: the subtitle position
/// box drove only the preview for a whole release, so an operator moved the
/// caption off their artwork, rendered, and it came back exactly where it was.
/// A control that changes what is on screen and not what is rendered is the
/// same defect as a wrong number, and a webview reports it as nothing at all.
#[test]
fn the_shape_and_size_boxes_reach_the_render() {
    let js = code_only(&read("app.js"));

    let at = js
        .find("\"render_project\"")
        .expect("the page does not render at all");
    let request = &js[at..js.len().min(at + 1600)];
    for field in ["aspect:", "resolution:"] {
        assert!(
            request.contains(field),
            "the render request carries no {field} — the Output screen's \
             chooser would change the preview and nothing else"
        );
    }

    // And both boxes are wired to something, rather than being decoration.
    for id in ["out-aspect", "out-size"] {
        assert!(
            js.contains(&format!("el(\"{id}\").addEventListener")),
            "{id} has no listener, so choosing in it does nothing"
        );
    }

    // The thumbnails follow the shape being rendered. This used to be sniffed
    // out of the geometry *string* by testing for a `1080x1920` prefix — one
    // size of one aspect — so a 4K Short showed landscape thumbnails.
    assert!(
        !js.contains("1080x1920"),
        "the grid is deciding its shape by matching one size's pixel string"
    );
}

/// The traffic-light reservation is macOS's, and it says so.
///
/// `titleBarStyle: "Overlay"` is macOS-only — Tauri's `title_bar_style` is
/// behind `#[cfg(target_os = "macos")]`, so Windows ignores it silently and
/// keeps its native title bar. The 82px at the left of `.titlebar` is room for
/// traffic lights that only exist under overlay; unguarded, it is a gap under
/// a Windows title bar with nothing in it.
///
/// Asserted rather than clicked for D-088's reason twice over: there is no
/// Windows machine here, and a webview accepts a wrong layout without a word.
/// The negated form of the CSS rule is the trap this pins shut — `data-os` does
/// not exist until `app.js` runs, so `:not([data-os="macos"])` would match in
/// that gap and shove the mark under the traffic lights on every macOS launch.
#[test]
fn the_traffic_light_padding_is_asked_for_by_platform() {
    let css = css_code_only(&read("styles.css"));
    let js = code_only(&read("app.js"));

    assert!(
        js.contains("dataset.os"),
        "app.js no longer tells the stylesheet which platform it is on, so the \
         macOS traffic-light padding is applied on Windows too"
    );

    assert!(
        css.contains(r#"[data-os="windows"]"#),
        "styles.css no longer gives the 82px traffic-light reservation back on \
         Windows, where there are no traffic lights to reserve it for"
    );

    // The default has to remain macOS: the attribute is absent until the module
    // loads, and a rule keyed on its absence fires in that window.
    assert!(
        !css.contains(r#":not([data-os="macos"])"#),
        "the platform rule is negated, so it matches before app.js sets the \
         attribute and macOS flashes its title bar under the traffic lights"
    );
}
