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

const el = (id) => document.getElementById(id);
const MEDIA = [
  "jpg", "jpeg", "png", "webp", "tif", "tiff", "bmp", "heic",
  "mp3", "m4a", "wav", "aac", "flac", "ogg", "opus", "wma",
  "txt", "md",
];

const TABS = ["scenes", "voice", "output", "render"];

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
// Why a render cannot start for a reason that is about this machine rather
// than about this project — a missing FFmpeg. Empty when there is none
// (D-105). Asked once per project open, not once per photograph (D-103).
let ffmpegBlocker = "";
let outDir = "";
let outName = "";
let outFull = "";
let outError = "";

// The provider's catalogue, fetched once per project, and the voice it falls
// back to when a project names none — so the window can say whose voice
// "default" actually is.
let voices = [];
let voicesLoaded = false;
let providerDefault = "";
// The machine's fallback voice, from Settings. Null means "the provider's own".
let appDefaultVoice = null;

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
  if (name === "voice") loadVoices();
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
    const settings = await invoke("app_settings");
    appDefaultVoice = settings.default_voice || null;
  } catch {
    appDefaultVoice = null;
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
    project = { root, name: root.split("/").pop(), scenes: [] };
    el("fill-name").textContent = project.name;
    el("t-name").textContent = project.name;
    el("t-path").textContent = root;
    show("fill");
    setStatus(root);
  } catch (error) {
    setStatus(String(error));
  }
}

async function openProject() {
  const chosen = await dialog({ directory: true, multiple: false, title: "Open a project folder" });
  if (typeof chosen === "string") await load(chosen);
}

async function load(path) {
  setStatus("Reading the folder…");
  const opening = project?.root !== path;
  try {
    project = await invoke("validate_project", { path });
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
  draw();
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
  el("s-scenes").textContent = String(project.scenes.length);
  el("s-mode").textContent = project.mode;
  el("s-root").textContent = project.root;
  el("s-output").textContent = project.output_path || project.output;

  drawChips();
  drawRows();
  drawProblems();
  drawVoiceChoice();
  drawStatus();

  el("pip-scenes").textContent = String(project.scenes.length);
  updateRender();
}

const count = (kind) => project.scenes.filter((s) => s.source === kind).length;
const attention = () => project.problems.filter((p) => p.severity === "error").length;

function drawChips() {
  const chips = [
    ["all", "All", project.scenes.length, false],
    ["attention", "Needs attention", attention(), true],
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
  return project.scenes.filter((scene) => {
    if (filter === "attention" && !broken.has(scene.id)) return false;
    if (["tts", "file", "silent"].includes(filter) && scene.source !== filter) return false;
    if (!needle) return true;
    return (scene.id + " " + scene.image + " " + scene.narration + " " + scene.audio)
      .toLowerCase().includes(needle);
  });
}

function drawRows() {
  const broken = new Set(project.problems.filter((p) => p.severity === "error" && p.scene).map((p) => p.scene));
  const rows = el("rows");
  const shown = visible();
  rows.innerHTML = "";
  el("grid-empty").hidden = shown.length > 0;

  const shape = project.geometry.startsWith("1080x1920") ? "portrait"
    : /^(\d+)x\1 /.test(project.geometry) ? "square" : "";

  for (const scene of shown) {
    const tr = document.createElement("tr");
    tr.id = "scene-" + scene.index;
    if (broken.has(scene.id)) tr.classList.add("problem");
    tr.innerHTML = `
      <td class="c-scene"></td>
      <td class="c-still"><img class="thumb ${shape}" loading="lazy" alt="" /></td>
      <td class="c-source"><div class="source-cell">
        <span class="file"></span><span class="narration"></span>
      </div></td>
      <td class="c-audio"><span class="badge"></span></td>
      <td class="c-resolved"></td>
      <td class="c-arrange"><div class="arrange"></div></td>`;

    // textContent, never innerHTML, for anything an operator named or typed: a
    // file called <img onerror=…> is a filename, not markup (D-052).
    tr.querySelector(".c-scene").textContent = scene.id;
    tr.querySelector(".thumb").src = convertFileSrc(scene.image_path);
    tr.querySelector(".file").textContent = scene.image;

    const badge = tr.querySelector(".badge");
    badge.classList.add(scene.source);
    badge.textContent = { tts: "TTS", file: "FILE", silent: "SILENT" }[scene.source] ?? scene.source;

    const cell = tr.querySelector(".narration");
    if (scene.source === "file") {
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
    if (scene.source !== "file" && project.convention) {
      cell.addEventListener("click", () => editNarration(scene, cell));
    }

    tr.querySelector(".c-resolved").innerHTML = resolved(scene);
    drawArrange(tr.querySelector(".arrange"), scene, shown.length);
    rows.appendChild(tr);
  }
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
    if (!save || text === scene.narration) return;
    try {
      setStatus("Saving…");
      await invoke("set_narration", { root: project.root, scene: scene.id, text });
      await load(project.root);
      setStatus(text.trim() ? `Scene ${scene.id} will be spoken.` : `Scene ${scene.id} is silent again.`);
    } catch (error) {
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

// Move a scene, or take it out (D-099).
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

  const total = project.scenes.length;
  const position = scene.index + 1;
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
  const parts = [`${shown} of ${project.scenes.length} scenes`];
  if (declared > 0) parts.push(`${declared.toFixed(1)}s declared`);
  const errors = attention();
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

// The voice that will actually be used if nothing is chosen here. Three
// answers in order of who wins: this run's override, the project's own
// `tts.voice`, then the machine's fallback — and the provider's own only when
// none of the three said anything (D-092).
// True when `project.yaml` left the choice open, which is what makes the
// machine's fallback apply at all.
function projectNamesNoVoice() {
  const named = project?.voice || "";
  return !named || named === "default";
}

function effectiveVoice() {
  if (chosenVoice) return chosenVoice;
  const named = project?.voice || "";
  if (named && named !== "default") return named;
  return appDefaultVoice || providerDefault;
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
    state.textContent = String(error);
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
    // identical. Each one now says which it is, in a word (D-091).
    const isCurrent = voice.id === current;
    if (isCurrent) li.classList.add(chosenVoice ? "on" : "is-default");
    li.innerHTML =
      `<span class="v-name"></span><span class="v-mark"></span>` +
      `<span class="v-lang"></span>` +
      `<span class="v-gender"></span><span class="v-note"></span>` +
      `<span class="v-id mono"></span><button class="v-play">▶</button>`;
    li.children[0].textContent = voiceName(voice);
    li.children[1].textContent = isCurrent
      ? (chosenVoice ? "✓ Selected" : "Project default")
      : "";
    li.children[2].textContent = languageOf(voice.locale);
    li.children[3].textContent = voice.gender;
    li.children[4].textContent = voice.note;
    li.children[4].title = voice.note;
    li.children[5].textContent = voice.id;
    li.children[6].title = "Hear this voice";
    li.setAttribute("aria-selected", String(isCurrent));
    li.title = isCurrent && chosenVoice
      ? "This voice reads every written line"
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

function chooseVoice(id) {
  chosenVoice = id || null;
  rememberChoices();
  drawVoiceChoice();
  drawVoices();
  // Clicking used to change nothing an operator could see: the row that was
  // already highlighted stayed highlighted, because it had been highlighted as
  // the project's default all along (D-091).
  const voice = voices.find((v) => v.id === effectiveVoice());
  setStatus(
    chosenVoice
      ? `${voice ? voiceName(voice) : chosenVoice} will read every written line.`
      : `Back to the voice named in project.yaml — ${project?.voice || "default"}.`,
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
    el("chosen-name").textContent = "The project's own voice";
    el("chosen-id").textContent = `${project?.voice || "default"} · ${project?.provider || ""}`;
  }

  const tag = el("chosen-tag");
  tag.textContent = chosenVoice ? "✓ Selected for this render" : "From project.yaml";
  tag.className = "chosen-tag" + (chosenVoice ? " on" : "");

  el("voice-default").disabled = !chosenVoice;
  el("rail-voice").textContent = voice
    ? `${voiceName(voice)} · ${languageOf(voice.locale)}`
    : current || "project default";
  el("go-voice").title = current
    ? `${current}${chosenVoice ? " — an override for the next render" : " — the project's default"}`
    : "The voice named in project.yaml";
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
  if (project.has_errors) {
    const n = attention();
    return `${n} scene${n === 1 ? " needs" : "s need"} attention — see the list on Scenes.`;
  }
  if (outError) return outError;
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
  return refreshOutput();
}

// The join and the validation both happen in Rust. This only shows the answer,
// and refuses to render while the answer is a complaint.
async function refreshOutput() {
  if (!project) return;
  outDir = el("out-dir").value;
  outName = el("out-name").value;
  try {
    outFull = await invoke("resolve_output", { dir: outDir, name: outName });
    outError = "";
  } catch (error) {
    outFull = "";
    outError = String(error);
  }
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
    localStorage.setItem("choices:" + project.root, JSON.stringify({ chosenVoice, outDir, outName }));
  } catch { /* storage unavailable */ }
}

function restoreChoices() {
  chosenVoice = null;
  let dir = project.output_dir ?? "";
  let name = project.output_name ?? "";
  try {
    const saved = JSON.parse(localStorage.getItem("choices:" + project.root) ?? "{}");
    if (typeof saved.chosenVoice === "string") chosenVoice = saved.chosenVoice;
    if (typeof saved.outDir === "string" && saved.outDir) dir = saved.outDir;
    if (typeof saved.outName === "string" && saved.outName) name = saved.outName;
  } catch { /* nothing remembered, or storage is unavailable */ }
  el("out-dir").value = dir;
  el("out-name").value = name;
  guard(refreshOutput());
}

// ------------------------------------------------------------------ render

async function render() {
  if (!project || rendering) return;
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
  el("r-voice").textContent = effectiveVoice() || project.voice || "default";
  el("r-out").textContent = outFull;
  tab("render");

  const progress = new Channel();
  let done = 0;
  progress.onmessage = (event) => {
    if (event.kind === "planned") {
      note(`${event.scenes} scenes, ${event.jobs} at a time, ${event.audio_jobs} narrations at a time`);
      el("progress-line").textContent = `0 of ${event.scenes}`;
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
        // to project.yaml (D-013, D-092).
        voice: chosenVoice || (projectNamesNoVoice() ? appDefaultVoice : null),
        outDir: el("out-dir").value,
        outName: el("out-name").value,
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
  await events.listen("tauri://drag-enter", () => { over.hidden = false; });
  await events.listen("tauri://drag-leave", () => { over.hidden = true; });
  await events.listen("tauri://drag-drop", async (event) => {
    over.hidden = true;
    const paths = event.payload?.paths ?? [];
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
    await addMedia(paths);
  });
}

// ------------------------------------------------------------------ plumbing

const guard = (promise) => Promise.resolve(promise).catch((error) => setStatus(String(error)));

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
el("add").addEventListener("click", chooseMedia);
el("render").addEventListener("click", render);
el("cancel").addEventListener("click", cancel);
el("recheck").addEventListener("click", () => project && load(project.root));
el("search").addEventListener("input", () => { drawRows(); drawStatus(); });
el("play").addEventListener("click", () => guard(invoke("open_film")));
el("reveal").addEventListener("click", () => guard(invoke("reveal_project")));
el("reveal-2").addEventListener("click", () => guard(invoke("reveal_project")));

el("preview").addEventListener("click", () => preview(null));
el("voice-default").addEventListener("click", () => chooseVoice(null));
el("voice-search").addEventListener("input", drawVoices);
el("locale").addEventListener("change", drawVoices);
el("gender").addEventListener("change", drawVoices);

el("out-name").addEventListener("input", () => guard(refreshOutput()));
el("out-dir").addEventListener("input", () => guard(refreshOutput()));
el("out-browse").addEventListener("click", () => guard(browseOutput()));
el("out-default").addEventListener("click", () => guard(resetOutput()));

for (const button of [...el("tabs").children, el("go-voice"), el("go-output")]) {
  button.addEventListener("click", () => tab(button.dataset.tab));
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
