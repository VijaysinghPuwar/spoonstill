// The shell's frontend. It holds no render state and constructs no path: it
// passes folders and files the operator chose into Rust commands and draws the
// rows that come back (D-010, plan.md M4).
//
// Note what it never does. It does not open a file — `open_film` and
// `reveal_project` take no arguments, so there is no path here to point at
// anything. It does not decide a duration, a filter string, or a cache key. It
// does not even join a folder and a file name: `resolve_output` does that in
// Rust, and this file only ever holds the two halves and shows the answer.
//
// Two levels, and only two: **home** is the operator's projects plus the
// settings that belong to no project, and **a project** is the rail.

const { invoke, Channel, convertFileSrc } = window.__TAURI__.core;

// The dialog plugin's JS package is an npm dependency this shell deliberately
// does not have: there is no bundler here, so its command is invoked by name.
// Same command, same capability check in Rust.
const dialog = (options) => invoke("plugin:dialog|open", { options });

// Which operating system is drawing this window, as an attribute the
// stylesheet can branch on.
//
// `titleBarStyle: "Overlay"` in tauri.conf.json is **macOS only** — Tauri's
// `title_bar_style` is behind `#[cfg(target_os = "macos")]`, so on Windows it
// is ignored without a word and the window keeps its native title bar. The
// stylesheet reserves 82px at the left of `.titlebar` for the traffic lights
// that overlay puts there; on Windows there are no traffic lights and that
// reservation is dead space under a title bar we did not ask for. Same shape
// as D-088: a macOS assumption that a webview accepts silently.
//
// The default is macOS, so that machine renders exactly as it did before this
// line existed and never flashes; a non-Mac corrects itself as the module
// loads. It cannot be an inline script in index.html — the CSP is
// `script-src 'self'`, which forbids one.
document.documentElement.dataset.os = /Mac|iPhone|iPad/.test(navigator.userAgent)
  ? "macos"
  : /Windows/.test(navigator.userAgent)
    ? "windows"
    : "other";

const el = (id) => document.getElementById(id);
const MEDIA = [
  "jpg", "jpeg", "png", "webp", "tif", "tiff", "bmp", "heic",
  "mp3", "m4a", "wav", "aac", "flac", "ogg", "opus", "wma",
  "txt", "md",
];

const TABS = ["scenes", "voice", "subtitles", "output", "render"];

// The only things the frontend remembers: which project is on screen, what the
// last render produced, and what the operator has chosen for the *next* render.
// Everything else is asked for again.
let project = null;
let film = null;
let rendering = false;
let filter = "all";

// The next render's two decisions. Both are overrides for one run — nothing
// here is written into `project.yaml`, which this program only reads (D-013).
// `chosenVoice` of null means "whatever the project says".
let chosenVoice = null;
// The subtitle decision for the next render (D-106). Both null means "whatever
// the project says"; neither is ever written into `project.yaml` (D-013).
let chosenSubtitles = null;
let chosenTheme = null;
let themes = [];
let themesLoaded = false;
// The last preview asked for, so a burst of clicks paints the last one rather
// than whichever request happens to come back last.
let previewToken = 0;
// The same net for the destination, which crosses the same boundary and had
// none (D-185). Coalescing makes two answers overlapping rarer, which makes a
// stale path on screen harder to notice rather than impossible — and the path
// on screen is the one thing the Output screen exists to be right about.
let outputToken = 0;
// Why a render cannot start for a reason that is about this machine rather
// than about this project — a missing FFmpeg. Empty when there is none
// (D-105). Asked once per project open, not once per photograph (D-103).
let ffmpegBlocker = "";
let outDir = "";
let outName = "";
let outFull = "";
let outError = "";
// The shape and size for the next render (D-143). Both null means "whatever
// `project.yaml` says", exactly like the voice and the subtitle theme above —
// and, like them, neither is ever written into that file (D-013).
let chosenAspect = null;
let chosenSize = null;
// The two lists the choosers are drawn from, fetched once. Every pixel
// dimension in them was computed by Rust, because 4K portrait is 2160x3840 and
// a page that worked that out itself would be a second OutputSpec (D-010).
let formats = null;

// The provider's catalogue, fetched once per project, and the voice it falls
// back to when a project names none — so the window can say whose voice
// "default" actually is.
let voices = [];
let voicesLoaded = false;
let providerDefault = "";
// The machine's fallback voice, from Settings. Null means "the provider's own".
// Only the Settings screen fills this, and only for its own <select> — the
// *rule* reads the setting in Rust, because a page that has not opened Settings
// used to hold `null` here and silently ignore the fallback (D-166).
let appDefaultVoice = null;

// The voice the next render will use and who chose it, as Rust resolved it:
// `{ voice, origin, overrideForRender, said, detail }` (D-166). Null until the
// first `refreshVoice()`. Everything that names a voice reads this, including
// the render request — so what is shown and what is sent cannot disagree.
let voiceState = null;

// Per-scene render state, keyed by scene index. Wiped at the start of a render
// and filled by the progress channel — never merged into `project`, which is
// what the folder says rather than what this run did.
let live = new Map();

// ------------------------------------------------------------------ screens

const SCREENS = ["start", "settings", "fill", "app"];

function show(which) {
  for (const name of SCREENS) el(name).hidden = name !== which;
  const inProject = which === "fill" || which === "app";
  el("home").disabled = !inProject;
  el("home").title = inProject ? "All projects" : "spoonstill";
}

function setStatus(text) { el("status").textContent = text || ""; }

function tab(name) {
  for (const button of el("tabs").children) button.classList.toggle("on", button.dataset.tab === name);
  for (const pane of TABS) el("pane-" + pane).hidden = pane !== name;
  el("pane-chapter").hidden = true;
  if (name === "voice") loadVoices();
  if (name === "subtitles") loadThemes();
}

// --------------------------------------------------------------------- home

// The operator's projects, newest first. The list is Rust's — kept in the OS
// config directory and written every time a project opens, so there is no way
// to open one and have it not appear here.
async function loadHome() {
  let recent = [];
  try {
    recent = await invoke("recent_projects");
  } catch (error) {
    setStatus(String(error));
  }

  const list = el("project-list");
  list.innerHTML = "";
  el("no-projects").hidden = recent.length > 0;

  for (const entry of recent) {
    const li = document.createElement("li");
    if (!entry.exists) li.classList.add("gone");
    li.innerHTML =
      `<span class="p-name"></span><span class="p-path mono"></span>` +
      `<span class="p-when"></span><button class="p-forget" title="">Forget</button>`;
    li.children[0].textContent = entry.name;
    li.children[1].textContent = entry.pretty || entry.path;
    li.children[2].textContent = entry.exists ? ago(entry.at) : "moved or deleted";
    li.children[3].title = "Take this off the list. The folder is not touched.";
    li.title = entry.path;

    if (entry.exists) li.addEventListener("click", () => load(entry.path));
    li.children[3].addEventListener("click", async (event) => {
      event.stopPropagation();
      try {
        await invoke("forget_project", { path: entry.path });
        await loadHome();
      } catch (error) {
        setStatus(String(error));
      }
    });
    list.appendChild(li);
  }
}

// "4 minutes ago". Presentation, so it happens here and not in Rust, which
// hands over seconds since the epoch and no opinion.
function ago(seconds) {
  let value = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  for (const [size, unit] of [[60, "second"], [60, "minute"], [24, "hour"], [7, "day"], [52, "week"]]) {
    if (value < size) return `${value} ${unit}${value === 1 ? "" : "s"} ago`;
    value = Math.floor(value / size);
  }
  return `${value} year${value === 1 ? "" : "s"} ago`;
}

function goHome() {
  if (rendering) {
    setStatus("A render is running — stop it first.");
    return;
  }
  project = null;
  film = null;
  live = new Map();
  voices = [];
  voicesLoaded = false;
  el("t-name").textContent = "spoonstill";
  el("t-path").textContent = "";
  el("counts").textContent = "";
  tab("scenes");
  show("start");
  setStatus("");
  loadHome();
}

// ----------------------------------------------------------------- settings

async function openSettings() {
  show("settings");
  setStatus("");
  // FFmpeg first: a machine that cannot render at all should not be told
  // about voices first (D-105).
  await checkFfmpegSetting();
  await checkProvider();
  await loadFallbackVoice();
  await loadActivityLog();
  await loadGraphics();
}

// Whether renders use the graphics card (D-199). Detection encodes a test
// frame, so the first answer can take a second or two.
function drawGraphics(view) {
  el("app-graphics").value = view.on ? "on" : "off";
  const said = el("app-graphics-said");
  said.classList.remove("bad");
  if (view.usable.length === 0) {
    said.textContent = "No graphics card this machine can encode with — renders use the processor.";
  } else {
    said.textContent = `Renders use ${view.using}.`;
  }
}

async function loadGraphics() {
  try {
    drawGraphics(await invoke("graphics_status"));
  } catch (error) {
    el("app-graphics-said").classList.add("bad");
    el("app-graphics-said").textContent = String(error);
  }
}

async function setGraphics(value) {
  const said = el("app-graphics-said");
  said.textContent = "Saving…";
  try {
    drawGraphics(await invoke("set_use_graphics", { on: value === "on" }));
  } catch (error) {
    said.classList.add("bad");
    said.textContent = String(error);
  }
}

// One CSV, every project, every event (D-093).
async function loadActivityLog() {
  const said = el("activity-said");
  try {
    const info = await invoke("activity_log");
    el("activity-path").textContent = info.path;
    el("activity-open").disabled = !info.exists;
    said.classList.remove("bad");
    said.textContent = info.exists
      ? `${(info.size / 1024).toFixed(0)} KB`
      : "Empty until you render.";
  } catch (error) {
    el("activity-path").textContent = "—";
    el("activity-open").disabled = true;
    said.classList.add("bad");
    said.textContent = String(error);
  }
}

async function openActivityLog(reveal) {
  try {
    await invoke("open_activity_log", { reveal });
  } catch (error) {
    const said = el("activity-said");
    said.classList.add("bad");
    said.textContent = String(error);
  }
}

async function checkProvider() {
  const state = el("app-provider-state");
  state.className = "state";
  state.textContent = "Checking…";
  try {
    const status = await invoke("provider_status", { provider: "edge" });
    providerDefault = status.default_voice || providerDefault;
    el("app-provider").textContent = status.id;
    state.className = "state " + (status.ready ? "ready" : "missing");
    state.textContent = status.ready
      ? "Ready. Written lines can be spoken."
      : status.need;
    // Offered only when it would do something. D-092, D-105.
    drawFix(el("app-provider-fix"), status, async () => {
      await checkProvider();
      await loadFallbackVoice();
    });
  } catch (error) {
    state.className = "state missing";
    state.textContent = String(error);
    el("app-provider-fix").hidden = true;
  }
}

// The other half of D-105: FFmpeg is checked and installed exactly the way the
// voice service is. Until now the screen reporting the *more* serious of the
// two problems was the one that could do less about it — a missing voice
// service had a button, and a missing FFmpeg had the string
// `brew install ffmpeg` and a suggestion to find a terminal.
async function checkFfmpegSetting() {
  const state = el("app-ffmpeg-state");
  state.className = "state";
  state.textContent = "Checking…";
  try {
    const status = await invoke("ffmpeg_status");
    state.className = "state " + (status.ready ? "ready" : "missing");
    state.textContent = status.ready
      ? "Ready. Projects can be rendered."
      : status.need;
    drawFix(el("app-ffmpeg-fix"), status, () => checkFfmpegSetting());
  } catch (error) {
    state.className = "state missing";
    state.textContent = String(error);
    el("app-ffmpeg-fix").hidden = true;
  }
}

// The fallback voice: what a project that names none will use. A *fallback*,
// never a write — project.yaml stays an input (D-013, D-092).
async function loadFallbackVoice() {
  const select = el("app-voice");
  const said = el("app-voice-said");
  try {
    const view = await invoke("app_settings");
    appDefaultVoice = view.settings.default_voice || null;
    el("app-chapter-cut").value = view.settings.chapter_cut || "time";
    // A file that is there and could not be read is said where its setting is
    // (D-171). `drawFix` hides itself when there is nothing wrong, so the
    // ordinary machine sees no change.
    drawFix(el("app-settings-fix"), view.problem || { ready: true },
      () => loadFallbackVoice());
  } catch {
    appDefaultVoice = null;
    el("app-settings-fix").hidden = true;
  }

  let catalogue = [];
  try {
    catalogue = await invoke("voices", { provider: "edge" });
  } catch {
    catalogue = [];
  }

  select.innerHTML = "";
  const own = document.createElement("option");
  own.value = "";
  own.textContent = providerDefault
    ? `The provider's own — ${describe(providerDefault)}`
    : "The provider's own";
  select.appendChild(own);

  for (const voice of catalogue) {
    const option = document.createElement("option");
    option.value = voice.id;
    option.textContent = `${voiceName(voice)} · ${languageOf(voice.locale)} · ${voice.gender}`;
    select.appendChild(option);
  }
  select.value = appDefaultVoice ?? "";
  select.disabled = catalogue.length === 0;

  said.textContent = catalogue.length === 0 ? "Install the voice service first." : "";
}

async function setFallbackVoice(id) {
  try {
    const settings = await invoke("set_default_voice", { voice: id || null });
    appDefaultVoice = settings.default_voice || null;
    el("app-voice").value = appDefaultVoice ?? "";
    el("app-voice-said").textContent = "";
  } catch (error) {
    el("app-voice-said").textContent = String(error);
  }
}

// ------------------------------------------------------------------ opening

async function newProject() {
  // The system dialog can make a folder, so "New project" is "choose where" —
  // and Rust refuses a folder that already holds a film.
  const chosen = await dialog({
    directory: true, multiple: false,
    title: "Choose an empty folder for the new project",
  });
  if (typeof chosen !== "string") return;
  try {
    const root = await invoke("create_project", { path: chosen });
    // Read back rather than assembled here (D-156). A project built by hand out
    // of the path was missing every field the rest of the page reads, it never
    // reached `remember`, so a new project was not on Home until something else
    // validated it — and `name` was split on "/", which is not how a Windows
    // path is spelled. `load` is the one way this window comes to have a
    // project open; an empty folder lands on the same fill screen.
    await load(root);
  } catch (error) {
    setStatus(String(error));
  }
}

async function openProject() {
  const chosen = await dialog({ directory: true, multiple: false, title: "Open a project folder" });
  if (typeof chosen === "string") await load(chosen);
}

async function load(path, fresh = false) {
  setStatus("Reading the folder…");
  const opening = project?.root !== path;
  try {
    project = await invoke("validate_project", { path, fresh });
  } catch (error) {
    project = null;
    show("start");
    setStatus(String(error));
    loadHome();
    return;
  }

  el("t-name").textContent = project.name;
  el("t-path").textContent = project.root;

  if (opening) {
    voices = [];
    voicesLoaded = false;
    themesLoaded = false;
    restoreChoices();
  }

  // "Choose photos…" is for a folder that has none — not for one whose photos
  // are all there and could not be read (D-103). Rust decides which this is;
  // a project with problems goes to the grid, where the problem list is.
  if (project.empty) {
    el("fill-name").textContent = project.name;
    show("fill");
    setStatus(project.root);
    return;
  }
  show("app");
  // Before the first draw, because the rail and the Voice screen both name a
  // voice and neither can name one until Rust has resolved it (D-166).
  await refreshVoice();
  draw();
  // Not awaited, for the same reason the FFmpeg check below is not: the two
  // lists are constants and the grid should not wait on an IPC round trip to
  // appear. `drawFormats` returns early until they land, and is called again
  // when they do.
  loadFormats().then(drawFormats);
  if (project.scenes.length === 0) tab("scenes");
  // Not awaited: whether this machine has FFmpeg is one spawn, and the grid
  // should not wait on it to appear. It settles into the Render screen and
  // the button a moment later.
  guard(checkFfmpeg());
}

// Ask, once, whether this machine can render at all.
//
// D-105, and the prevention half of it. Every render needs FFmpeg; a machine
// without it used to discover that as a wall of per-photograph probe failures
// (D-103), and even after D-103 named it correctly the operator was handed a
// command line and left to find a terminal. Asked here, at project open, the
// answer reaches the Render screen as a disabled button that explains itself
// (D-089) with the fix attached to the explanation.
//
// A check that cannot run is deliberately *not* a blocker: the render itself
// is still the authority on whether FFmpeg works, and a window that refuses to
// render because it could not ask is worse than one that tries and reports.
async function checkFfmpeg() {
  try {
    const status = await invoke("ffmpeg_status");
    ffmpegBlocker = status.ready ? "" : status.need;
    drawFix(el("render-fix"), status, () => checkFfmpeg());
  } catch {
    ffmpegBlocker = "";
    el("render-fix").hidden = true;
  }
  updateRender();
}

// ------------------------------------------------------------------- adding

async function chooseMedia() {
  const chosen = await dialog({
    multiple: true,
    title: "Choose photos, recordings and scripts",
    filters: [{ name: "Photos, recordings and scripts", extensions: MEDIA }],
  });
  const files = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
  if (files.length > 0) await addMedia(files);
}

async function addMedia(files) {
  if (!project || rendering) return;
  setStatus(`Copying ${files.length} file${files.length === 1 ? "" : "s"} in…`);
  try {
    const report = await invoke("add_media", { root: project.root, files });
    let line = report.summary;
    if (report.orphans > 0) line += " — add more photos to use the rest";
    if (report.skipped.length > 0) line += ` · skipped ${report.skipped.join("; ")}`;
    await load(project.root);
    setStatus(line);
  } catch (error) {
    setStatus(String(error));
  }
}

// ------------------------------------------------------------------ drawing

function draw() {
  el("s-geometry").textContent = project.geometry;
  drawFormats();
  el("s-scenes").textContent = String(allRows().length);
  el("s-mode").textContent = project.mode;
  el("s-root").textContent = project.root;
  el("s-output").textContent = project.output_path || project.output;

  drawChips();
  drawRows();
  drawProblems();
  drawVoiceChoice();
  drawStatus();

  el("pip-scenes").textContent = String(allRows().length);
  updateRender();
}

const count = (kind) => project.scenes.filter((s) => s.source === kind).length;

// Every row of the grid in film order: finished scenes and the ones still
// waiting for a picture (D-193). Rust numbers both (`position`), so the order
// is not worked out here.
function allRows() {
  const waiting = (project.waiting ?? []).map((w) => ({ ...w, waiting: true, source: "waiting" }));
  return [...project.scenes, ...waiting].sort((a, b) => a.position - b.position);
}
const attention = () => project.problems.filter((p) => p.severity === "error").length;

function drawChips() {
  const chips = [
    ["all", "All", allRows().length, false],
    ["attention", "Needs attention", attention(), true],
    ["waiting", "Needs a picture", (project.waiting ?? []).length, true],
    ["tts", "Spoken", count("tts"), false],
    ["file", "Supplied", count("file"), false],
    ["silent", "Silent", count("silent"), false],
  ];
  const bar = el("chips");
  bar.innerHTML = "";
  for (const [key, label, n, loud] of chips) {
    const button = document.createElement("button");
    button.className = "chip" + (filter === key ? " on" : "") + (loud && n > 0 ? " attention" : "");
    button.innerHTML = `<span></span><span class="n"></span>`;
    button.children[0].textContent = label;
    button.children[1].textContent = String(n);
    button.addEventListener("click", () => { filter = key; drawChips(); drawRows(); });
    bar.appendChild(button);
  }
}

function visible() {
  const needle = el("search").value.trim().toLowerCase();
  const broken = new Set(project.problems.filter((p) => p.severity === "error" && p.scene).map((p) => p.scene));
  return allRows().filter((scene) => {
    if (filter === "attention" && !broken.has(scene.id)) return false;
    if (["tts", "file", "silent", "waiting"].includes(filter) && scene.source !== filter) return false;
    if (!needle) return true;
    return (scene.id + " " + scene.image + " " + scene.narration + " " + scene.audio)
      .toLowerCase().includes(needle);
  });
}

function drawRows() {
  const broken = new Set(project.problems.filter((p) => p.severity === "error" && p.scene).map((p) => p.scene));
  const rows = el("rows");
  const shown = visible();
  // Kept across the redraw: emptying the table collapses it, which clamps the
  // scroll to the top — after every picture dropped on row 300 of 580 (D-194).
  const wrap = rows.closest(".grid-wrap");
  const keep = wrap.scrollTop;
  rows.innerHTML = "";
  el("grid-empty").hidden = shown.length > 0;

  // The aspect this run will render in — the override if there is one, else
  // the project's own. It used to be sniffed out of the geometry string by
  // testing for a `1080x1920` prefix, which is one size of one aspect: a 4K
  // Short showed landscape thumbnails (D-143).
  const aspect = chosenAspect ?? project.aspect;
  const shape = aspect === "9:16" ? "portrait" : aspect === "1:1" ? "square" : "";

  for (const scene of shown) {
    const tr = document.createElement("tr");
    tr.id = scene.waiting ? "waiting-" + scene.id : "scene-" + scene.index;
    if (broken.has(scene.id)) tr.classList.add("problem");
    tr.innerHTML = `
      <td class="c-scene"></td>
      <td class="c-still">${scene.waiting
        ? `<button class="thumb-missing ${shape}">+</button>`
        : `<button class="thumb-button"><img class="thumb ${shape}" loading="lazy" alt="" /></button>`}</td>
      <td class="c-source"><div class="source-cell">
        <span class="file"></span><span class="narration"></span>
      </div></td>
      <td class="c-audio"><span class="badge"></span></td>
      <td class="c-resolved"></td>
      <td class="c-arrange"><div class="arrange"></div></td>`;

    // textContent, never innerHTML, for anything an operator named or typed: a
    // file called <img onerror=…> is a filename, not markup (D-052).
    tr.querySelector(".c-scene").textContent = scene.id;
    if (!scene.waiting) {
      tr.querySelector(".thumb").src = convertFileSrc(scene.image_path);
      tr.querySelector(".file").textContent = scene.image;
    }

    const badge = tr.querySelector(".badge");
    badge.classList.add(scene.source);
    badge.textContent = { tts: "TTS", file: "FILE", silent: "SILENT", waiting: "NO PICTURE" }[scene.source]
      ?? scene.source;

    const cell = tr.querySelector(".narration");
    if (scene.waiting && !scene.narration) {
      cell.textContent = scene.audio;
      cell.classList.add("blank");
    } else if (scene.source === "file") {
      cell.textContent = scene.audio;
      cell.classList.add("blank");
      cell.title = "This scene has a recording. Its length is the recording's.";
    } else if (scene.narration) {
      cell.textContent = scene.narration;
      cell.title = scene.narration;
    } else {
      cell.textContent = project.convention ? "Write what this scene should say…" : "silent";
      cell.classList.add("blank");
    }
    if (scene.source !== "file" && project.convention && !(scene.waiting && !scene.narration)) {
      // A real control, not a `<span>` with a click handler (D-183). It had
      // no `tabindex` and no `role`, so writing a narration — the one thing
      // this grid exists to let you do — could not be reached from the
      // keyboard at all, and a screen reader announced it as text.
      cell.tabIndex = 0;
      cell.setAttribute("role", "button");
      cell.setAttribute("aria-label", `Narration for scene ${scene.id}`);
      cell.addEventListener("click", () => editNarration(scene, cell));
      cell.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          // Space scrolls the grid otherwise, which moves the row out from
          // under the editor that is about to open.
          event.preventDefault();
          editNarration(scene, cell);
        }
      });
    }

    tr.querySelector(".c-resolved").innerHTML = resolved(scene);
    drawArrange(tr.querySelector(".arrange"), scene, shown.length);
    tr.dataset.scene = scene.id;
    drawPictureControl(tr.querySelector(".c-still"), scene);
    rows.appendChild(tr);
  }
  wrap.scrollTop = keep;
}

// Duration, and the honesty about it: a silent scene's length is *declared*, a
// spoken or supplied one is *measured* and is not known until the render
// resolves it (D-021). A number here before that would be a guess, so it is a
// dash until the render fills it in.
function resolved(scene) {
  const now = live.get(scene.index);
  if (now?.frames) {
    return `${now.seconds.toFixed(3)}<span class="frames">${now.frames}f</span>`;
  }
  if (now?.seconds) return `${now.seconds.toFixed(3)}<span class="frames">audio</span>`;
  if (scene.seconds != null) {
    return `${scene.seconds.toFixed(3)}<span class="frames">declared</span>`;
  }
  return `<span class="frames">—</span>`;
}

// Put the keyboard back on the row that was being edited.
//
// The cell the editor replaced is detached by the redraw that follows a save,
// so this finds the new one rather than holding the old. A filter that now
// hides the row leaves nothing to focus, which is correct and is why this is
// optional chaining rather than an assertion.
function refocusNarration(scene) {
  document.getElementById("scene-" + scene.index)?.querySelector(".narration")?.focus();
}

// One row, edited in place. Enter or blur saves; Escape puts it back.
//
// A textarea rather than an input, and it grows to whatever it holds. The grid
// shows one elided line because it is a review grid at 500 rows (D-051) — but
// the moment the operator opens a line to read or change it, showing them two
// thirds of their own sentence is the same defect as clipping it. Narration is
// not a short field: D-095 exists because a scene can hold an hour of it.
function editNarration(scene, cell) {
  if (rendering) return;
  const input = document.createElement("textarea");
  input.className = "narration-input";
  input.rows = 1;
  input.value = scene.narration;
  input.placeholder = "What this scene says. Leave it empty for a silent scene.";
  cell.replaceWith(input);

  // Capped, not unbounded: one very long line must not push every other row
  // off the screen. Past the cap the textarea scrolls, which is the one place
  // in this window where scrolling text is the right answer.
  const fit = () => {
    input.style.height = "auto";
    // `box-sizing: border-box` is set for everything in this window, so the
    // height has to carry the border that `scrollHeight` does not — without it
    // every line is two pixels short and the last one is shaved.
    const border = input.offsetHeight - input.clientHeight;
    const cap = Math.round(innerHeight * 0.4);
    input.style.height = Math.min(input.scrollHeight + border, cap) + "px";
  };
  input.addEventListener("input", fit);
  fit();

  input.focus();
  input.select();

  let settled = false;
  const finish = async (save) => {
    if (settled) return;
    settled = true;
    const text = input.value;
    input.replaceWith(cell);
    // Escape, or nothing typed: the row is unchanged and the keyboard stays
    // on it. Without this an operator is dropped at the top of the document
    // after every edit (D-183).
    if (!save || text === scene.narration) {
      cell.focus();
      return;
    }
    try {
      setStatus("Saving…");
      // The command answers with the row it changed, and that row replaces
      // this one (D-182). Reloading the whole project here read all 500
      // scenes and probed every file twice over — 2.6 s at n=500 — to show
      // one line of text the operator had just typed themselves.
      //
      // Nothing here decides what the row should say: every field comes from
      // Rust, off the same function that builds the grid (D-010).
      const edited = await invoke("set_narration", { root: project.root, scene: scene.id, text });
      if (edited) {
        Object.assign(scene, edited);
        drawRows();
        drawStatus();
      } else {
        // Rust could not find the scene it had just written to, which means
        // something changed underneath us. Its cheap answer would be a
        // confident wrong one, so ask for the whole project.
        await load(project.root);
      }
      refocusNarration(scene);
      setStatus(text.trim() ? `Scene ${scene.id} will be spoken.` : `Scene ${scene.id} is silent again.`);
    } catch (error) {
      cell.focus();
      setStatus(String(error));
    }
  };

  input.addEventListener("keydown", (event) => {
    // Enter saves, because a scene is one spoken line far more often than it
    // is a paragraph. Shift+Enter is the way to a second line.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      finish(true);
    }
    if (event.key === "Escape") finish(false);
  });
  input.addEventListener("blur", () => finish(true));
}

// Move a scene, or take it out (D-100).
//
// Only under the folder convention: there, the scene's *number is* its
// position, so reordering means renaming files and spoonstill owns that
// naming. A manifest's order is the operator's own column, and rewriting
// someone's CSV is not this program's business (D-050).
//
// Positions are the scene's real index in the film, not its row in a filtered
// view — "move up" while a filter hides the scene above would otherwise move
// it somewhere the operator cannot see.
function drawArrange(box, scene, showing) {
  if (!project.convention) {
    box.textContent = "";
    box.title = "This project's order comes from its manifest — edit the CSV to change it.";
    return;
  }

  const total = allRows().length;
  const position = scene.position;
  const filtered = showing !== total;

  // `wordy` marks a button whose label is a word rather than a glyph. Below
  // 1080px the stylesheet swaps that word for ✕ so the column can be narrow
  // enough to stay on screen; the word survives as the tooltip and as the
  // accessible name, which is why both are set here rather than left to the
  // text (D-101).
  const button = (label, hint, enabled, run, wordy) => {
    const b = document.createElement("button");
    b.className = "arrange-button" + (wordy ? " wordy" : "");
    b.textContent = label;
    b.title = hint;
    b.setAttribute("aria-label", label);
    b.disabled = !enabled || rendering;
    if (enabled && !rendering) b.addEventListener("click", run);
    box.appendChild(b);
    return b;
  };

  button("↑", filtered ? `Move to ${position - 1} (positions are the film's, not this filtered list's)`
                       : `Move to position ${position - 1}`,
    position > 1, () => arrange("move_scene", { root: project.root, scene: scene.id, to: position - 1 },
      `Scene ${scene.id} moved up.`));

  button("↓", filtered ? `Move to ${position + 1} (positions are the film's, not this filtered list's)`
                       : `Move to position ${position + 1}`,
    position < total, () => arrange("move_scene", { root: project.root, scene: scene.id, to: position + 1 },
      `Scene ${scene.id} moved down.`));

  // Two steps rather than a dialog. A modal confirm blocks a webview outright,
  // and a single click that renumbers the whole film is a click nobody meant.
  const remove = button("Remove", "Move this scene's files to removed/ — nothing is deleted", true, () => {
    if (remove.dataset.armed) {
      arrange("remove_scene", { root: project.root, scene: scene.id }, null);
      return;
    }
    box.querySelectorAll(".arrange-button").forEach((b) => delete b.dataset.armed);
    remove.dataset.armed = "1";
    remove.textContent = "Remove?";
    remove.setAttribute("aria-label", "Remove scene " + scene.id + " — click again to confirm");
    remove.classList.add("armed");
    setStatus(`Click again to take scene ${scene.id} out. Its files move to removed/, not deleted.`);
    setTimeout(() => {
      if (!remove.dataset.armed) return;
      delete remove.dataset.armed;
      remove.textContent = "Remove";
      remove.setAttribute("aria-label", "Remove");
      remove.classList.remove("armed");
    }, 4000);
  }, true);
}

// One arrangement, then a reload — the folder is the truth and it has changed
// under us, so nothing here edits the model in place.
async function arrange(command, args, said) {
  if (rendering) return;
  try {
    setStatus("Rearranging…");
    const answer = await invoke(command, args);
    await load(project.root);
    setStatus(said ?? String(answer));
  } catch (error) {
    setStatus(String(error));
  }
}

function drawProblems() {
  const list = el("problem-list");
  list.innerHTML = "";
  el("problems").hidden = project.problems.length === 0;
  for (const problem of project.problems) {
    const li = document.createElement("li");
    li.className = problem.severity === "error" ? "error" : "warn";
    const sev = document.createElement("span");
    sev.className = "sev";
    sev.textContent = problem.severity;
    const text = document.createElement("span");
    text.textContent = (problem.scene ? `scene ${problem.scene}: ` : "") + problem.message;
    li.append(sev, text);

    // Nearly every problem in this list is about the operator's own files and
    // there is nothing to press. A missing tool is the exception, and it is
    // also the one an operator is least equipped to fix by hand — so it is the
    // one that gets a button, here, in the list where it was reported (D-105).
    if (problem.install) {
      const said = document.createElement("span");
      said.className = "fix-said";
      said.hidden = true;
      const fix = document.createElement("button");
      fix.className = "primary small";
      fix.textContent = "Install it for me";
      fix.addEventListener("click", () =>
        guard(runInstall(problem.install, fix, said, () => load(project.root))));
      li.append(fix, said);
    }
    list.appendChild(li);
  }
}

function drawStatus() {
  const shown = visible().length;
  const declared = project.scenes.reduce((sum, s) => sum + (s.seconds ?? 0), 0);
  const parts = [`${shown} of ${allRows().length} scenes`];
  if (declared > 0) parts.push(`${declared.toFixed(1)}s declared`);
  // An empty project's one "error" is that it has nothing in it yet, which
  // is not something that "needs attention".
  const errors = project.empty ? 0 : attention();
  el("counts").innerHTML = "";
  el("counts").textContent = parts.join("   ");
  if (errors > 0) {
    const loud = document.createElement("span");
    loud.className = "attention";
    loud.textContent = `   ${errors} ${errors === 1 ? "needs" : "need"} attention`;
    el("counts").appendChild(loud);
  }
  setStatus(project.root);
}


// -------------------------------------------------------------- missing tools
//
// D-105. Every external program spoonstill needs can be absent, and being
// absent used to produce one line of grey text holding four instructions,
// three of which needed a terminal:
//
//   `edge-tts` is not on this machine. Install it with `pip install edge-tts`
//   (or `brew install edge-tts`), press Install in Settings, or point
//   SPOONSTILL_EDGE_TTS at it.
//
// It was shown on the Voice screen above an empty list, with the only button
// that could act on it one level up under Settings. Rust now answers in three
// separate fields — `need`, `install`, `detail` — and this is the one function
// that draws them. Wherever a tool can be missing, this appears *there*: the
// plain sentence, the button that ends it, and the technical half folded away.
//
// `onFixed` is awaited after a successful install and reloads whatever the
// missing tool was blocking, so the screen the operator is already looking at
// becomes the screen that works. They never navigate anywhere to apply a fix.
function drawFix(host, status, onFixed) {
  host.replaceChildren();
  host.hidden = Boolean(status.ready);
  if (status.ready) return;

  const need = document.createElement("p");
  need.className = "fix-need";
  need.textContent = status.need || "Something spoonstill needs is not available.";

  const actions = document.createElement("div");
  actions.className = "fix-actions";

  const said = document.createElement("p");
  said.className = "fix-said";
  said.hidden = true;

  if (status.install) {
    const install = document.createElement("button");
    install.className = "primary";
    install.textContent = "Install it for me";
    install.addEventListener("click", () =>
      guard(runInstall(status.install, install, said, onFixed)));
    actions.appendChild(install);
  }

  const again = document.createElement("button");
  again.textContent = "Check again";
  again.addEventListener("click", () => guard(onFixed()));
  actions.appendChild(again);

  host.append(need, actions, said);

  // Never the first thing anybody sees, and never thrown away either: this is
  // the path that was tried and the line the tool printed, which is what a
  // diagnostics report is made of.
  if (status.detail) {
    const more = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = "Technical details";
    const detail = document.createElement("p");
    detail.className = "fix-detail mono wrap";
    detail.textContent = status.detail;
    more.append(summary, detail);
    host.appendChild(more);
  }
}

// Run this machine's own package manager, and say so while it happens.
//
// Homebrew on a cold cache genuinely takes minutes, so the button says what is
// happening rather than going quiet — an operator who thinks the window has
// frozen closes it, and closing it mid-install is how a half-installed tool
// happens.
async function runInstall(tool, button, said, onFixed) {
  const was = button.textContent;
  button.disabled = true;
  button.textContent = "Installing…";
  said.hidden = false;
  said.classList.remove("bad");
  said.textContent =
    "Working. This uses the package manager already on your machine and can take " +
    "a few minutes — you can leave this window open.";
  try {
    const ran = await invoke("install_tool", { tool });
    said.textContent = `Done. ${ran}`;
    // The whole point of the button: the screen repairs itself. Rust locates
    // the binary again after installing (D-104) rather than trusting the path
    // it resolved before the file existed.
    await onFixed();
  } catch (error) {
    said.classList.add("bad");
    said.textContent = String(error);
    button.disabled = false;
    button.textContent = was === "Installing…" ? "Try again" : was;
  }
}

// ------------------------------------------------------------------- voices

// `en-GB` is not a language, it is a code for one. The platform already knows
// every one of them, so this asks it rather than shipping a table that would go
// stale — and falls back to the code itself where it does not.
//
// It asks for the **parts** rather than for the whole tag on purpose. Given
// `en-GB` the platform answers "British English", which is correct and which
// files the English voices under A, B and I — the operator looking for English
// then has to already know they want the Australian one. Built from its parts
// it reads "English (United Kingdom)", and every English sits together.
const languageOf = (() => {
  const make = (type) => {
    try { return new Intl.DisplayNames(["en"], { type }); } catch { return null; }
  };
  const language = make("language");
  const region = make("region");
  const script = make("script");
  const say = (names, code) => {
    try { return names?.of(code) || code; } catch { return code; }
  };

  return (locale) => {
    if (!locale) return "";
    const parts = String(locale).split("-");
    const tail = parts
      .slice(1)
      .map((part) => {
        if (/^[A-Z][a-z]{3}$/.test(part)) return say(script, part);
        if (/^([A-Za-z]{2}|[0-9]{3})$/.test(part)) return say(region, part.toUpperCase());
        return null;
      })
      .filter(Boolean);
    const head = say(language, parts[0]);
    return tail.length > 0 ? `${head} (${tail.join(", ")})` : head;
  };
})();

// Edge spells a voice `en-GB-RyanNeural`. That is the id the renderer needs and
// the id stays visible, but it is not a name anyone chooses a narrator by, so
// the list leads with "Ryan" and keeps the id in its own column.
function voiceName(voice) {
  const locale = voice.locale || voice.id.split("-").slice(0, 2).join("-");
  let name = voice.id;
  if (name.startsWith(locale + "-")) name = name.slice(locale.length + 1);
  name = name.replace(/Neural$/, "");
  return name.replace(/([a-z])([A-Z])/g, "$1 $2") || voice.id;
}

// "Ryan · British English" — for anywhere a voice is named outside the list.
//
// Works with no catalogue loaded, because the id already carries both halves:
// Settings names the provider's default voice before any project is open.
function describe(id) {
  if (!id) return "";
  const known = voices.find((v) => v.id === id);
  const voice = known ?? { id: String(id), locale: String(id).split("-").slice(0, 2).join("-") };
  const language = languageOf(voice.locale);
  return language ? `${voiceName(voice)} · ${language}` : voiceName(voice);
}

// Which of the four answers decided the voice, in the words each surface has
// room for. Rust's `said` is the long form; these are the column-width form.
// Kept beside each other so no origin can quietly lose its label.
const VOICE_MARK = {
  run: "\u2713 Selected",
  project: "From project.yaml",
  fallback: "Your fallback",
  unchosen: "Nobody chose",
};

// Ask Rust which voice the next render uses and who chose it (D-166).
//
// The page used to answer this itself, in two places that had to agree — the
// Render summary read `effectiveVoice()` and the render request built
// `chosenVoice || (projectNamesNoVoice() ? appDefaultVoice : null)`. One rule,
// two spellings, nothing asserting they matched. Now there is one call, and
// both read its result.
//
// On failure the previous answer is kept rather than cleared: a stale label is
// wrong about *when*, and clearing it would make the screen wrong about *what*,
// which is the mistake this whole decision exists to stop.
async function refreshVoice() {
  try {
    voiceState = await invoke("voice_choice", {
      chosen: chosenVoice,
      projectVoice: project?.voice ?? "",
      providerDefault,
    });
  } catch (error) {
    setStatus(String(error));
  }
  return voiceState;
}

function effectiveVoice() {
  return voiceState?.voice || "";
}

async function loadVoices() {
  if (!project || voicesLoaded) return;
  voicesLoaded = true;

  const state = el("provider-state");
  state.className = "state";
  state.textContent = `Asking ${project.provider} what it has…`;

  try {
    const status = await invoke("provider_status", { provider: project.provider });
    providerDefault = status.default_voice || "";
    // `providerDefault` is an input to the rule, so the answer is re-asked the
    // moment it lands — this is the first point at which "nobody chose this"
    // can name the voice an operator who does nothing will actually hear.
    await refreshVoice();
    if (!status.ready) {
      // Left un-loaded on purpose: the fix for "edge-tts is not installed" is
      // to install it, and the operator who just did that comes straight back
      // to this screen. Re-asking costs one process spawn.
      voicesLoaded = false;
      state.className = "state missing";
      // The plain sentence, and only that. The command lines and the
      // environment variable that used to be on this line are in `detail`,
      // behind the disclosure the component draws (D-105).
      state.textContent = status.need;
      // The button, here, rather than "press Install in Settings" — and when
      // it succeeds the catalogue loads underneath it without the operator
      // going anywhere.
      drawFix(el("voice-fix"), status, () => loadVoices());
      drawVoiceChoice();
      return;
    }
    el("voice-fix").hidden = true;
  } catch (error) {
    voicesLoaded = false;
    state.className = "state missing";
    state.textContent = String(error);
    el("voice-fix").hidden = true;
    return;
  }

  try {
    voices = await invoke("voices", { provider: project.provider });
  } catch (error) {
    voicesLoaded = false;
    state.className = "state missing";
    // Same shape as a missing tool (D-105): one sentence and a button,
    // never a wall of Python traceback with nothing to press. This is the
    // network call `provider_status` above does not cover (D-094) — it can
    // fail on a machine that has `edge-tts` and still has no route to the
    // service right now, which reads exactly like a broken install unless
    // there is something here to press (D-141).
    state.textContent = "";
    drawFix(
      el("voice-fix"),
      {
        ready: false,
        need: "Could not load the voice list.",
        detail: String(error),
      },
      () => loadVoices(),
    );
    return;
  }

  // Sorted by the name of the language, not by its code — so every English
  // sits together under E rather than scattered between Amharic and Zulu.
  voices.sort((a, b) =>
    languageOf(a.locale).localeCompare(languageOf(b.locale)) ||
    voiceName(a).localeCompare(voiceName(b)));

  const spoken = count("tts");
  state.className = "state ready";
  state.textContent =
    `${project.provider} is ready — ${voices.length} voices, ` +
    `${spoken} scene${spoken === 1 ? "" : "s"} will be spoken.`;

  drawLocales();
  drawVoiceChoice();
  drawVoices();
}

function drawLocales() {
  const seen = new Map();
  for (const voice of voices) {
    if (voice.locale && !seen.has(voice.locale)) seen.set(voice.locale, languageOf(voice.locale));
  }
  const sorted = [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]));

  const select = el("locale");
  select.innerHTML = `<option value="">Every language</option>`;
  for (const [code, name] of sorted) {
    const option = document.createElement("option");
    option.value = code;
    // The code stays, quietly: it is what goes in `project.yaml`, and an
    // operator comparing `en-GB` with `en-AU` needs to see which is which.
    option.textContent = `${name}  ·  ${code}`;
    select.appendChild(option);
  }

  // Open on a language the operator can read. The project's own voice decides
  // it; failing that, the one this machine is set to. Landing on Afrikaans
  // because it sorts first is how the list became unusable.
  const family = (navigator.language || "en").split("-")[0];
  const home = [effectiveVoice().split("-").slice(0, 2).join("-"), navigator.language]
    .find((code) => code && seen.has(code));
  select.value = home ?? [...seen.keys()].find((code) => code.startsWith(family + "-")) ?? "";
}

function drawVoices() {
  const needle = el("voice-search").value.trim().toLowerCase();
  const locale = el("locale").value;
  const gender = el("gender").value;
  const current = effectiveVoice();
  const origin = voiceState?.origin ?? "unchosen";

  const shown = voices.filter((voice) => {
    if (locale && voice.locale !== locale) return false;
    if (gender && voice.gender !== gender) return false;
    if (!needle) return true;
    return `${voiceName(voice)} ${voice.id} ${languageOf(voice.locale)} ${voice.gender} ${voice.note}`
      .toLowerCase().includes(needle);
  });

  const list = el("voice-rows");
  list.innerHTML = "";
  el("voice-empty").hidden = shown.length > 0;
  el("voice-count").textContent = voices.length ? `${shown.length} of ${voices.length}` : "";

  for (const voice of shown) {
    const li = document.createElement("li");
    // A highlight alone could not tell "you picked this" from "this is what
    // project.yaml already said", which are different facts and looked
    // identical. Each one now says which it is, in a word (D-091) — and there
    // are four such facts, not two, which is what D-166 added: the machine's
    // fallback, and nobody having chosen at all.
    const isCurrent = current !== "" && voice.id === current;
    if (isCurrent) li.classList.add(origin === "run" ? "on" : "is-default");
    li.innerHTML =
      `<span class="v-name"></span><span class="v-mark"></span>` +
      `<span class="v-lang"></span>` +
      `<span class="v-gender"></span><span class="v-note"></span>` +
      `<span class="v-id mono"></span><button class="v-play">▶</button>`;
    li.children[0].textContent = voiceName(voice);
    li.children[1].textContent = isCurrent ? (VOICE_MARK[origin] ?? "") : "";
    li.children[2].textContent = languageOf(voice.locale);
    li.children[3].textContent = voice.gender;
    li.children[4].textContent = voice.note;
    li.children[4].title = voice.note;
    li.children[5].textContent = voice.id;
    li.children[6].title = "Hear this voice";
    li.setAttribute("aria-selected", String(isCurrent));
    li.title = isCurrent
      ? (voiceState?.detail ?? "")
      : `Use ${voiceName(voice)} for the next render`;

    li.addEventListener("click", () => chooseVoice(voice.id));
    li.children[6].addEventListener("click", (event) => {
      event.stopPropagation();
      preview(voice.id);
    });
    list.appendChild(li);
  }

  // Whatever is current is worth seeing without hunting for it.
  const marked = list.querySelector("li.on, li.is-default");
  if (marked) marked.scrollIntoView({ block: "nearest" });
}

async function chooseVoice(id) {
  chosenVoice = id || null;
  rememberChoices();
  await refreshVoice();
  drawVoiceChoice();
  drawVoices();
  // The voice is one of the things that can block Render, so choosing one has
  // to release it — and clearing one has to put it back (D-167).
  updateRender();
  // Clicking used to change nothing an operator could see: the row that was
  // already highlighted stayed highlighted, because it had been highlighted as
  // the project's default all along (D-091). And "Use the project default"
  // used to promise project.yaml even when project.yaml named nothing — the
  // answer it lands on is whatever Rust says it lands on (D-166).
  const voice = voices.find((v) => v.id === effectiveVoice());
  const named = voice ? voiceName(voice) : effectiveVoice();
  setStatus(
    chosenVoice
      ? `${named} will read every written line.`
      : voiceState?.detail ?? "",
  );
}

function drawVoiceChoice() {
  const current = effectiveVoice();
  const voice = voices.find((v) => v.id === current);

  if (voice) {
    el("chosen-name").textContent = `${voiceName(voice)} — ${languageOf(voice.locale)}`;
    el("chosen-id").textContent = `${voice.gender} · ${voice.id}`;
  } else if (current) {
    el("chosen-name").textContent = describe(current);
    el("chosen-id").textContent = current;
  } else {
    // No name to show at all: nobody chose one and the provider could not be
    // reached to say what it would use. Naming "the project's own voice" here
    // was the same false claim the tag below used to make.
    el("chosen-name").textContent = "No voice chosen";
    el("chosen-id").textContent = project?.provider || "";
  }

  // Four answers, four tags. This used to read "From project.yaml" for every
  // one of the three that are not a run override — a false statement about a
  // file, on the screen whose whole job is to say whose voice you will hear
  // (D-166). `chosen-why` carries the same fact as something to act on.
  const origin = voiceState?.origin ?? "unchosen";
  const tag = el("chosen-tag");
  tag.textContent = voiceState?.said ?? "";
  tag.className = "chosen-tag" + (origin === "run" ? " on" : "")
    + (origin === "unchosen" ? " unchosen" : "");
  el("chosen-why").textContent = voiceState?.detail ?? "";
  drawPin();

  el("voice-default").disabled = !chosenVoice;
  // The rail is what an operator reads at the moment they reach for Render, so
  // it is the one place "nobody chose this" matters most (D-166).
  el("rail-voice").textContent = voice
    ? `${voiceName(voice)} · ${languageOf(voice.locale)}`
    : current || "none chosen";
  el("rail-voice-said").textContent = voiceState?.said ?? "";
  el("go-voice").classList.toggle("unchosen", origin === "unchosen");
  el("go-voice").title = voiceState?.detail ?? "";
}

// The machine's fallback, offered where the operator is already choosing a
// voice (D-168). It lived only under Settings, one level up and behind Home,
// which is a long way from the screen where somebody has just found the voice
// they want for all ten parts of their film.
//
// A toggle, not a one-way switch: the same control that sets it is the one
// that clears it, because a setting an operator cannot find their way back out
// of is worse than no setting.
function drawPin() {
  const button = el("pin-voice");
  const voice = effectiveVoice();
  const pinned = Boolean(voiceState?.isFallback);
  button.disabled = !voice;
  button.textContent = pinned ? "\u2713 Used for every project" : "Use for every project";
  button.className = pinned ? "on" : "";
  button.title = !voice
    ? "Choose a voice first."
    : pinned
      ? `Every project on this machine that names no voice is read by ${voice}. `
        + "Click to stop."
      : `Read every project that names no voice in ${voice}. `
        + "A project with its own tts.voice still wins.";
}

async function pinVoice() {
  const voice = effectiveVoice();
  if (!voice) return;
  const pinned = Boolean(voiceState?.isFallback);
  const button = el("pin-voice");
  button.disabled = true;
  try {
    await invoke("set_default_voice", { voice: pinned ? null : voice });
    await refreshVoice();
    drawVoiceChoice();
    drawVoices();
    // Clearing it can put a project back into "nobody chose", which is a state
    // Render is held on (D-167) — so the button has to be re-asked here too.
    updateRender();
    setStatus(voiceState?.detail ?? "");
  } catch (error) {
    setStatus(String(error));
  } finally {
    drawPin();
  }
}

// An audition. It goes through the same cache and the same normalization the
// render uses, so it sounds like the film will sound and hearing it twice costs
// nothing (D-084).
async function preview(id) {
  if (!project) return;
  const voice = id || effectiveVoice() || project.voice || "default";
  const button = el("preview");
  const was = button.textContent;
  button.disabled = true;
  button.textContent = "Speaking…";
  setStatus(`Auditioning ${describe(voice)}…`);
  try {
    const path = await invoke("preview_voice", {
      root: project.root,
      provider: project.provider,
      voice,
      text: "",
    });
    const player = el("player");
    player.src = convertFileSrc(path);
    await player.play();
    setStatus(describe(voice));
  } catch (error) {
    setStatus(String(error));
  } finally {
    button.disabled = false;
    button.textContent = was;
  }
}

// One place decides whether Render can run, and the same place says why. It
// used to be three copies of the same boolean, and the reason a disabled
// button was disabled lived on the Output screen — so a project with five
// good scenes and a folder whose name ends in a space looked simply broken
// (D-089).
function renderBlocker() {
  if (!project) return "Open a project first.";
  // Before anything about the project, because a machine that cannot render
  // cannot render a perfect project either — and this one has a button on the
  // Render screen rather than a fix the operator has to go and find.
  if (ffmpegBlocker) return ffmpegBlocker;
  // A project imported from a chapter and not yet given a single picture
  // (D-193). Said plainly, rather than as "1 scene needs attention".
  if (project.scenes.length === 0 && (project.waiting ?? []).length > 0) {
    return "No scene has a picture yet. Add pictures to render.";
  }
  // Reachable since the Import chapter screen can open over an empty project:
  // "1 scene needs attention" over a project with no scenes was false.
  if (project.empty) return "Add photos or import a chapter to start.";
  if (project.has_errors) {
    const n = attention();
    return `${n} scene${n === 1 ? " needs" : "s need"} attention — see the list on Scenes.`;
  }
  if (outError) return outError;
  // Asked once, not every time (D-167). This fires only when *nobody* has
  // answered — no pick, no `tts.voice`, no fallback in Settings — so setting a
  // fallback once ends it for every project on this machine, which is the
  // reported workflow's actual fix. A project with nothing to speak is never
  // asked: the voice would change no frame of it.
  if (voiceState?.origin === "unchosen" && count("tts") > 0) {
    return "No voice is chosen, so every line would be read in whatever voice "
      + "its own script suggests. Pick one on Voice — or set a fallback in "
      + "Settings, and you will not be asked again.";
  }
  return "";
}

function updateRender() {
  const why = renderBlocker();
  const button = el("render");
  button.disabled = Boolean(why) || rendering;
  button.title = why || (outFull ? `Renders to ${outFull}` : "");
  const note = el("rail-why");
  note.textContent = why;
  note.hidden = !why;
  el("go-output").classList.toggle("bad", Boolean(outError));
}

// ------------------------------------------------------------------- output

function resetOutput() {
  el("out-dir").value = project?.output_dir ?? "";
  el("out-name").value = project?.output_name ?? "";
  chosenAspect = null;
  chosenSize = null;
  drawFormats();
  return refreshOutput();
}

// ------------------------------------------------- shape and size (D-143)

// Fetched once per window. The lists are the same for every project — what
// changes per project is which entry is the project's own, and that comes from
// `project.aspect` and `project.short_edge`.
async function loadFormats() {
  if (formats) return;
  try {
    formats = await invoke("output_formats");
  } catch (error) {
    note(String(error), true);
  }
}

// Draw both choosers, selecting this run's answer — the override if the
// operator picked one, otherwise the project's own.
function drawFormats() {
  if (!formats || !project) return;

  const aspect = chosenAspect ?? project.aspect;
  const aspects = el("out-aspect");
  aspects.innerHTML = "";
  for (const choice of formats.aspects) {
    const option = document.createElement("option");
    option.value = choice.id;
    // The ratio and what it is for, because an operator making a Short is not
    // thinking "9:16" (D-143).
    option.textContent = `${choice.id} — ${choice.description}`;
    aspects.appendChild(option);
  }
  aspects.value = aspect;

  // The project's own size may have no name — `short_edge: 900` is legal — so
  // the numbers stay available as an entry of their own rather than being
  // silently rounded to the nearest name.
  const named = formats.sizes.find((s) => s.short_edge === project.short_edge);
  const sizes = el("out-size");
  sizes.innerHTML = "";
  if (!named) {
    const option = document.createElement("option");
    option.value = String(project.short_edge);
    option.textContent = `${project.short_edge}px short edge — the project's own`;
    sizes.appendChild(option);
  }
  for (const size of formats.sizes) {
    const option = document.createElement("option");
    option.value = size.id;
    option.textContent = `${size.id} — ${size.dimensions[aspect] ?? ""}`;
    sizes.appendChild(option);
  }
  sizes.value = chosenSize ?? (named ? named.id : String(project.short_edge));

  const size = formats.sizes.find((s) => s.id === sizes.value);
  el("out-dimensions").textContent = size
    ? `${size.dimensions[aspect]} — ${size.description}`
    : project.geometry;
}

function chooseFormat() {
  if (!project) return;
  const aspect = el("out-aspect").value;
  const size = el("out-size").value;
  // Null means "the project's own", which is what makes this an override
  // rather than an edit: a run that changes nothing sends nothing (D-013).
  chosenAspect = aspect === project.aspect ? null : aspect;
  const named = formats?.sizes.find((s) => s.short_edge === project.short_edge);
  chosenSize = size === (named ? named.id : String(project.short_edge)) ? null : size;
  drawFormats();
  // The scenes grid crops its thumbnails to the shape being rendered, and the
  // subtitle preview is drawn in it.
  drawRows();
  drawPreview();
  rememberChoices();
}

// The join and the validation both happen in Rust. This only shows the answer,
// and refuses to render while the answer is a complaint.
async function refreshOutput() {
  if (!project) return;
  const token = ++outputToken;
  outDir = el("out-dir").value;
  outName = el("out-name").value;
  let full = "";
  let problem = "";
  try {
    full = await invoke("resolve_output", { dir: outDir, name: outName });
  } catch (error) {
    problem = String(error);
  }
  // A slower earlier answer must not paint over a faster later one — the same
  // rule `drawPreview` has carried since D-106.
  if (token !== outputToken) return;
  outFull = full;
  outError = problem;
  el("out-full").textContent = outFull || "—";
  el("out-problem").textContent = outError;
  el("out-problem").hidden = !outError;
  el("out-name").classList.toggle("bad", Boolean(outError));
  el("rail-output").textContent = outFull ? outFull.split(/[\\/]/).pop() : "—";
  el("go-output").title = outFull || outError;
  updateRender();
  rememberChoices();
}

async function browseOutput() {
  const chosen = await dialog({
    directory: true, multiple: false,
    title: "Choose the folder to save the film into",
  });
  if (typeof chosen !== "string") return;
  el("out-dir").value = chosen;
  await refreshOutput();
}

// The two choices are the window's, not the project's — so they live in the
// window's own storage, keyed by folder, and `project.yaml` stays an input.
function rememberChoices() {
  if (!project) return;
  try {
    localStorage.setItem(
      "choices:" + project.root,
      JSON.stringify({
        chosenVoice, outDir, outName, chosenSubtitles, chosenTheme,
        chosenAspect, chosenSize,
        subtitlePosition: el("subs-position").value,
      }),
    );
  } catch { /* storage unavailable */ }
}

function restoreChoices() {
  chosenVoice = null;
  chosenSubtitles = null;
  chosenTheme = null;
  chosenAspect = null;
  chosenSize = null;
  let dir = project.output_dir ?? "";
  let name = project.output_name ?? "";
  try {
    const saved = JSON.parse(localStorage.getItem("choices:" + project.root) ?? "{}");
    if (typeof saved.chosenVoice === "string") chosenVoice = saved.chosenVoice;
    if (typeof saved.chosenSubtitles === "boolean") chosenSubtitles = saved.chosenSubtitles;
    if (typeof saved.chosenTheme === "string") chosenTheme = saved.chosenTheme;
    if (saved.subtitlePosition === "top" || saved.subtitlePosition === "bottom") {
      el("subs-position").value = saved.subtitlePosition;
    }
    if (typeof saved.outDir === "string" && saved.outDir) dir = saved.outDir;
    if (typeof saved.outName === "string" && saved.outName) name = saved.outName;
    if (typeof saved.chosenAspect === "string") chosenAspect = saved.chosenAspect;
    if (typeof saved.chosenSize === "string") chosenSize = saved.chosenSize;
  } catch { /* nothing remembered, or storage is unavailable */ }
  el("out-dir").value = dir;
  el("out-name").value = name;
  guard(refreshOutput());
}

// ------------------------------------------------------------------ render

async function render() {
  if (!project || rendering) return;
  // Both standing answers re-asked at the moment of use, not at the moment
  // they were last drawn: the fallback voice can be changed in Settings while
  // this project is open (D-166).
  await refreshVoice();
  await refreshOutput();
  if (outError) {
    tab("output");
    setStatus(outError);
    return;
  }

  rendering = true;
  film = null;
  live = new Map();
  el("render").hidden = true;
  el("cancel").hidden = false;
  el("play").hidden = true;
  el("reveal-2").hidden = true;
  buildLive();
  el("live-note").textContent = "";
  el("bar").style.width = "0";
  el("r-voice").textContent = voiceState
    ? `${voiceState.voice || "chosen per line"} \u2014 ${voiceState.said.toLowerCase()}`
    : "\u2014";
  el("r-out").textContent = outFull;
  tab("render");

  const progress = new Channel();
  let done = 0;
  progress.onmessage = (event) => {
    if (event.kind === "planned") {
      note(`${event.scenes} scenes, ${event.jobs} at a time, ${event.audio_jobs} narrations at a time`);
      // Every render says what encodes it, so "is it using my graphics card"
      // has an answer where the operator is looking (D-199).
      note(`video encoded on ${event.encoder}`);
      // Why the pool is smaller than the machine's cores, said once, where the
      // operator is already looking (D-144).
      if (event.limited_by_memory) {
        note(`${event.per_worker} per scene at this size, so fewer run at once`);
      }
      el("progress-line").textContent = `0 of ${event.scenes}`;
      return;
    }
    if (event.kind === "memoryPressure") {
      // Arrives before any worker starts. What it warns about is a machine
      // that stops responding, and a message drawn afterwards is one nobody
      // gets to read (D-144).
      const fix =
        event.fits >= 1
          ? `Try ${event.fits} at a time.`
          : "Try a smaller output size, or close other applications.";
      note(
        `warning: ${event.jobs} scenes at once need about ${event.needed}, ` +
          `and this machine should spare about ${event.budget}. ${fix}`,
        true,
      );
      return;
    }
    if (event.kind === "warned") {
      // Everything `still validate` would have said and this screen used to
      // discard — an enlarged photograph, a recording paired with nothing.
      // Before the pool, so it is not buried under five minutes of progress.
      note(`warning: ${event.detail}`, true);
      return;
    }
    if (event.kind === "joining") {
      note(`joining ${event.segments} scenes`);
      el("progress-line").textContent = "Joining…";
      return;
    }
    const current = live.get(event.index) ?? {};
    if (event.kind === "audio") {
      live.set(event.index, { ...current, seconds: event.duration, cached: event.reused });
    } else if (event.kind === "segment") {
      done += 1;
      live.set(event.index, {
        ...current,
        seconds: event.duration,
        frames: event.frames,
        reused: event.reused,
      });
      el("bar").style.width = `${(done / project.scenes.length) * 100}%`;
      el("progress-line").textContent = `${done} of ${project.scenes.length}`;
    } else if (event.kind === "failed") {
      live.set(event.index, { ...current, failed: event.detail });
      note(`${event.id} failed — ${event.detail}`, true);
    }
    updateLive(event.index);
    markRow(event.index);
  };

  try {
    film = await invoke("render_project", {
      request: {
        path: project.root,
        jobs: null,
        audioJobs: null,
        force: false,
        // The override this run asked for — the Voice screen's pick, or the
        // machine's fallback when the project names none. Never written back
        // to project.yaml (D-013, D-092). Null hands the question to the
        // renderer, which reads project.yaml and then D-158's script rule.
        // The same object the Render summary above was drawn from (D-166).
        voice: voiceState?.overrideForRender ?? null,
        outDir: el("out-dir").value,
        outName: el("out-name").value,
        // D-106, and the same override rule as the voice above it: null means
        // "whatever project.yaml says", and nothing here writes to that file.
        subtitles: chosenSubtitles,
        subtitleTheme: chosenTheme,
        // The position box drove only the preview until a real project showed
        // why that is the same defect as a highlight that means nothing
        // (D-091): the operator moves the caption off their artwork's own
        // lettering, renders, and it comes back exactly where it was.
        subtitlePosition: el("subs-position").value,
        // D-143, and the same override rule again: null means "whatever
        // project.yaml says". The window sends what its boxes say, which is
        // D-106's own lesson about the position box — a control that only
        // changed the preview.
        aspect: chosenAspect,
        resolution: chosenSize,
      },
      onProgress: progress,
    });
    el("bar").style.width = "100%";
    el("progress-line").textContent =
      `Done — ${film.scenes} scenes, ${film.duration.toFixed(1)}s, ` +
      `${film.reused_segments} reused, ${film.reused_audio} narrations from cache.`;
    el("r-out").textContent = film.path;
    el("play").hidden = false;
    el("reveal-2").hidden = false;
    setStatus(film.path);
  } catch (error) {
    el("progress-line").textContent = String(error);
    note(String(error), true);
    setStatus("Stopped. The film file was not written.");
  } finally {
    rendering = false;
    el("render").hidden = false;
    el("cancel").hidden = true;
    updateRender();
  }
}

function markRow(index) {
  const row = el("scene-" + index);
  if (!row) return;
  const state = live.get(index);
  row.classList.toggle("running", Boolean(state?.seconds) && !state?.frames);
  row.classList.toggle("done", Boolean(state?.frames));
  row.classList.toggle("reused", Boolean(state?.reused));
  const scene = project.scenes[index];
  if (scene) row.querySelector(".c-resolved").innerHTML = resolved(scene);
}

// The pool renders several scenes at once and they finish in whatever order
// the workers free up (D-076). The film is still joined in *scene* order:
// `pool::run` returns results indexed by input position, pinned by
// `results_come_back_in_input_order`, which reverse-sleeps so completion order
// is the opposite of input order. A completion-ordered log made a correct film
// look scrambled, so this list is the film's own order — every scene present
// from the start, each row updating in place (D-091).
function buildLive() {
  const list = el("live");
  list.innerHTML = "";
  project.scenes.forEach((scene, index) => {
    const li = document.createElement("li");
    li.id = "live-" + index;
    li.className = "waiting";
    li.innerHTML =
      `<span class="l-id mono"></span><span class="l-state"></span>` +
      `<span class="l-detail mono"></span>`;
    li.children[0].textContent = scene.id;
    li.children[1].textContent = "waiting";
    list.appendChild(li);
  });
}

function updateLive(index) {
  const li = el("live-" + index);
  if (!li) return;
  const s = live.get(index) ?? {};
  let state = "waiting";
  let cls = "waiting";
  let detail = "";

  if (s.failed) {
    state = "failed";
    cls = "bad";
    detail = s.failed;
  } else if (s.frames) {
    state = s.reused ? "reused" : "rendered";
    cls = "done";
    detail = `${s.frames}f · ${s.seconds.toFixed(3)}s`;
  } else if (s.seconds !== undefined) {
    state = "narration ready";
    cls = "running";
    detail = `${s.seconds.toFixed(3)}s${s.cached ? " · cached" : ""}`;
  }

  li.className = cls;
  li.children[1].textContent = state;
  li.children[2].textContent = detail;
}

// Everything that is not about one scene — the plan, the join, a failure.
function note(text, bad = false) {
  const line = el("live-note");
  if (!line) return;
  line.textContent = text;
  line.classList.toggle("bad", bad);
}

async function cancel() {
  setStatus("Stopping — letting the current scene finish its frame…");
  await invoke("cancel_render");
}

// ----------------------------------------------------------------- dropping

// Tauri reports a drop as a window event carrying real paths, which is what
// makes this work at all: a browser `DataTransfer` would give us file handles
// the Rust side cannot open. If the event API is unavailable the buttons still
// do everything — nothing here is the only way to reach a feature.
async function watchDrops() {
  const events = window.__TAURI__?.event;
  if (!events) return;
  const over = el("drop");
  const follow = (position) => {
    if (position) edgeScroll(position.y / dragScale());
    const tr = rowAt(position);
    showDropTarget(tr);
    // Over a row the row says what will happen; anywhere else the whole
    // window does, because a drop there adds to the end (D-194).
    over.hidden = Boolean(tr);
  };
  await events.listen("tauri://drag-enter", (event) => {
    dragPaths = event.payload?.paths ?? [];
    follow(event.payload?.position);
  });
  await events.listen("tauri://drag-over", (event) => follow(event.payload?.position));
  await events.listen("tauri://drag-leave", () => { dragPaths = []; endDrag(); });
  await events.listen("tauri://drag-drop", async (event) => {
    const paths = event.payload?.paths ?? [];
    const tr = rowAt(event.payload?.position);
    dragPaths = [];
    endDrag();
    if (paths.length === 0) return;
    // A folder dropped with no project open is a project being opened, which is
    // almost always what was meant.
    if (!project && paths.length === 1) {
      await load(paths[0]);
      return;
    }
    if (!project) {
      setStatus("Open a project first, then drop your photos in.");
      return;
    }
    // On the Import chapter screen a text file is the chapter, not a scene.
    if (!el("pane-chapter").hidden) {
      if (paths.length === 1 && /\.(txt|md)$/i.test(paths[0])) await readChapter(paths[0]);
      else setStatus("Drop one .txt file here to import it as the chapter.");
      return;
    }
    // Onto one scene: one picture on exactly that scene, or several on it and
    // the scenes after it that are still waiting for one.
    if (tr) {
      if (rendering) return;
      if (pictureBusy) {
        setStatus("Nothing was changed — the last picture is still being saved. Drop it again.");
        logUsage("drop refused", "still saving the last picture");
        return;
      }
      if (paths.some((path) => !isPicture(path))) {
        setStatus("Nothing was changed — only pictures can be dropped on a scene.");
        logUsage("drop refused", `${paths.length} files, not all pictures, on scene ${tr.dataset.scene}`);
        return;
      }
      if (paths.length === 1) await putPicture(tr.dataset.scene, paths[0]);
      else await pictureChange("fill_pictures", { scene: tr.dataset.scene, paths });
      return;
    }
    logUsage("drop at the end", `${paths.length} files`);
    await addMedia(paths);
  });
}

// ---------------------------------------------------------------- pictures
//
// D-194. A picture goes on the row it was put on: by clicking that row's +,
// by dropping a file onto that row, or from the menu on a picture already
// there — Replace, Move to scene, Remove. Nothing is deleted; Rust moves a
// replaced or removed picture to removed/ and says so.

// One row in the activity log for something done on a screen that Rust would
// not otherwise hear about (D-194). Counts and scene numbers only — never the
// words of a narration — and never allowed to get in the way.
const logUsage = (event, detail = "") => {
  invoke("usage", { event, detail: String(detail) }).catch(() => {});
};

const PICTURE = ["jpg", "jpeg", "png", "webp", "heic", "heif", "tif", "tiff", "bmp"];
const isPicture = (path) => PICTURE.includes((path.split(".").pop() || "").toLowerCase());

function drawPictureControl(cell, scene) {
  const button = cell.querySelector("button");
  if (!button || !project.convention) return;
  button.disabled = rendering;
  if (scene.waiting) {
    button.title = `Choose a picture for scene ${scene.id}`;
    button.setAttribute("aria-label", `Choose a picture for scene ${scene.id}`);
    button.addEventListener("click", () => guard(choosePicture(scene.id)));
  } else {
    button.title = `Scene ${scene.id}'s picture — click to replace, move or remove; drag it onto another scene to move it`;
    button.setAttribute("aria-label", `Picture options for scene ${scene.id}`);
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      // A press that turned into a drag is not a click.
      if (movedPicture) { movedPicture = false; return; }
      openPictureMenu(button, scene);
    });
    button.addEventListener("pointerdown", (event) => startPictureDrag(event, button, scene));
  }
}

// Dragging a picture from one scene's row to another's (D-194).
//
// Done with pointer events inside the page rather than the system's drag and
// drop, which Tauri takes for files (see the drop handler). A press that moves
// less than a few pixels is a click and opens the menu; one that moves further
// is a drag, and the row under the pointer says what letting go will do — the
// same label, the same rule as `move_picture`: onto a scene with no picture it
// moves; onto one with a picture the two swap.
let movedPicture = false;

function startPictureDrag(event, button, scene) {
  // Every press starts as a click. A drag that ended on another row produces
  // no click on this button to consume the flag, and the next real click on a
  // picture was swallowed — found by the drag stress test.
  movedPicture = false;
  if (rendering || pictureBusy || event.button !== 0) return;
  const start = { x: event.clientX, y: event.clientY };
  let dragging = false;
  let target = null;
  const label = el("drop-label");

  let lastY = start.y;
  let lastX = start.x;
  const scroller = setInterval(() => {
    if (dragging && edgeScroll(lastY)) move({ clientX: lastX, clientY: lastY, buttons: 1, fromTimer: true });
  }, 40);

  const move = (e) => {
    lastY = e.clientY;
    lastX = e.clientX;
    // No button held means the release was missed — a click that switched
    // windows, a busy machine. A drag that outlived its button would turn the
    // operator's next ordinary click into a move they never made, so it ends
    // here, having done nothing.
    if ((e.buttons & 1) === 0) {
      if (dragging) logUsage("picture drag cancelled", "button release was missed");
      cancelled();
      return;
    }
    if (!dragging && Math.hypot(e.clientX - start.x, e.clientY - start.y) < 6) return;
    if (!dragging) {
      dragging = true;
      closePictureMenu();
      document.body.classList.add("moving-picture");
    }
    if (!e.fromTimer) edgeScroll(e.clientY);
    const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest?.("#rows tr[data-scene]");
    target = hit && hit.dataset.scene !== scene.id ? hit : null;
    for (const old of el("rows").querySelectorAll("tr.drop-target")) {
      if (old !== target) old.classList.remove("drop-target");
    }
    if (target) {
      target.classList.add("drop-target");
      const swapping = !target.querySelector(".thumb-missing");
      label.textContent = swapping
        ? `Swap pictures with scene ${target.dataset.scene}`
        : `Move picture to scene ${target.dataset.scene}`;
      label.classList.remove("bad");
    } else {
      label.textContent = `Moving scene ${scene.id}'s picture — let go on another scene`;
      label.classList.add("bad");
    }
    label.hidden = false;
    label.style.left = e.clientX + 14 + "px";
    label.style.top = e.clientY + 14 + "px";
  };

  const finish = () => {
    clearInterval(scroller);
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", finish);
    document.removeEventListener("pointercancel", cancelled);
    window.removeEventListener("blur", cancelled);
    try { button.releasePointerCapture(event.pointerId); } catch { /* already released */ }
    document.body.classList.remove("moving-picture");
    showDropTarget(null);
    if (!dragging || aborted) return;
    movedPicture = true;
    if (target) {
      guard(pictureChange("move_picture", { scene: scene.id, to: target.dataset.scene }));
    } else {
      setStatus("Nothing was changed — let go on another scene's row to move a picture.");
      logUsage("picture drag let go on no scene", `from scene ${scene.id}`);
    }
  };
  let aborted = false;
  const cancelled = () => { target = null; aborted = true; finish(); };

  // Captured, so the release reaches this drag wherever the pointer is.
  try { button.setPointerCapture(event.pointerId); } catch { /* not supported */ }
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", finish);
  document.addEventListener("pointercancel", cancelled);
  window.addEventListener("blur", cancelled);
}

async function choosePicture(id) {
  const chosen = await dialog({
    multiple: false,
    title: `Choose the picture for scene ${id}`,
    filters: [{ name: "Pictures", extensions: PICTURE }],
  });
  if (typeof chosen === "string") await putPicture(id, chosen);
  else logUsage("picture chooser closed without a file", `scene ${id}`);
}

// One picture change at a time (D-194). On a slow machine a second drop or an
// impatient second click arrives while the first is still being written, and
// it would land on a grid that is about to be redrawn under it. While this is
// set every picture gesture is ignored, and the status line says why.
let pictureBusy = false;

async function pictureChange(command, args) {
  if (!project || rendering) return;
  if (pictureBusy) {
    setStatus("Still saving the last change — try again in a moment.");
    logUsage("ignored while saving", command);
    return;
  }
  pictureBusy = true;
  closePictureMenu();
  try {
    setStatus("Working…");
    const said = await invoke(command, { root: project.root, ...args });
    await load(project.root);
    sayWithUndo(said);
    flashRow(args.to ?? args.scene);
  } catch (error) {
    setStatus(String(error));
  } finally {
    pictureBusy = false;
  }
}

// What a picture change did, with the way to take it back next to it.
function sayWithUndo(said) {
  const status = el("status");
  status.textContent = said + " ";
  const button = document.createElement("button");
  button.className = "link undo";
  button.textContent = "Undo (⌘Z)";
  button.addEventListener("click", () => { logUsage("undo", "button"); guard(undoPicture()); });
  status.appendChild(button);
}

async function undoPicture() {
  if (!project || rendering || pictureBusy) return;
  pictureBusy = true;
  try {
    const said = await invoke("undo_picture", { root: project.root });
    await load(project.root);
    sayWithUndo(said);
  } catch (error) {
    setStatus(String(error));
  } finally {
    pictureBusy = false;
  }
}

// ⌘Z / Ctrl+Z undoes a picture change — but never while typing, where it is
// the text box's own undo.
document.addEventListener("keydown", (event) => {
  if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "z" || event.shiftKey) return;
  const typing = event.target.closest?.("input, textarea, [contenteditable]");
  if (typing || el("app").hidden) return;
  event.preventDefault();
  logUsage("undo", "keyboard");
  guard(undoPicture());
});

const putPicture = (scene, path) => pictureChange("set_picture", { scene, path });

// A brief mark on the row that just changed, so a fast operator sees where it
// landed without reading the status line. Matched by number, so `24` from the
// move box finds row `024`.
function flashRow(id) {
  const wanted = Number(id);
  const tr = [...el("rows").querySelectorAll("tr[data-scene]")]
    .find((row) => Number(row.dataset.scene) === wanted);
  if (!tr) return;
  tr.classList.add("landed");
  setTimeout(() => tr.classList.remove("landed"), 1200);
}

let pictureMenu = null;

function closePictureMenu() {
  pictureMenu?.remove();
  pictureMenu = null;
}

function openPictureMenu(anchor, scene) {
  closePictureMenu();
  const menu = document.createElement("div");
  menu.className = "picture-menu";
  menu.setAttribute("role", "menu");
  menu.innerHTML = `
    <button data-do="replace">Replace…</button>
    <form class="move-row"><label>Move to scene <input type="text" inputmode="numeric" size="5" /></label>
      <button type="submit">Move</button></form>
    <button data-do="remove" class="danger-text">Remove picture</button>`;
  menu.querySelector('[data-do="replace"]').addEventListener("click", () => {
    closePictureMenu();
    guard(choosePicture(scene.id));
  });
  menu.querySelector('[data-do="remove"]').addEventListener("click", () =>
    guard(pictureChange("remove_picture", { scene: scene.id })));
  const input = menu.querySelector("input");
  input.setAttribute("aria-label", "Scene number to move the picture to");
  menu.querySelector("form").addEventListener("submit", (event) => {
    event.preventDefault();
    const to = input.value.trim();
    if (!/^\d+$/.test(to)) {
      setStatus("Type the number of the scene to move it to.");
      input.focus();
      return;
    }
    guard(pictureChange("move_picture", { scene: scene.id, to }));
  });
  menu.addEventListener("keydown", (event) => { if (event.key === "Escape") closePictureMenu(); });
  document.body.appendChild(menu);
  const rect = anchor.getBoundingClientRect();
  const top = Math.min(rect.bottom + 4, innerHeight - menu.offsetHeight - 8);
  menu.style.left = Math.max(8, rect.left) + "px";
  menu.style.top = Math.max(8, top) + "px";
  pictureMenu = menu;
  input.focus();
}

document.addEventListener("click", (event) => {
  if (pictureMenu && !pictureMenu.contains(event.target)) closePictureMenu();
});

// ------------------------------------------------------- drag onto a scene
//
// The webview never sees a file drag — Tauri takes it so a drop carries real
// paths — so "which row is under the pointer" is worked out here from the
// position Tauri reports.
//
// **That position is not in the same units on the two platforms**, whatever
// its type says (Tauri calls it `PhysicalPosition` on both). Read in the
// pinned wry 0.55.1, not assumed:
//
// - macOS: `draggingLocation()` flipped to the top left — window **points**,
//   which is what the page measures in already. Dividing by the Retina ratio
//   halved it, so the row lit up was the one at half the pointer's height —
//   reported by the author as "it's selecting randomly".
// - Windows: `ScreenToClient` — **physical** pixels of the client area, which
//   is the page (D-160 keeps the native title bar outside it), so there it
//   divides.

let dragPaths = [];

// Scroll the scenes list while something is held near its top or bottom edge
// (D-194), so a row that is not on screen can still be dropped on — at 580
// scenes most of them are not. Faster the closer to the edge. Returns whether
// it scrolled.
function edgeScroll(clientY) {
  const wrap = el("rows").closest(".grid-wrap");
  if (!wrap || el("pane-scenes").hidden) return false;
  const box = wrap.getBoundingClientRect();
  const zone = 48;
  const top = clientY - box.top;
  const bottom = box.bottom - clientY;
  let step = 0;
  if (top >= 0 && top < zone) step = -Math.ceil((zone - top) / 3);
  else if (bottom >= 0 && bottom < zone) step = Math.ceil((zone - bottom) / 3);
  if (!step) return false;
  const before = wrap.scrollTop;
  wrap.scrollTop += step;
  return wrap.scrollTop !== before;
}

const dragScale = () =>
  document.documentElement.dataset.os === "windows" ? window.devicePixelRatio || 1 : 1;

function rowAt(position) {
  if (!position || el("app").hidden || el("pane-scenes").hidden || !project?.convention) return null;
  const ratio = dragScale();
  const hit = document.elementFromPoint(position.x / ratio, position.y / ratio);
  return hit?.closest?.("#rows tr[data-scene]") ?? null;
}

function showDropTarget(tr) {
  for (const old of el("rows").querySelectorAll("tr.drop-target")) {
    if (old !== tr) old.classList.remove("drop-target");
  }
  const label = el("drop-label");
  if (!tr) {
    label.hidden = true;
    return;
  }
  tr.classList.add("drop-target");
  const plan = dropPlan(tr.dataset.scene);
  label.textContent = plan.says;
  label.classList.toggle("bad", !plan.ok);
  label.classList.toggle("warn", plan.ok && Boolean(plan.replaces));
  const rect = tr.getBoundingClientRect();
  label.hidden = false;
  label.style.top = Math.max(4, rect.top - label.offsetHeight - 2) + "px";
  label.style.left = rect.left + 12 + "px";
}

// What dropping the held files on scene `id` would do — said on the label
// while dragging, and the same rule Rust applies (`picture::set` for one,
// `picture::fill` for several), so what the label promises is what happens.
function dropPlan(id) {
  const n = dragPaths.length;
  if (n === 0) return { ok: false, says: "" };
  if (dragPaths.some((path) => !isPicture(path))) {
    return { ok: false, says: n === 1 ? "That is not a picture" : "Not all of these are pictures" };
  }
  const rows = allRows();
  const at = rows.findIndex((row) => row.id === id);
  if (n === 1) {
    return rows[at]?.waiting
      ? { ok: true, says: `Put on scene ${id}` }
      : { ok: true, replaces: true, says: `Replace scene ${id}'s picture — the old one is kept` };
  }
  const targets = rows.slice(at).filter((row) => row.waiting).slice(0, n);
  if (targets.length < n) {
    return { ok: false, says: `${n} pictures — only ${targets.length} scenes from ${id} on need one` };
  }
  return { ok: true, says: `${n} pictures → scenes ${targets[0].id}–${targets[n - 1].id}` };
}

function endDrag() {
  showDropTarget(null);
  el("drop").hidden = true;
}

// --------------------------------------------------------- import chapter
//
// D-193. Two steps on one screen: the text, then the cuts. The cutting and the
// estimates are Rust's (`chapter_cut`, `chapter_review`) — this page only holds
// the list the operator is editing, and nothing reaches the project until
// "Add … scenes".
//
// The list edits like text, because it is used at speed and repetitively:
// Enter splits a cut where the cursor is, Backspace at the start of a cut
// joins it to the one above, Delete at the end joins the one below, and the
// arrow keys walk off the top or bottom of a cut into its neighbour.

let pieces = [];
let reviewToken = 0;

// The way to cut opens on this machine's choice in Settings (D-194), and can
// be changed for this one chapter here without changing Settings.
async function chooseCutDefault() {
  try {
    const view = await invoke("app_settings");
    el("chapter-by").value = view.settings.chapter_cut || "time";
  } catch {
    el("chapter-by").value = "time";
  }
  showSeconds();
}

function showSeconds() {
  el("chapter-seconds").hidden = el("chapter-by").value !== "time";
}

async function setCutDefault(by) {
  const said = el("app-chapter-cut-said");
  try {
    await invoke("set_chapter_cut", { by });
    said.textContent = "Saved. Import chapter will cut this way.";
  } catch (error) {
    said.textContent = String(error);
  }
}

async function openChapter() {
  if (!project || rendering) return;
  if (pieces.length === 0) await chooseCutDefault();
  if (el("app").hidden) {
    show("app");
    await refreshVoice();
    draw();
  }
  for (const button of el("tabs").children) button.classList.remove("on");
  for (const pane of TABS) el("pane-" + pane).hidden = true;
  el("pane-chapter").hidden = false;
  showChapterStep(pieces.length > 0 ? "review" : "write");
  if (pieces.length === 0) el("chapter-text").focus();
}

function closeChapter() {
  pieces = [];
  el("chapter-text").value = "";
  if (project?.empty) { el("counts").textContent = ""; show("fill"); return; }
  tab("scenes");
}

function showChapterStep(step) {
  el("chapter-write").hidden = step !== "write";
  el("chapter-review").hidden = step !== "review";
}

async function chooseChapter() {
  const chosen = await dialog({
    multiple: false,
    title: "Choose the chapter",
    filters: [{ name: "Text", extensions: ["txt", "md"] }],
  });
  if (typeof chosen === "string") await readChapter(chosen);
}

async function readChapter(path) {
  try {
    el("chapter-text").value = await invoke("chapter_read", { path });
    showChapterStep("write");
    el("chapter-text").focus();
    setStatus("");
  } catch (error) {
    setStatus(String(error));
  }
}

async function cutChapter() {
  const text = el("chapter-text").value;
  if (!text.trim()) {
    setStatus("Paste the chapter first.");
    el("chapter-text").focus();
    return;
  }
  const view = await invoke("chapter_cut", {
    text,
    by: el("chapter-by").value,
    min: Number(el("chapter-min").value) || 3,
    max: Number(el("chapter-max").value) || 5,
  });
  pieces = view.cuts.map((c) => c.text);
  logUsage("chapter cut", `by ${el("chapter-by").value}, ${pieces.length} cuts, ${text.length} characters`);
  showChapterStep("review");
  drawCuts(view, 0, 0);
}

// Redraw the whole list, then put the cursor at `at` in cut `index`.
function drawCuts(view, index, at) {
  const list = el("chapter-cuts");
  list.innerHTML = "";
  view.cuts.forEach((cut, i) => {
    const li = document.createElement("li");
    li.innerHTML = `<span class="n"></span><textarea rows="1" spellcheck="false"></textarea>
      <span class="s"></span><button class="join link" title="Join with the next scene">⤓</button>`;
    li.querySelector(".n").textContent = String(i + 1);
    const box = li.querySelector("textarea");
    box.value = cut.text;
    box.setAttribute("aria-label", `Scene ${i + 1}`);
    box.addEventListener("input", () => { pieces[i] = box.value; fit(box); review(); });
    box.addEventListener("keydown", (event) => cutKeys(event, box, i));
    const join = li.querySelector(".join");
    join.disabled = i === view.cuts.length - 1;
    join.setAttribute("aria-label", `Join scene ${i + 1} with the next`);
    join.addEventListener("click", () => joinCuts(i));
    list.appendChild(li);
  });
  drawEstimates(view);
  fitAll(list);
  const target = list.children[Math.min(index, list.children.length - 1)]?.querySelector("textarea");
  if (target) {
    target.focus();
    target.setSelectionRange(at, at);
  }
}

function fit(box) {
  box.style.height = "auto";
  box.style.height = box.scrollHeight + 2 + "px";
}

// Every box at once: all the writes, then all the reads, then all the writes.
// `fit` one box at a time makes the page lay itself out once per box — five
// hundred times for a long chapter, on every split and join.
function fitAll(list) {
  const boxes = [...list.querySelectorAll("textarea")];
  for (const box of boxes) box.style.height = "auto";
  const heights = boxes.map((box) => box.scrollHeight);
  boxes.forEach((box, i) => { box.style.height = heights[i] + 2 + "px"; });
}

// The seconds and the total, from Rust, without rebuilding the list — so the
// cut being typed in keeps its cursor.
function drawEstimates(view) {
  const max = Number(el("chapter-max").value) || 5;
  const rows = el("chapter-cuts").children;
  view.cuts.forEach((cut, i) => {
    const s = rows[i]?.querySelector(".s");
    if (!s) return;
    s.textContent = cut.seconds.toFixed(1) + "s";
    // Long enough to be worth a look, not an error: the story decides.
    s.classList.toggle("long", cut.seconds > max + 2.5);
  });
  el("chapter-summary").textContent = view.summary;
  const n = pieces.filter((p) => p.trim()).length;
  el("chapter-add").textContent = `Add ${n} scene${n === 1 ? "" : "s"}`;
  el("chapter-add").disabled = n === 0;
}

const review = onFrame(async () => {
  const token = ++reviewToken;
  const view = await invoke("chapter_review", { pieces });
  if (token === reviewToken) drawEstimates(view);
});

async function restructure(index, at) {
  const view = await invoke("chapter_review", { pieces });
  drawCuts(view, index, at);
}

function joinCuts(i) {
  if (i + 1 >= pieces.length) return;
  logUsage("chapter cuts joined", `cuts ${i + 1} and ${i + 2} of ${pieces.length}`);
  const at = pieces[i].trimEnd().length;
  pieces.splice(i, 2, (pieces[i].trimEnd() + " " + pieces[i + 1].trimStart()).trim());
  guard(restructure(i, at + 1));
}

function cutKeys(event, box, i) {
  const start = box.selectionStart;
  const end = box.selectionEnd;
  const collapsed = start === end;
  const length = box.value.length;

  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    const before = box.value.slice(0, start).trim();
    const after = box.value.slice(end).trim();
    // At either end there is nothing to split — move on instead, which is what
    // someone pressing Enter to confirm a cut means.
    if (!before || !after) {
      focusCut(i + 1, 0);
      return;
    }
    pieces.splice(i, 1, before, after);
    logUsage("chapter cut split", `cut ${i + 1} of ${pieces.length - 1}`);
    guard(restructure(i + 1, 0));
    return;
  }
  if (event.key === "Backspace" && collapsed && start === 0 && i > 0) {
    event.preventDefault();
    joinCuts(i - 1);
    return;
  }
  if (event.key === "Delete" && collapsed && start === length && i + 1 < pieces.length) {
    event.preventDefault();
    joinCuts(i);
    return;
  }
  if (event.key === "ArrowUp" && collapsed && !box.value.slice(0, start).includes("\n")
      && start <= firstLineEnd(box) && i > 0) {
    event.preventDefault();
    focusCut(i - 1, null);
    return;
  }
  if (event.key === "ArrowDown" && collapsed && start >= lastLineStart(box) && i + 1 < pieces.length) {
    event.preventDefault();
    focusCut(i + 1, 0);
  }
}

// A textarea wraps, so "on the first line" is judged by where the caret sits
// against the box's own width — approximated by the text length that fits one
// visual line. At the ends it is exact; in the middle an arrow moves within
// the cut, which is what an arrow in a paragraph does.
function firstLineEnd(box) { return box.value.length === 0 ? 0 : Math.min(box.value.length, visualLine(box)); }
function lastLineStart(box) { return Math.max(0, box.value.length - visualLine(box)); }
function visualLine(box) {
  const lines = Math.max(1, Math.round(box.scrollHeight / parseFloat(getComputedStyle(box).lineHeight)));
  return Math.ceil(box.value.length / lines);
}

function focusCut(index, at) {
  const box = el("chapter-cuts").children[index]?.querySelector("textarea");
  if (!box) return;
  box.focus();
  const where = at ?? box.value.length;
  box.setSelectionRange(where, where);
}

async function addChapter() {
  if (!project || rendering) return;
  const button = el("chapter-add");
  button.disabled = true;
  try {
    const done = await invoke("import_chapter", { root: project.root, pieces });
    pieces = [];
    el("chapter-text").value = "";
    await load(project.root);
    filter = "waiting";
    tab("scenes");
    drawChips();
    drawRows();
    drawStatus();
    // Land on the first scene just added — that is where the work starts, and
    // in a project that already had waiting scenes it is not the top.
    document.getElementById("waiting-" + done.first)?.scrollIntoView({ block: "start" });
    setStatus(done.summary);
  } catch (error) {
    setStatus(String(error));
    button.disabled = false;
  }
}

// ------------------------------------------------------------------ plumbing

const guard = (promise) => Promise.resolve(promise).catch((error) => setStatus(String(error)));

// Run `fn` at most once per animation frame, however often this is called
// (D-185).
//
// Every `input` handler below fires on each keystroke, and each one either
// rebuilds a list or crosses the process boundary: `drawRows` empties the
// scenes table and builds it again — about 8,500 elements at 500 scenes —
// `drawVoices` does the same to the provider's whole catalogue, and
// `drawPreview` and `refreshOutput` each ask Rust. A fast typist outruns all
// four.
//
// The first call in a frame schedules the work and the rest are dropped.
//
// **Cancel-and-reschedule is the same thing here, and that was measured
// rather than argued.** The obvious worry about a debounce — that it
// postpones the redraw for as long as somebody keeps typing — is true of a
// `setTimeout` debounce and **not** of this one: `cancelAnimationFrame`
// followed by `requestAnimationFrame` inside one frame still runs on the
// next, so both forms redraw exactly once per frame while the keys are going
// down. Driven through this file in node, twelve keystrokes over twelve
// frames gave twelve redraws either way, and twelve inside one frame gave
// one. This form is kept because it holds no handle and cannot cancel a
// callback that has already begun, not because it behaves differently.
//
// Nothing stale can be drawn, and that is why no token is needed here: every
// `fn` reads the field's **current** value when it runs, not a value captured
// when it was scheduled. What is dropped is intermediate states nobody sees.
// The two handlers that cross the process boundary carry a token as well,
// because an answer can arrive after a later question.
function onFrame(fn) {
  let queued = false;
  return () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      fn();
    });
  };
}

el("home").addEventListener("click", goHome);
el("rail-home").addEventListener("click", goHome);
el("fill-back").addEventListener("click", goHome);
el("settings-open").addEventListener("click", () => guard(openSettings()));
el("settings-back").addEventListener("click", goHome);
el("app-voice").addEventListener("change", (e) => guard(setFallbackVoice(e.target.value)));
el("app-voice-clear").addEventListener("click", () => guard(setFallbackVoice("")));
el("activity-open").addEventListener("click", () => guard(openActivityLog(false)));
el("activity-reveal").addEventListener("click", () => guard(openActivityLog(true)));
el("new-project").addEventListener("click", newProject);
el("open-project").addEventListener("click", openProject);
el("choose-media").addEventListener("click", chooseMedia);
el("fill-chapter").addEventListener("click", () => guard(openChapter()));
el("import-chapter").addEventListener("click", () => guard(openChapter()));
el("chapter-file").addEventListener("click", () => guard(chooseChapter()));
el("chapter-cancel").addEventListener("click", closeChapter);
el("chapter-cut").addEventListener("click", () => guard(cutChapter()));
el("chapter-back").addEventListener("click", () => {
  // The text box still holds what was cut; hand-made edits to the cuts are
  // not carried back into it, so say so rather than lose them silently.
  showChapterStep("write");
  setStatus("Cutting again replaces the changes you made to the list.");
  el("chapter-text").focus();
});
el("chapter-add").addEventListener("click", () => guard(addChapter()));
el("chapter-by").addEventListener("change", showSeconds);
el("app-chapter-cut").addEventListener("change", (e) => guard(setCutDefault(e.target.value)));
el("app-graphics").addEventListener("change", (e) => guard(setGraphics(e.target.value)));
el("chapter-text").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    guard(cutChapter());
  }
});
el("add").addEventListener("click", chooseMedia);
el("render").addEventListener("click", render);
el("cancel").addEventListener("click", cancel);
el("recheck").addEventListener("click", () => project && load(project.root, true));
el("search").addEventListener("input", onFrame(() => { drawRows(); drawStatus(); }));
el("play").addEventListener("click", () => guard(invoke("open_film")));
el("reveal").addEventListener("click", () => guard(invoke("reveal_project")));
el("reveal-2").addEventListener("click", () => guard(invoke("reveal_project")));

el("preview").addEventListener("click", () => preview(null));
el("pin-voice").addEventListener("click", () => guard(pinVoice()));
el("voice-default").addEventListener("click", () => chooseVoice(null));
el("voice-search").addEventListener("input", onFrame(drawVoices));
el("subs-default").addEventListener("click", resetSubtitles);
el("subs-position").addEventListener("change", () => { drawPreview(); rememberChoices(); });
el("subs-text").addEventListener("input", onFrame(drawPreview));
el("locale").addEventListener("change", drawVoices);
el("gender").addEventListener("change", drawVoices);

el("out-name").addEventListener("input", onFrame(() => guard(refreshOutput())));
el("out-dir").addEventListener("input", onFrame(() => guard(refreshOutput())));
el("out-browse").addEventListener("click", () => guard(browseOutput()));
el("out-default").addEventListener("click", () => guard(resetOutput()));
el("out-aspect").addEventListener("change", chooseFormat);
el("out-size").addEventListener("change", chooseFormat);

for (const button of [...el("tabs").children, el("go-voice"), el("go-output")]) {
  button.addEventListener("click", () => tab(button.dataset.tab));
}

// --------------------------------------------------------------- subtitles

// D-106. The same shape as the Voice screen: a list, a selection that says so
// in words rather than by highlight alone (D-091), and a real preview.

async function loadThemes() {
  if (!themesLoaded) {
    try {
      themes = await invoke("subtitle_themes");
      themesLoaded = true;
    } catch (error) {
      note(String(error), true);
      return;
    }
  }
  drawThemes();
}

const subtitlesOn = () =>
  chosenSubtitles === null ? Boolean(project && project.subtitles) : chosenSubtitles;

const effectiveTheme = () =>
  chosenTheme || (project && project.subtitle_theme) || "classic";

function drawThemes() {
  if (!project) return;
  const on = subtitlesOn();
  const current = effectiveTheme();

  el("subs-state").textContent = on ? `Subtitles on — ${current}` : "No subtitles";

  // How many scenes actually have words. A subtitle setting that silently does
  // nothing for half the film is worth saying out loud before the render.
  const withWords = project.scenes.filter((scene) => scene.caption).length;
  const total = project.scenes.length;
  el("subs-coverage").textContent = !on
    ? "Nothing is burned into the picture."
    : withWords === total
      ? `All ${total} scenes have words to show.`
      : `${withWords} of ${total} scenes have words — the rest render without a caption.`;

  // D-091: say which state this is, rather than leaving a highlight to imply it.
  const mine = chosenSubtitles !== null || chosenTheme !== null;
  el("subs-tag").textContent = mine ? "Your choice, this run" : "From project.yaml";

  el("theme-rows").innerHTML = "";

  // "No subtitles" is a row in the same list, not a switch beside it. Off is a
  // real choice — and on this screen it is the *usual* one, since D-106 makes
  // subtitles opt-in — so it belongs among the things you can pick rather than
  // as a second control that disagrees with the list underneath it.
  const none = document.createElement("li");
  none.className = on ? "" : "on";
  none.innerHTML = `<div class="t-name"></div><div class="t-desc"></div><span class="t-mark"></span>`;
  none.querySelector(".t-name").textContent = "No subtitles";
  none.querySelector(".t-desc").textContent =
    "Leave the picture alone. Nothing is burned in, and the film is the film you would get without this screen.";
  none.querySelector(".t-mark").textContent = on ? "" : "\u2713 Selected";
  none.addEventListener("click", () => chooseTheme(null));
  el("theme-rows").append(none);

  for (const theme of themes) {
    const row = document.createElement("li");
    row.className = on && theme.id === current ? "on" : "";
    row.innerHTML = `<div class="t-name"></div><div class="t-desc"></div><span class="t-mark"></span>`;
    row.querySelector(".t-name").textContent = theme.id;
    row.querySelector(".t-desc").textContent = theme.description;
    // D-091 again: the row says which state it is in, rather than leaving a
    // highlight to imply it — including the state where a theme is chosen but
    // subtitles are switched off, which a highlight alone cannot express.
    row.querySelector(".t-mark").textContent =
      on && theme.id === current ? "\u2713 Selected" : theme.default ? "Default" : "";
    row.addEventListener("click", () => chooseTheme(theme.id));
    el("theme-rows").append(row);
  }
  drawPreview();
}

// One handler for the whole list. `null` is the "No subtitles" row; anything
// else is a look, and picking a look is itself the request to burn them —
// choosing one and having nothing happen is the defect D-091 describes.
function chooseTheme(id) {
  if (id === null) {
    chosenSubtitles = false;
  } else {
    chosenSubtitles = true;
    chosenTheme = id;
  }
  drawThemes();
  rememberChoices();
  updateRender();
}

function resetSubtitles() {
  chosenSubtitles = null;
  chosenTheme = null;
  drawThemes();
  rememberChoices();
  updateRender();
}

// The renderer's own preview, painted straight into a canvas. The response is
// eight bytes of little-endian width and height, then straight RGBA — see
// `subtitle_preview` in main.rs for why it is raw rather than an image file.
async function drawPreview() {
  const canvas = el("theme-canvas");
  const token = ++previewToken;
  let body;
  try {
    body = await invoke("subtitle_preview", {
      text: el("subs-text").value,
      // Empty means "no subtitles" — the bare frame, which is what that row
      // actually produces. A preview showing a caption under a header reading
      // "Nothing is burned into the picture" contradicts itself.
      theme: subtitlesOn() ? effectiveTheme() : "",
      position: el("subs-position").value,
      shortEdge: 360,
      // The shape this run will render in (D-143). Scale is free — the themes
      // are fractions of the frame — but shape is not: the same sentence wraps
      // to two lines at 16:9 and to four in a Short, and a landscape preview of
      // a vertical film is wrong about the one thing this chooser is for.
      aspect: chosenAspect ?? project?.aspect ?? "",
    });
  } catch (error) {
    el("subs-note").textContent = String(error);
    return;
  }
  // A slower earlier request must not paint over a faster later one.
  if (token !== previewToken) return;

  const bytes = new Uint8Array(body);
  const head = new DataView(bytes.buffer, bytes.byteOffset, 8);
  const width = head.getUint32(0, true);
  const height = head.getUint32(4, true);
  canvas.width = width;
  canvas.height = height;

  const pixels = new Uint8ClampedArray(bytes.buffer, bytes.byteOffset + 8, width * height * 4);
  canvas.getContext("2d").putImageData(new ImageData(pixels, width, height), 0, 0);
  el("subs-note").textContent = subtitlesOn()
    ? "Drawn by the renderer, at " + width + "\u00d7" + height +
      " \u2014 the film gets the same design at its own size."
    : "No subtitles: this is the frame, untouched. Click any look to see it.";
}

// Two theme switches, one setting: the one in the title bar is always to hand,
// the one on Settings is where someone goes looking for it.
function setTheme(name) {
  document.documentElement.dataset.theme = name;
  for (const group of ["theme", "theme-2"]) {
    for (const button of el(group).children) button.classList.toggle("on", button.dataset.theme === name);
  }
  try { localStorage.setItem("theme", name); } catch { /* storage unavailable */ }
}

for (const group of ["theme", "theme-2"]) {
  for (const button of el(group).children) {
    button.addEventListener("click", () => setTheme(button.dataset.theme));
  }
}

let startingTheme = "dark";
try { startingTheme = localStorage.getItem("theme") || "dark"; } catch { /* storage unavailable */ }
setTheme(startingTheme);

show("start");
loadHome();
watchDrops();

// `spoonstill-desktop /path/to/film` — a folder named on the command line, or
// handed over by the file manager, opens straight into the grid.
invoke("initial_project")
  .then((path) => path && load(path))
  .catch(() => { /* nothing was named; the home screen is already up */ });
