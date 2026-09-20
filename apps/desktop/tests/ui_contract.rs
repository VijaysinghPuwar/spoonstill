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
        "app-settings-fix", // Settings: this machine's own settings file (D-171)
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

/// D-171. A settings file that is there and cannot be read is drawn, not
/// swallowed.
///
/// The page used to take `app_settings` as the settings themselves. It returns
/// `{settings, problem}` now, so a page that kept the old shape would read
/// `undefined.default_voice`, throw, and land in the `catch` — which sets the
/// fallback to `null` and is exactly the silence D-171 exists to end. Both
/// halves are asserted because either alone passes against that.
#[test]
fn a_damaged_settings_file_is_drawn_where_its_setting_is() {
    let js = code_only(&read("app.js"));
    assert!(
        js.contains("view.settings.default_voice"),
        "the page reads `app_settings` as the settings themselves again, so \
         the problem beside them cannot reach it"
    );
    assert!(
        js.contains("drawFix(el(\"app-settings-fix\")"),
        "a damaged settings file is read and then thrown away"
    );

    // And Rust still hands both halves over.
    let rust = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("src/main.rs");
    assert!(
        rust.contains("struct AppSettingsView"),
        "the command stopped carrying the problem"
    );
    assert!(
        rust.contains("machine::read()"),
        "`app_settings` went back to `load`, which cannot say anything is wrong"
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

/// D-156. A project is opened in one place, and making one goes through it.
///
/// `load` is what tells Rust which project this window is on (D-127), what puts
/// it on Home (D-086), and what fills in every field the grid reads. "New
/// project" used to build a `project` object out of the returned path instead,
/// so the window looked open on a folder Rust had never been told about and the
/// first drop onto it was refused — silently, in the status line, on a screen
/// whose only job is to receive that drop.
#[test]
fn making_a_project_opens_it_the_same_way_opening_one_does() {
    let js = code_only(&read("app.js"));

    let at = js
        .find("\"create_project\"")
        .expect("the page cannot make a project at all");
    // Close, deliberately: `load` is the next statement. A wider window reaches
    // the definition of `load` itself, which is a few lines below and would
    // make this pass against the very code it was written against.
    let after = &js[at..js.len().min(at + 200)];

    assert!(
        after.contains("await load("),
        "create_project's answer is not handed to load(), so Rust is never told \
         which project this window has open and every write into it — the drop \
         that follows most of all — is refused: {after}"
    );
    assert!(
        !after.contains("project = {"),
        "the page is assembling a project view of its own again: {after}"
    );
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

/// The voice on the screen is the voice in the render request (D-166).
///
/// These were two expressions of one rule, in two files. The Render summary
/// read `effectiveVoice()`; the request built
/// `chosenVoice || (projectNamesNoVoice() ? appDefaultVoice : null)`. Nothing
/// asserted they agreed, and one of them was reading a variable that the page
/// only filled if the operator had visited Settings in that session — so the
/// machine's fallback voice was saved, displayed, and ignored.
///
/// Asserted rather than clicked for D-088's reason: a webview renders a wrong
/// voice name without complaint, and a render in the wrong voice looks like a
/// finished render.
#[test]
fn the_voice_shown_is_the_voice_sent() {
    let js = code_only(&read("app.js"));

    assert!(
        js.contains("invoke(\"voice_choice\""),
        "the page is deciding the voice itself instead of asking the one \
         function that knows the rule"
    );

    let at = js
        .find("\"render_project\"")
        .expect("the page does not render at all");
    let request = &js[at..js.len().min(at + 1600)];
    assert!(
        request.contains("voice: voiceState?.overrideForRender"),
        "the render request is building its own answer, so what it sends can \
         drift from what the Voice screen shows"
    );

    // The page may still hold the fallback for the Settings <select> to show.
    // What it may not do is resolve *with* it: that is the copy of the rule
    // that silently did nothing on a launch where Settings was never opened.
    for spelling in ["appDefaultVoice ||", "projectNamesNoVoice"] {
        assert!(
            !js.contains(spelling),
            "`{spelling}` is the page resolving a voice again — the fallback \
             is read in Rust, from the file, where the page cannot forget it"
        );
    }
}

/// Every origin Rust can return has a word the page can print (D-166).
///
/// The two files are joined by a serde name, which no compiler checks. Adding
/// a fifth answer in Rust and forgetting the page gives a blank column on the
/// one screen that exists to say whose voice you will hear — the failure this
/// decision started from, one variant along.
///
/// It reads across crates since D-168 moved the rule into `spoonstill-app` to
/// share it with the command line, which makes the seam wider rather than
/// narrower: the enum is now edited by people not looking at this window.
#[test]
fn the_page_has_a_word_for_every_origin_rust_can_return() {
    let rust = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/spoonstill-app/src/voice.rs"),
    )
    .expect("reading the shared voice module");

    let at = rust
        .find("enum VoiceOrigin {")
        .expect("VoiceOrigin is gone");
    let body = &rust[at..at + rust[at..].find('}').expect("unclosed enum")];
    let variants: Vec<String> = body
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code).trim())
        .filter(|line| line.ends_with(','))
        .map(|line| line.trim_end_matches(',').to_lowercase())
        .collect();

    assert_eq!(
        variants.len(),
        4,
        "expected the four answers of D-092/D-166, found {variants:?}"
    );

    let js = code_only(&read("app.js"));
    let at = js.find("const VOICE_MARK = {").expect("VOICE_MARK is gone");
    let marks = &js[at..at + js[at..].find("};").expect("unclosed VOICE_MARK")];

    for variant in &variants {
        assert!(
            marks.contains(&format!("{variant}:")),
            "VoiceOrigin::{variant} has no word in VOICE_MARK, so that row \
             would be marked with nothing: {marks}"
        );
    }
}

/// A render nobody chose a voice for is stopped, once, and told why (D-167).
///
/// `renderBlocker` is the only thing that decides whether Render can run and
/// the only thing that says why (D-089), so the check belongs in it and
/// nowhere else — a second `disabled = true` somewhere would grey the button
/// out with no sentence beside it, which is the defect D-089 was written for.
///
/// Two halves, and the second is the one that keeps this from being a nuisance:
/// a project with nothing to speak must not be asked for a voice, because the
/// voice would change no frame of it.
#[test]
fn a_render_with_no_voice_chosen_is_stopped_and_told_why() {
    let js = code_only(&read("app.js"));

    let at = js
        .find("function renderBlocker()")
        .expect("nothing decides whether a render can start");
    let body = &js[at..js.len().min(at + 1400)];

    assert!(
        body.contains("\"unchosen\""),
        "a render can start with nobody having chosen a voice — the reported \
         defect, in the one function that could stop it"
    );
    assert!(
        body.contains("count(\"tts\")"),
        "the block is not scoped to projects that speak, so a film of supplied \
         recordings would be held up over a voice that changes no frame of it"
    );
    assert!(
        body.contains("Settings"),
        "the block does not say how to stop being asked, which is the whole \
         difference between asking once and asking every time"
    );

    // And choosing one releases it. Without this the operator does exactly what
    // the sentence tells them to and the button stays grey.
    let at = js
        .find("async function chooseVoice(")
        .expect("choosing a voice is gone");
    let choose = &js[at..js.len().min(at + 900)];
    assert!(
        choose.contains("updateRender()"),
        "choosing a voice does not re-ask whether Render can run, so the \
         button stays disabled after the operator has fixed it"
    );
}

/// The machine's fallback is settable from where voices are chosen (D-168).
///
/// It existed under Settings, one level up and behind Home — a long way from
/// the screen on which somebody has just found the voice they want for all ten
/// parts of their film, and far enough that D-166 found the setting had never
/// once been used. A control is only as good as the distance to it.
///
/// It is a toggle on purpose: the same button that sets it clears it. A
/// setting an operator cannot find their way back out of is worse than no
/// setting, and clearing this one can put a project back into the state
/// D-167 holds Render on — so it has to re-ask, which is the last assertion.
#[test]
fn the_fallback_voice_can_be_set_from_the_voice_screen() {
    let js = code_only(&read("app.js"));

    let at = js
        .find("async function pinVoice()")
        .expect("there is no way to set the fallback from the Voice screen");
    let pin = &js[at..js.len().min(at + 1200)];

    assert!(
        pin.contains("invoke(\"set_default_voice\""),
        "the pin does not reach the setting it exists to change"
    );
    assert!(
        pin.contains("isFallback") || code_only(&read("app.js")).contains("isFallback"),
        "nothing reads whether this voice is already the machine's, so the \
         control cannot tell setting from clearing"
    );
    assert!(
        pin.contains("voice: pinned ? null : voice"),
        "the pin is one-way — an operator who sets a fallback by accident has \
         to go and find Settings to undo it"
    );
    assert!(
        pin.contains("updateRender()"),
        "clearing the fallback can put the project back into \"nobody chose\", \
         which Render is held on (D-167), and nothing re-asks"
    );

    // And it is wired, which in a webview fails silently (D-105's lesson).
    assert!(
        js.contains("el(\"pin-voice\").addEventListener"),
        "the pin has no listener, so it is a button that does nothing"
    );
}

/// The terminal can do it too (D-168).
///
/// *If the CLI cannot do it, it does not exist* is this project's rule, and
/// the fallback voice broke it for a whole milestone: `AppSettings` was written
/// under Tauri's own config directory, which `still` has never been able to
/// read. Asserted here rather than in the CLI's own tests because this is the
/// file that knows what the window offers.
#[test]
fn everything_the_voice_screen_can_set_the_terminal_can_set() {
    let cli = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/spoonstill-cli/src/main.rs"),
    )
    .expect("reading the CLI");

    for flag in ["long = \"use\"", "forget"] {
        assert!(
            cli.contains(flag),
            "`still voices` cannot {flag} — the window can set a fallback \
             voice the command line cannot"
        );
    }
    assert!(
        cli.contains("machine::load().default_voice"),
        "`still render` never reads the machine's fallback, so setting one \
         changes the window's renders and not the terminal's"
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

/// D-160. The window names itself on Windows, where the native title bar is
/// real and the config's empty title left it blank.
///
/// Both halves are asserted, because each alone is the bug: the config must
/// stay empty (a title there prints over the page's own header on macOS, since
/// `TitleBarStyle::Overlay` does not hide the title text), and `main.rs` must
/// set one on Windows (or the taskbar and Alt-Tab entry have no name).
#[test]
fn the_window_names_itself_on_windows() {
    let config = read("../tauri.conf.json");
    assert!(
        config.contains(r#""title": """#),
        "tauri.conf.json now sets a window title for both platforms — on macOS \
         `titleBarStyle: Overlay` leaves the title text visible, so it would be \
         drawn over the header the page draws itself (D-160)"
    );

    let main = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
    )
    .expect("src/main.rs");

    assert!(
        main.contains("set_title(WINDOW_TITLE)"),
        "nothing sets the window title on Windows, so the native title bar is \
         blank and the taskbar entry has no name (D-160)"
    );

    // The constant and its guard are checked as two neighbouring lines rather
    // than one string, because this tree checks out with CRLF on Windows and a
    // multi-line literal would then match nothing here.
    let guarded = main
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            pair[0] == r#"#[cfg(target_os = "windows")]"#
                && pair[1].starts_with("const WINDOW_TITLE")
        });
    assert!(
        guarded,
        "WINDOW_TITLE is no longer Windows-only, so macOS is no longer provably \
         unchanged by D-160"
    );
}

/// D-148. Every window command that can fail has to say so somewhere an
/// operator's report can be checked against.
///
/// This file held **no `record` call at all**, so `runs.csv` covered renders
/// started from a terminal and nothing anybody did in the app — and the failure
/// the author actually reported (D-141) left no trace in the one file D-093
/// says to open. A source scan is the right instrument for the same reason the
/// two tests above give: what it prevents is a *new* command being added six
/// months from now without one, which no test of today's behaviour can catch.
///
/// The exemptions are listed by name, each with a reason, so adding one is a
/// decision rather than an omission.
#[test]
fn every_fallible_window_command_is_written_down() {
    let main = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("readable main.rs");
    let code = code_only(&main);

    // Pure, local, and answerable from the arguments alone. A failure in one
    // of these is a bug in the window rather than something about this
    // machine, and a row saying so would be noise in a file an operator reads.
    let exempt = [
        // Joins two strings the operator typed and checks the result (D-089).
        "resolve_output",
        // Draws a caption on a canvas in memory (D-106).
        "subtitle_preview",
        // Reads this machine's own settings file, already reported on screen.
        "activity_log",
        // Sets a flag on a running render (D-045); the render logs the rest.
        "cancel_render",
    ];

    // Every `#[tauri::command]` whose signature returns a `Result`.
    let mut unlogged = Vec::new();
    for block in code.split("#[tauri::command]").skip(1) {
        let Some(signature_end) = block.find('{') else {
            continue;
        };
        let signature = &block[..signature_end];
        if !signature.contains("-> Result<") {
            continue;
        }
        let Some(name) = signature
            .split("fn ")
            .nth(1)
            .and_then(|rest| rest.split('(').next())
            .map(str::trim)
        else {
            continue;
        };
        if exempt.contains(&name) {
            continue;
        }
        // The body of the command itself, which is the wrapper: it must hand
        // its outcome to `journalled`.
        let body_end = block[signature_end..]
            .find("\n}\n")
            .map_or(block.len(), |at| signature_end + at);
        if !block[signature_end..body_end].contains("journalled(") {
            unlogged.push(name.to_owned());
        }
    }

    assert!(
        unlogged.is_empty(),
        "these window commands can fail and write nothing down: {unlogged:?} — \
         wrap the outcome in `journalled`, or add the name to `exempt` with a reason"
    );
}

/// D-182. Saving one narration does not ask for the whole project back.
///
/// The save used to end `await load(project.root)`, which re-read all 500
/// scenes and probed every file — 1.32 s at n=500, on top of the 1.32 s the
/// command itself had just spent on a `load` whose entire result was one
/// `matches!`. The command answers with the row it changed; the page assigns
/// that row and redraws.
///
/// **Both halves are asserted**, because either alone passes against the
/// defect: keeping the reload and *also* assigning the row would be slower
/// than before, and assigning nothing while dropping the reload would leave
/// the operator's own sentence off the screen they just typed it on.
///
/// The `load` that survives is the fallback for `null`, which is the state
/// where Rust could not find the scene it had just written to. It is
/// deliberately not forbidden — what is forbidden is reloading on the
/// ordinary path.
#[test]
fn saving_one_narration_does_not_reload_the_whole_project() {
    let js = code_only(&read("app.js"));

    let at = js
        .find("\"set_narration\"")
        .expect("the page cannot save a narration at all");
    // To the end of the `try`, which is where the old reload sat.
    let after = &js[at..js.len().min(at + 700)];

    assert!(
        after.contains("Object.assign(scene,"),
        "the saved row is not applied, so the grid still shows what was there \
         before the operator typed: {after}"
    );
    assert!(
        after.contains("drawRows()"),
        "nothing redraws after the row changes: {after}"
    );

    // The one `load` allowed here is the `else` arm. Anything else is the
    // reload this decision removed.
    let reloads = after.match_indices("load(project.root)").count();
    assert_eq!(
        reloads, 1,
        "the save path reloads the whole project {reloads} time(s); exactly one \
         is allowed, and only as the fallback for a row Rust could not find: \
         {after}"
    );
    let fallback = after
        .split("Object.assign(scene,")
        .nth(1)
        .expect("checked above");
    assert!(
        fallback.contains("} else {"),
        "the surviving reload is not the fallback arm: {after}"
    );
}

// --- D-183: the palette is checked, not eyeballed --------------------------

/// One `oklch(L C H)` token, as the stylesheet writes it.
fn oklch(css: &str, token: &str) -> (f64, f64, f64) {
    let at = css
        .find(&format!("--{token}: oklch("))
        .unwrap_or_else(|| panic!("--{token} is not defined in styles.css"));
    let open = css[at..].find('(').expect("checked above") + at + 1;
    let close = css[open..].find(')').expect("an unclosed oklch()") + open;
    let parts: Vec<f64> = css[open..close]
        .split_whitespace()
        .map(|n| {
            n.parse()
                .unwrap_or_else(|_| panic!("--{token} has a non-numeric component: {n:?}"))
        })
        .collect();
    assert_eq!(parts.len(), 3, "--{token} is not `oklch(L C H)`");
    (parts[0], parts[1], parts[2])
}

/// Oklch to sRGB, then WCAG relative luminance, then the contrast ratio.
///
/// Spelled out here rather than taken from a crate: it is thirty lines, the
/// window has no build step and no dependencies of its own, and a second
/// implementation of a number the design was chosen against is the point
/// (D-172's golden vector, same reasoning). Reproduced against the audit's
/// independently-sampled figures and agreeing within 0.02.
fn contrast(fg: (f64, f64, f64), bg: (f64, f64, f64)) -> f64 {
    fn srgb((l, c, h): (f64, f64, f64)) -> [f64; 3] {
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let (l_, m_, s_) = (
            l + 0.3963377774 * a + 0.2158037573 * b,
            l - 0.1055613458 * a - 0.0638541728 * b,
            l - 0.0894841775 * a - 1.2914855480 * b,
        );
        let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
        let linear = [
            4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
            -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
            -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
        ];
        linear.map(|x| {
            let x = x.clamp(0.0, 1.0);
            if x <= 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            }
        })
    }
    fn luminance(rgb: [f64; 3]) -> f64 {
        let lin = rgb.map(|c| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]
    }
    let (a, b) = (luminance(srgb(fg)), luminance(srgb(bg)));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The two theme blocks, each as its own snippet, so `--3` in one is never
/// read as `--3` in the other.
fn themes(css: &str) -> [(&'static str, String); 2] {
    let dark_at = css.find(":root {").expect("the dark block");
    let light_at = css
        .find("html[data-theme=\"light\"] {")
        .expect("the light block");
    let end = css[light_at..].find('}').expect("an unclosed block") + light_at;
    [
        ("dark", css[dark_at..light_at].to_owned()),
        ("light", css[light_at..end].to_owned()),
    ]
}

/// D-183. Every level of ink is readable on every surface it can land on.
///
/// `--ink-3` shipped at **3.46–3.99:1** — under the AA minimum on all four
/// base surfaces, and worse on a hovered row, which is the state every row an
/// operator is reading happens to be in. It is used 42 times.
///
/// The surfaces are listed rather than derived, because "which background can
/// this text sit on" is a fact about the markup and not about the stylesheet.
/// Each is here because a rule really does put `--ink-3` on it:
/// `--hover` through `.grid tbody tr:hover td` and `.projects li:hover`,
/// `--eb` through `.grid tr.problem td`, `--in` through the fields,
/// `--c` through the cards.
///
/// **`--accent-soft` is deliberately absent**, and that is the interesting
/// one. No value of `--3` clears 4.5:1 there without collapsing the gap to
/// `--ink-2`, so a selected row steps its secondary text up a level instead —
/// which `.chip.on .n` already did, on the same background, before any of
/// this. That rule is asserted separately below.
#[test]
fn every_level_of_ink_is_readable_on_every_surface_it_lands_on() {
    let css = read("styles.css");
    // Text tokens, and the least this level of ink may ever be.
    const INKS: [&str; 3] = ["1", "2", "3"];
    const SURFACES: [&str; 8] = ["b", "p", "r", "g", "h", "eb", "in", "c"];

    for (theme, block) in themes(&css) {
        for ink in INKS {
            let colour = oklch(&block, ink);
            for surface in SURFACES {
                let ratio = contrast(colour, oklch(&block, surface));
                assert!(
                    ratio >= 4.5,
                    "{theme}: --ink-{ink} on --{surface} is {ratio:.2}:1, under the 4.5:1 \
                     WCAG AA minimum for normal text. Move the token, or step this \
                     surface's text up a level the way `.chip.on .n` does."
                );
            }
        }

        // And the levels stay three levels. Without this, the cheapest way to
        // pass everything above is to make all three tokens the same, which
        // would satisfy the letter of it and destroy what the palette is for.
        let ground = oklch(&block, "b");
        let (one, two, three) = (
            contrast(oklch(&block, "1"), ground),
            contrast(oklch(&block, "2"), ground),
            contrast(oklch(&block, "3"), ground),
        );
        assert!(
            one > two * 1.5 && two > three * 1.2,
            "{theme}: the ink series is {one:.1} / {two:.1} / {three:.1} on the page \
             background — these are no longer three distinguishable levels"
        );
    }
}

/// The one surface a level of ink cannot clear, handled where it happens.
///
/// Asserted because it is load-bearing for the test above: that test omits
/// `--accent-soft` from its surface list, and the omission is only honest
/// while these rules exist.
#[test]
fn a_selected_row_steps_its_secondary_text_up_a_level() {
    let css = css_code_only(&read("styles.css"));
    for rule in [
        ".chip.on .n",
        ".voices li.on .v-gender, .voices li.on .v-id",
    ] {
        let at = css.find(rule).unwrap_or_else(|| {
            panic!(
                "{rule} is gone, and with it the reason the contrast test may skip --accent-soft"
            )
        });
        let body = &css[at..css.len().min(at + 120)];
        assert!(
            body.contains("var(--ink-2)") || body.contains("var(--ink)"),
            "{rule} no longer steps up on --accent-soft: {body}"
        );
    }
}

/// D-183. Keyboard focus is visible, and nothing takes the ring away without
/// giving one back.
///
/// The stylesheet had **zero** `:focus-visible` rules. Three selectors set
/// `outline: none`, which for a keyboard user removes the only indication of
/// where they are; each now has a `:focus-visible` companion that puts a real
/// ring back, and the mouse still sees exactly what it saw.
#[test]
fn a_suppressed_focus_ring_is_always_given_back() {
    let css = css_code_only(&read("styles.css"));

    assert!(
        css.contains(":focus-visible"),
        "no rule in the stylesheet draws a keyboard focus ring"
    );

    for rule in css.split('}') {
        let Some((selectors, body)) = rule.split_once('{') else {
            continue;
        };
        if !body.contains("outline: none") {
            continue;
        }
        // Every selector that gives up the ring must appear again with
        // `:focus-visible` in place of `:focus`.
        for selector in selectors.split(',') {
            let selector = selector.trim();
            if selector.is_empty() {
                continue;
            }
            let restored = selector.replace(":focus", ":focus-visible");
            assert!(
                css.contains(&restored),
                "`{selector}` sets `outline: none` and nothing puts a ring back for \
                 the keyboard — add `{restored}`"
            );
        }
    }
}

/// D-183. Reordering controls are quiet by colour, not by transparency.
///
/// `.arrange { opacity: 0.42 }` composited its label to **2.19:1** dark and
/// **1.98:1** light: enabled controls that read as disabled, in the column an
/// operator had already failed to find once (D-101). Transparency is the part
/// that made the number meaningless, so transparency is what went — the
/// quietness is `--ink-2`, a step below the row's own text, stepping up to
/// full ink when the row is under the pointer.
#[test]
fn the_arrange_controls_are_not_faded_into_illegibility() {
    let css = css_code_only(&read("styles.css"));
    let at = css.find(".arrange {").expect("the arrange group is gone");
    let body = &css[at..css.len().min(at + 200)];
    assert!(
        !body.contains("opacity"),
        "the arrange group is transparent again, which is how its label reached \
         2.19:1: {body}"
    );
    // The disabled buttons keep theirs, and must: that is the one thing in
    // this column that is *meant* to be indistinct, and WCAG exempts it.
    assert!(
        css.contains(".arrange-button:disabled { opacity:"),
        "a disabled reorder button is no longer distinguishable from an enabled one"
    );
}

/// D-183. The narration cell is a control, and the keyboard can reach it.
///
/// It was a `<span>` with a click handler — no `tabindex`, no `role` — so the
/// one thing this grid exists to let an operator do could not be done without
/// a mouse, and a screen reader announced it as text. Focus is put back on it
/// when the editor closes, or every edit drops the keyboard at the top of the
/// document.
#[test]
fn the_narration_cell_is_reachable_from_the_keyboard() {
    let js = code_only(&read("app.js"));
    let at = js
        .find("editNarration(scene, cell)")
        .expect("the narration cell cannot be edited at all");
    let around = &js[at.saturating_sub(400)..js.len().min(at + 600)];

    for wanted in ["tabIndex", "\"role\"", "\"aria-label\"", "keydown"] {
        assert!(
            around.contains(wanted),
            "the narration cell is not a control — {wanted} is missing: {around}"
        );
    }
    assert!(
        js.contains("function refocusNarration"),
        "nothing puts the keyboard back on the row that was edited"
    );
}

/// D-185. No keystroke does a list's worth of work, or crosses the process
/// boundary, on its own.
///
/// Measured through the shipped `app.js` in node: twelve characters typed
/// into the scene filter rebuilt the grid **twelve times** — about 8,500
/// elements each at 500 scenes — and nine into the subtitle box made **nine**
/// `subtitle_preview` calls. One each, now.
///
/// Derived rather than a list of five: the point is that the *next* one is
/// caught too. Only the `el("id")` listeners are checked, which is exactly
/// right — the narration textarea's autosize is bound to a local element, is
/// O(1), and touches nothing outside itself.
#[test]
fn no_input_handler_redraws_or_calls_rust_per_keystroke() {
    let js = code_only(&read("app.js"));

    let mut checked = 0;
    for (at, _) in js.match_indices(".addEventListener(\"input\"") {
        // Back to the start of the statement, forward to the end of it.
        let from = js[..at].rfind('\n').map_or(0, |n| n + 1);
        let to = js[at..]
            .find(");\n")
            .map_or(js.len(), |n| (at + n + 2).min(js.len()));
        let statement = &js[from..to];
        if !statement.contains("el(\"") {
            continue;
        }
        checked += 1;
        assert!(
            statement.contains("onFrame("),
            "this input handler runs on every keystroke and is not coalesced to a \
             frame:\n  {}\nWrap it in `onFrame(…)`, or if it really is O(1) and \
             touches nothing outside itself, bind it to the element rather than \
             through `el(\"id\")`.",
            statement.trim()
        );
    }
    assert!(
        checked >= 5,
        "only {checked} input handlers were found — the scan is matching nothing, \
         so this passes by finding nothing to check"
    );
}

/// And it really waits for a frame.
///
/// The one property worth pinning about `onFrame`, and the only one: a
/// version that called `fn` straight through would satisfy every other test
/// here — the handlers would still be *wrapped* — while doing exactly the
/// per-keystroke work this decision removed.
///
/// **What is deliberately not asserted is throttle versus debounce.** That
/// looked like the interesting choice and it is not: `cancelAnimationFrame`
/// followed by `requestAnimationFrame` inside one frame still runs on the
/// next, so both forms redraw once per frame. Measured both ways through the
/// shipped file in node — twelve keystrokes over twelve frames gave twelve
/// redraws either way, and twelve inside one frame gave one. A test that
/// forbade one of them would be pinning a preference and calling it a
/// property.
#[test]
fn the_frame_coalescer_actually_waits_for_a_frame() {
    let js = code_only(&read("app.js"));
    let at = js
        .find("function onFrame(")
        .expect("the frame coalescer is gone");
    let body = &js[at..js.len().min(at + 400)];
    assert!(
        body.contains("requestAnimationFrame"),
        "`onFrame` no longer waits for a frame, so every handler wrapped in it \
         is doing per-keystroke work again: {body}"
    );
}

/// D-185. Both answers that cross the process boundary carry a token.
///
/// `drawPreview` has had one since D-106. `refreshOutput` had none, and it
/// paints the destination path — the one thing the Output screen exists to be
/// right about. Coalescing makes two answers overlapping rarer, which makes a
/// stale path on screen **harder to notice** rather than impossible, so the
/// net went in with the coalescing rather than instead of it.
#[test]
fn an_answer_from_rust_cannot_paint_over_a_later_one() {
    let js = code_only(&read("app.js"));
    for (function, token) in [
        ("async function drawPreview(", "previewToken"),
        ("async function refreshOutput(", "outputToken"),
    ] {
        let at = js
            .find(function)
            .unwrap_or_else(|| panic!("{function} is gone"));
        let body = &js[at..js.len().min(at + 1400)];
        assert!(
            body.contains(&format!("++{token}")),
            "{function} does not claim a {token}: {body}"
        );
        assert!(
            body.contains(&format!("!== {token}")),
            "{function} never checks its {token}, so a slow earlier answer can \
             paint over a fast later one: {body}"
        );
    }
}
