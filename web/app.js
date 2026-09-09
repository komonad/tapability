// Tapa web front end.
//
// The page is a view: every rule, every deduction and every puzzle comes from
// the WebAssembly engine in web/worker.js. All this file does is draw the board,
// turn the mouse and keyboard into commands, and keep the control column in
// sync. Colours and layout mirror src/render.rs, so the browser and the native
// window look like the same game.

"use strict";

/* ---- board values, matching src/model.rs ------------------------------- */

const UNKNOWN = 0;
const WALL = 1;
const EMPTY = 2;

const C = {
  background: "#f6f7fa",
  unknown: "#e8eaf0",
  empty: "#ffffff",
  wall: "#2a2d3c",
  clue: "#fdf8e2",
  solutionWall: "#969eb2",
  highlight: "#4068b0",
  dot: "#788092",
  grid: "#a8aebc",
  border: "#464c60",
  hover: "#3a82e2",
  wrong: "#d63030",
  win: "#2e9e42",
  highlightOutline: "#ffc448",
  errorOutline: "#e23434",
  clueText: "#303028",
  clueBad: "#c82020",
  dim: "#707f88",
};

const MIN_CELL = 11;
const MAX_CELL = 38;
const STORAGE_KEY = "tapa.settings";

/* ---- elements ---------------------------------------------------------- */

const els = {
  title: document.getElementById("title"),
  status: document.getElementById("status"),
  info: document.getElementById("info"),
  stats: document.getElementById("stats"),
  canvas: document.getElementById("board"),
  stage: document.getElementById("stage"),
  fields: document.getElementById("fields"),
  help: document.getElementById("help"),
  message: document.getElementById("message"),
  outputWrap: document.getElementById("output-wrap"),
  output: document.getElementById("output"),
  outputTitle: document.getElementById("output-title"),
  outputClose: document.getElementById("btn-output-close"),
};

const buttonIds = [
  "btn-new",
  "btn-clear",
  "btn-check",
  "btn-solution",
  "btn-onestep",
  "btn-cluestep",
  "btn-undo",
  "btn-apply",
  "btn-defaults",
  "btn-print",
  "btn-bench",
];
for (const id of buttonIds) {
  els[id] = document.getElementById(id);
}

/* ---- view state -------------------------------------------------------- */

const ui = {
  state: null,
  cells: null,
  clues: new Map(),
  cols: 0,
  rows: 0,
  cell: 20,
  dpr: 1,
  hover: -1,
  // last clue cell the cursor was over, so the Step one clue button still has a
  // target after the pointer moved onto the button itself
  lastClue: -1,
  shift: false,
  // set when Shift-hover ends, so the spotlight goes away like it does in the
  // native game instead of falling back to the last painted wall
  suppressAnchor: false,
  painting: false,
  strokeLeft: true,
  lastPaintCell: -1,
  busy: false,
  fields: [],
  outputTitle: "",
};

/* ---- worker plumbing --------------------------------------------------- */

const worker = new Worker("worker.js");
let nextId = 1;
const waiting = new Map();

worker.addEventListener("message", (event) => {
  const { id, state, error } = event.data;
  const resolve = waiting.get(id);
  if (!resolve) return;
  waiting.delete(id);
  resolve(error ? { fatal: true, error } : state);
});

worker.addEventListener("error", (event) => {
  setStatus(`the engine worker failed: ${event.message}`, "bad");
});

/** Send one command; resolves with the reply state. */
function send(cmd) {
  const id = nextId++;
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    worker.postMessage({ id, cmd });
  });
}

/** Send one command and forget about it (used while dragging). */
function fire(cmd) {
  send(cmd).then((state) => {
    if (state && !state.fatal) applyState(state);
    else if (state && state.fatal) setStatus(state.error, "bad");
  });
}

/** Send one command, showing the busy state while it runs. */
async function run(cmd) {
  setBusy(true);
  const state = await send(cmd);
  setBusy(false);
  if (state.fatal) {
    setStatus(state.error, "bad");
    return null;
  }
  applyState(state);
  return state;
}

/* ---- state -> screen --------------------------------------------------- */

function applyState(state) {
  ui.state = state;
  ui.cells = decodeDigits(state.cells);
  ui.clues = parseClues(state.clues);
  if (state.w !== ui.cols || state.h !== ui.rows) {
    ui.cols = state.w;
    ui.rows = state.h;
    layout();
  }
  updateChrome(state);
  render();
}

function updateChrome(state) {
  if (state.w > 0) {
    els.title.textContent = `Tapa ${state.w}x${state.h}`;
  }
  setStatus(state.status, state.kind === 2 ? "bad" : state.kind === 1 ? "good" : "");

  if (state.w > 0) {
    const broken = state.errorsCells && state.errorsCells.includes("1");
    els.info.textContent = `seed ${state.seed}    ${state.clueCount} clues${
      broken ? "   red = rule broken" : ""
    }`;
  } else {
    els.info.textContent = "";
  }
  els.stats.textContent = state.stats ? `last generation: ${state.stats}` : "";

  const problem = state.error || state.settingsError || "";
  els.message.textContent = problem;
  els.message.className = `message${problem ? " bad" : ""}`;

  const hasPuzzle = state.w > 0 && !ui.busy;
  for (const id of [
    "btn-clear",
    "btn-check",
    "btn-solution",
    "btn-onestep",
    "btn-cluestep",
    "btn-print",
    "btn-bench",
  ]) {
    els[id].disabled = !hasPuzzle;
  }
  els["btn-new"].disabled = ui.busy;
  els["btn-apply"].disabled = ui.busy;
  els["btn-defaults"].disabled = ui.busy;
  els["btn-undo"].disabled = !hasPuzzle || !state.canUndo;

  if (state.output) {
    showOutput(ui.outputTitle || "Output", state.output);
  }
}

function setStatus(text, kind) {
  els.status.textContent = text;
  els.status.className = `status${kind ? ` ${kind}` : ""}`;
}

function setBusy(busy) {
  ui.busy = busy;
  for (const id of buttonIds) {
    els[id].disabled = busy;
  }
  const slider = document.getElementById("f-size-slider");
  if (slider) slider.disabled = busy;
  if (ui.state) {
    updateChrome(ui.state);
  }
  render();
}

function showOutput(title, text) {
  ui.outputTitle = title;
  els.outputTitle.textContent = title;
  els.output.textContent = text;
  els.outputWrap.hidden = false;
}

function hideOutput() {
  els.outputWrap.hidden = true;
}

/* ---- decoding the engine reply ----------------------------------------- */

function decodeDigits(text) {
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) {
    out[i] = text.charCodeAt(i) - 48;
  }
  return out;
}

function parseClues(text) {
  const map = new Map();
  for (const entry of text.split(";")) {
    if (!entry) continue;
    const cut = entry.indexOf(":");
    if (cut < 0) continue;
    map.set(Number(entry.slice(0, cut)), entry.slice(cut + 1));
  }
  return map;
}

/* ---- layout ------------------------------------------------------------ */

function layout() {
  const cols = ui.cols || 20;
  const rows = ui.rows || 20;
  const availW = Math.max(220, els.stage.clientWidth || window.innerWidth - 380);
  const availH = Math.max(240, window.innerHeight - 230);
  const cell = Math.max(
    MIN_CELL,
    Math.min(MAX_CELL, Math.floor(Math.min(availW / cols, availH / rows))),
  );
  ui.cell = cell;
  ui.dpr = window.devicePixelRatio || 1;

  const width = cols * cell;
  const height = rows * cell;
  els.canvas.style.width = `${width}px`;
  els.canvas.style.height = `${height}px`;
  els.canvas.width = Math.round(width * ui.dpr);
  els.canvas.height = Math.round(height * ui.dpr);
  render();
}

window.addEventListener("resize", layout);

/* ---- drawing ----------------------------------------------------------- */

function render() {
  const canvas = els.canvas;
  const ctx = canvas.getContext("2d");
  const cell = ui.cell;
  const cols = ui.cols || 20;
  const rows = ui.rows || 20;
  const width = cols * cell;
  const height = rows * cell;

  ctx.setTransform(ui.dpr, 0, 0, ui.dpr, 0, 0);
  ctx.fillStyle = C.background;
  ctx.fillRect(0, 0, width, height);

  const state = ui.state;
  if (!state || state.w === 0 || !ui.cells || ui.cells.length !== cols * rows) {
    placeholder(ctx, width, height, "Generating a puzzle with a unique solution...");
    return;
  }

  const cells = ui.cells;
  const n = cols * rows;
  const errors = decodeDigits(state.errorsCells);
  const wrong = decodeDigits(state.wrong);
  const solution = state.solution ? decodeDigits(state.solution) : null;
  const showSolution = Boolean(state.showSolution);
  const highlight = highlightMask();

  // cell fills
  for (let i = 0; i < n; i += 1) {
    const x = (i % cols) * cell;
    const y = Math.floor(i / cols) * cell;
    let color;
    if (ui.clues.has(i)) {
      color = C.clue;
    } else if (showSolution && solution) {
      color = solution[i] === WALL ? C.solutionWall : C.empty;
    } else if (highlight && highlight[i]) {
      color = C.highlight;
    } else if (cells[i] === WALL) {
      color = C.wall;
    } else if (cells[i] === EMPTY) {
      color = C.empty;
    } else {
      color = C.unknown;
    }
    ctx.fillStyle = color;
    ctx.fillRect(x, y, cell, cell);
  }

  // a small square marks a cell the player declared empty
  if (!showSolution) {
    const side = Math.max(2, Math.floor(cell / 12));
    const offset = Math.floor((cell - side + 1) / 2);
    ctx.fillStyle = C.dot;
    for (let i = 0; i < n; i += 1) {
      if (ui.clues.has(i) || cells[i] !== EMPTY) continue;
      if (highlight && highlight[i]) continue;
      ctx.fillRect(
        (i % cols) * cell + offset,
        Math.floor(i / cols) * cell + offset,
        side,
        side,
      );
    }
  }

  // clue numbers
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const [idx, text] of ui.clues) {
    const digits = text.split("");
    const count = digits.length;
    const scale = count <= 1 ? 0.66 : count === 2 ? 0.46 : count === 3 ? 0.36 : 0.3;
    ctx.font = `600 ${Math.max(9, Math.round(cell * scale))}px "Segoe UI", system-ui, sans-serif`;
    ctx.fillStyle = errors[idx] ? C.clueBad : C.clueText;
    const ccols = count <= 2 ? Math.max(1, count) : 2;
    const crows = Math.ceil(count / ccols);
    const cw = cell / ccols;
    const ch = cell / crows;
    const bx = (idx % cols) * cell;
    const by = Math.floor(idx / cols) * cell;
    digits.forEach((digit, k) => {
      ctx.fillText(
        digit,
        bx + (k % ccols) * cw + cw / 2,
        by + Math.floor(k / ccols) * ch + ch / 2,
      );
    });
  }

  // grid lines
  ctx.strokeStyle = C.grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let x = 0; x <= cols; x += 1) {
    const px = x * cell + 0.5;
    ctx.moveTo(px, 0);
    ctx.lineTo(px, height);
  }
  for (let y = 0; y <= rows; y += 1) {
    const py = y * cell + 0.5;
    ctx.moveTo(0, py);
    ctx.lineTo(width, py);
  }
  ctx.stroke();

  // spotlighted wall group and live rule violations, outline only
  if (highlight) {
    outlineGroup(ctx, highlight, C.highlightOutline, 2);
  }
  outlineGroup(ctx, errors, C.errorOutline, 2);

  // hover
  if (ui.hover >= 0 && !ui.clues.has(ui.hover) && !showSolution) {
    const x = (ui.hover % cols) * cell;
    const y = Math.floor(ui.hover / cols) * cell;
    ctx.strokeStyle = C.hover;
    ctx.lineWidth = 2;
    ctx.strokeRect(x + 1, y + 1, cell - 2, cell - 2);
  }

  // wrong cells from the last check
  ctx.strokeStyle = C.wrong;
  ctx.lineWidth = 2;
  const inset = Math.floor(cell / 5);
  for (let i = 0; i < n; i += 1) {
    if (!wrong[i]) continue;
    const x = (i % cols) * cell;
    const y = Math.floor(i / cols) * cell;
    ctx.strokeRect(x + 2, y + 2, cell - 4, cell - 4);
    ctx.beginPath();
    ctx.moveTo(x + inset, y + inset);
    ctx.lineTo(x + cell - inset, y + cell - inset);
    ctx.moveTo(x + cell - inset, y + inset);
    ctx.lineTo(x + inset, y + cell - inset);
    ctx.stroke();
  }

  // outer frame, green once the board is solved
  ctx.strokeStyle = state.solved ? C.win : C.border;
  ctx.lineWidth = state.solved ? 3 : 2;
  ctx.strokeRect(1, 1, width - 2, height - 2);

  if (ui.busy) {
    ctx.fillStyle = "rgba(246, 247, 250, 0.55)";
    ctx.fillRect(0, 0, width, height);
    placeholder(ctx, width, height, "Working in WebAssembly...");
  }
}

function placeholder(ctx, width, height, text) {
  ctx.fillStyle = C.dim;
  ctx.font = '15px "Segoe UI", system-ui, sans-serif';
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text, width / 2, height / 2);
}

/** Draw only the edges of `mask` that face a cell outside it, so a group of
 *  cells reads as one shape instead of a grid of boxes. */
function outlineGroup(ctx, mask, color, width) {
  const cols = ui.cols;
  const rows = ui.rows;
  const cell = ui.cell;
  const inside = (x, y) => x >= 0 && y >= 0 && x < cols && y < rows && mask[y * cols + x];
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  for (let y = 0; y < rows; y += 1) {
    for (let x = 0; x < cols; x += 1) {
      if (!mask[y * cols + x]) continue;
      const x0 = x * cell;
      const y0 = y * cell;
      const x1 = x0 + cell;
      const y1 = y0 + cell;
      if (!inside(x, y - 1)) {
        ctx.moveTo(x0, y0);
        ctx.lineTo(x1, y0);
      }
      if (!inside(x, y + 1)) {
        ctx.moveTo(x0, y1);
        ctx.lineTo(x1, y1);
      }
      if (!inside(x - 1, y)) {
        ctx.moveTo(x0, y0);
        ctx.lineTo(x0, y1);
      }
      if (!inside(x + 1, y)) {
        ctx.moveTo(x1, y0);
        ctx.lineTo(x1, y1);
      }
    }
  }
  ctx.stroke();
}

/* ---- hover spotlight --------------------------------------------------- */

/** Which wall group is spotlighted: the last painted wall, or the one under
 *  the cursor while Shift is held (exactly like the native game). */
function anchorIndex() {
  const state = ui.state;
  if (!state || !ui.cells || state.w === 0) return -1;
  if (ui.shift) {
    return ui.hover >= 0 && ui.cells[ui.hover] === WALL ? ui.hover : -1;
  }
  if (ui.suppressAnchor) return -1;
  return typeof state.anchor === "number" ? state.anchor : -1;
}

function highlightMask() {
  const anchor = anchorIndex();
  if (anchor < 0 || !ui.cells || ui.cells[anchor] !== WALL) return null;
  return floodFill(anchor);
}

function floodFill(start) {
  const cols = ui.cols;
  const rows = ui.rows;
  const cells = ui.cells;
  const mask = new Uint8Array(cols * rows);
  mask[start] = 1;
  const stack = [start];
  while (stack.length) {
    const c = stack.pop();
    const x = c % cols;
    const y = (c - x) / cols;
    if (x > 0 && !mask[c - 1] && cells[c - 1] === WALL) {
      mask[c - 1] = 1;
      stack.push(c - 1);
    }
    if (x + 1 < cols && !mask[c + 1] && cells[c + 1] === WALL) {
      mask[c + 1] = 1;
      stack.push(c + 1);
    }
    if (y > 0 && !mask[c - cols] && cells[c - cols] === WALL) {
      mask[c - cols] = 1;
      stack.push(c - cols);
    }
    if (y + 1 < rows && !mask[c + cols] && cells[c + cols] === WALL) {
      mask[c + cols] = 1;
      stack.push(c + cols);
    }
  }
  return mask;
}

/* ---- mouse ------------------------------------------------------------- */

function cellAt(event) {
  const rect = els.canvas.getBoundingClientRect();
  const x = Math.floor((event.clientX - rect.left) / ui.cell);
  const y = Math.floor((event.clientY - rect.top) / ui.cell);
  if (x < 0 || y < 0 || x >= ui.cols || y >= ui.rows) return -1;
  return y * ui.cols + x;
}

els.canvas.addEventListener("pointerdown", (event) => {
  els.canvas.focus();
  const cell = cellAt(event);
  if (cell < 0 || ui.busy) return;
  event.preventDefault();
  const x = cell % ui.cols;
  const y = Math.floor(cell / ui.cols);

  // middle click, or Alt + click for trackpads without a middle button
  if (event.button === 1 || (event.button === 0 && event.altKey)) {
    stepClue(cell);
    return;
  }
  if (event.button !== 0 && event.button !== 2) return;

  ui.painting = true;
  ui.strokeLeft = event.button === 0;
  ui.lastPaintCell = cell;
  ui.suppressAnchor = false;
  try {
    els.canvas.setPointerCapture(event.pointerId);
  } catch {
    /* synthetic events have no real pointer to capture */
  }
  fire(`paint begin ${x} ${y} ${ui.strokeLeft ? 1 : 0}`);
});

els.canvas.addEventListener("pointermove", (event) => {
  const cell = cellAt(event);
  if (ui.painting) {
    if (cell < 0 || cell === ui.lastPaintCell) return;
    ui.lastPaintCell = cell;
    ui.suppressAnchor = false;
    const x = cell % ui.cols;
    const y = Math.floor(cell / ui.cols);
    fire(`paint move ${x} ${y}`);
    return;
  }
  if (cell !== ui.hover) {
    ui.hover = cell;
    if (cell >= 0 && ui.clues.has(cell)) {
      ui.lastClue = cell;
    }
    render();
  }
});

function endStroke(event) {
  if (!ui.painting) return;
  ui.painting = false;
  fire("paint end");
  if (event) {
    ui.hover = cellAt(event);
    render();
  }
}

els.canvas.addEventListener("pointerup", endStroke);
els.canvas.addEventListener("pointercancel", endStroke);
els.canvas.addEventListener("pointerleave", () => {
  if (ui.hover !== -1) {
    ui.hover = -1;
    render();
  }
});
els.canvas.addEventListener("contextmenu", (event) => event.preventDefault());
els.canvas.addEventListener("auxclick", (event) => {
  if (event.button === 1) event.preventDefault();
});
els.canvas.addEventListener("mousedown", (event) => {
  if (event.button === 1) event.preventDefault();
});

/* ---- clue stepping ----------------------------------------------------- */

/** Deduce what a single clue forces, exactly like a middle click does. */
function stepClue(cell) {
  const index = typeof cell === "number" && cell >= 0 ? cell : ui.lastClue;
  if (index < 0) {
    setStatus("Hover a clue cell first, then step it.", "bad");
    return;
  }
  if (ui.clues.has(index)) {
    ui.lastClue = index;
  }
  const x = index % ui.cols;
  const y = Math.floor(index / ui.cols);
  fire(`cluestep ${x} ${y}`);
}

/* ---- keyboard ---------------------------------------------------------- */

const KEYS = { n: "new", r: "clear", c: "check", s: "solution", d: "onestep", z: "undo" };

window.addEventListener("keydown", (event) => {
  const tag = event.target && event.target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
  if (event.ctrlKey || event.metaKey || event.altKey) return;

  if (event.key === "Shift") {
    if (!ui.shift) {
      ui.shift = true;
      render();
    }
    return;
  }
  if (event.key === "Escape") {
    hideOutput();
    return;
  }
  const action = KEYS[event.key.toLowerCase()];
  if (!action || ui.busy) return;
  event.preventDefault();
  press(action);
});

window.addEventListener("keyup", (event) => {
  if (event.key === "Shift") {
    ui.shift = false;
    ui.suppressAnchor = true;
    render();
  }
});

window.addEventListener("blur", () => {
  ui.shift = false;
  ui.suppressAnchor = true;
  ui.painting = false;
  render();
});

function press(action) {
  switch (action) {
    case "new":
      newPuzzle();
      break;
    case "clear":
      run("reset");
      break;
    case "check":
      run("check");
      break;
    case "solution":
      run("solution");
      break;
    case "onestep":
      run("onestep");
      break;
    case "undo":
      run("undo");
      break;
    default:
      break;
  }
}

/* ---- settings column --------------------------------------------------- */

function buildFields(fields) {
  ui.fields = fields;
  els.fields.textContent = "";
  for (const field of fields) {
    const row = document.createElement("div");
    row.className = "field";

    const label = document.createElement("label");
    label.textContent = field.label;
    label.htmlFor = `f-${field.key}`;

    const input = document.createElement("input");
    input.id = `f-${field.key}`;
    input.dataset.key = field.key;
    input.value = field.value;
    input.title = field.help;
    input.spellcheck = false;
    input.autocomplete = "off";

    row.append(label, input);

    // the board size also gets a slider, under its field
    if (field.key === "size") {
      const slider = document.createElement("input");
      slider.type = "range";
      slider.id = "f-size-slider";
      slider.min = "3";
      slider.max = "60";
      slider.step = "1";
      slider.value = field.value;
      slider.title = field.help;
      slider.setAttribute("aria-label", "Board size");
      // dragging only updates the number; releasing rebuilds the puzzle
      slider.addEventListener("input", () => {
        input.value = slider.value;
      });
      slider.addEventListener("change", () => {
        input.value = slider.value;
        applySettings();
      });
      row.append(slider);
    }

    row.addEventListener("mouseenter", () => showHelp(field, row));
    row.addEventListener("mouseleave", () => showHelp(null, null));
    input.addEventListener("focus", () => showHelp(field, row));
    input.addEventListener("blur", () => showHelp(null, null));
    els.fields.append(row);
  }
}

function showHelp(field, row) {
  for (const other of els.fields.children) {
    other.classList.toggle("active", other === row);
  }
  els.help.textContent = field
    ? `${field.label}: ${field.help}`
    : "Hover a setting to read what it does.";
}

/** Write the engine's settings back into the inputs (after Defaults, or on load). */
function fillFromText(text) {
  const values = new Map();
  for (const line of text.split("\n")) {
    const clean = line.split("#")[0].trim();
    if (!clean) continue;
    const cut = clean.indexOf("=");
    if (cut < 0) continue;
    values.set(clean.slice(0, cut).trim(), clean.slice(cut + 1).trim());
  }
  for (const input of els.fields.querySelectorAll("input")) {
    if (input.type === "range") continue;
    const value = values.has(input.dataset.key) ? values.get(input.dataset.key) : "";
    input.value = value;
    if (input.dataset.key === "size" && /^\d+$/.test(value)) {
      const slider = document.getElementById("f-size-slider");
      if (slider) slider.value = value;
    }
  }
}

function readSaved() {
  try {
    const text = window.localStorage.getItem(STORAGE_KEY);
    if (!text) return [];
    return text
      .split("\n")
      .map((line) => line.split("#")[0].trim())
      .filter(Boolean)
      .map((line) => {
        const cut = line.indexOf("=");
        return cut < 0 ? null : [line.slice(0, cut).trim(), line.slice(cut + 1).trim()];
      })
      .filter(Boolean);
  } catch {
    return [];
  }
}

function saveSettings(text) {
  try {
    window.localStorage.setItem(STORAGE_KEY, text);
  } catch {
    /* private mode, ignore */
  }
}

async function applySettings() {
  setBusy(true);
  let state = null;
  let seedText = "";
  for (const input of els.fields.querySelectorAll("input")) {
    if (input.type === "range") continue;
    state = await send(`set ${input.dataset.key} ${input.value.trim()}`);
    if (input.dataset.key === "seed") seedText = input.value.trim();
    if (state.settingsError) {
      setBusy(false);
      applyState(state);
      fillFromText(state.settingsText);
      return;
    }
  }
  if (state) {
    saveSettings(state.settingsText);
    // the engine may have normalised the values, and the size slider has to
    // follow whatever size was applied
    fillFromText(state.settingsText);
  }
  const seed = /^\d+$/.test(seedText) ? Number(seedText) : newSeed();
  await generate(seed);
}

async function useDefaults() {
  setBusy(true);
  const state = await send("defaults");
  setBusy(false);
  if (state.fatal) {
    setStatus(state.error, "bad");
    return;
  }
  applyState(state);
  fillFromText(state.settingsText);
  saveSettings(state.settingsText);
  await generate(newSeed());
}

/* ---- puzzle flow ------------------------------------------------------- */

function newSeed() {
  const hi = Date.now() % 2 ** 31;
  const lo = Math.floor(Math.random() * 2 ** 31);
  return hi * 2 ** 21 + lo;
}

async function generate(seed) {
  setBusy(true);
  setStatus("Generating a puzzle with a unique solution...", "");
  const state = await send(`generate ${seed}`);
  setBusy(false);
  if (state.fatal) {
    setStatus(state.error, "bad");
    return;
  }
  applyState(state);
}

function newPuzzle() {
  const current = ui.state && ui.state.seed ? ui.state.seed : 0;
  generate(current + 1);
}

async function batch(kind) {
  const count = kind === "bench" ? 3 : 1;
  ui.outputTitle = kind === "bench" ? `Bench: ${count} generations` : "Print: 1 puzzle";
  setBusy(true);
  setStatus(
    kind === "bench"
      ? `Benchmarking ${count} generations inside WebAssembly...`
      : "Generating a puzzle to print...",
    "",
  );
  const state = await send(`${kind} ${count}`);
  setBusy(false);
  if (state.fatal) {
    setStatus(state.error, "bad");
    return;
  }
  applyState(state);
}

/* ---- wiring ------------------------------------------------------------ */

els["btn-new"].addEventListener("click", newPuzzle);
els["btn-clear"].addEventListener("click", () => run("reset"));
els["btn-check"].addEventListener("click", () => run("check"));
els["btn-solution"].addEventListener("click", () => run("solution"));
els["btn-onestep"].addEventListener("click", () => run("onestep"));
els["btn-cluestep"].addEventListener("click", () => stepClue(ui.hover));
els["btn-undo"].addEventListener("click", () => run("undo"));
els["btn-apply"].addEventListener("click", applySettings);
els["btn-defaults"].addEventListener("click", useDefaults);
els["btn-print"].addEventListener("click", () => batch("print"));
els["btn-bench"].addEventListener("click", () => batch("bench"));
els.outputClose.addEventListener("click", hideOutput);

async function start() {
  setStatus("Loading the WebAssembly engine...", "");
  const init = await send("init");
  if (init.fatal) {
    setStatus(`cannot start the engine: ${init.error}`, "bad");
    return;
  }
  buildFields(init.fields || []);

  // remembered settings win over the defaults the engine just reported
  let state = init;
  for (const [key, value] of readSaved()) {
    state = await send(`set ${key} ${value}`);
  }
  fillFromText(state.settingsText);
  saveSettings(state.settingsText);

  const seedInput = document.getElementById("f-seed");
  const seedText = seedInput ? seedInput.value.trim() : "";
  await generate(/^\d+$/.test(seedText) ? Number(seedText) : newSeed());
}

start();
