// End-to-end check of the web front end in headless Chrome.
//
//   1. node tools/serve.cjs 8080
//   2. chrome --headless=new --remote-debugging-port=9222 --user-data-dir=<dir> about:blank
//   3. node tools/browser-check.cjs [http://127.0.0.1:8080/] [127.0.0.1:9222]
//
// It drives the real page with real input events (CDP Input.*), asserts on the
// engine state, and writes web-check.png so the layout can be inspected.

const fs = require("node:fs");
const path = require("node:path");

const pageUrl = process.argv[2] || "http://127.0.0.1:8080/";
const debugHost = process.argv[3] || "127.0.0.1:9222";

let failures = 0;
function check(label, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${label}${ok || !detail ? "" : `: ${detail}`}`);
  if (!ok) failures += 1;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function main() {
  const list = await (await fetch(`http://${debugHost}/json/list`)).json();
  const target = list.find((t) => t.type === "page");
  if (!target) throw new Error("no page target; is Chrome running with --remote-debugging-port?");
  const socket = new WebSocket(target.webSocketDebuggerUrl);

  let nextId = 0;
  const pending = new Map();
  const consoleErrors = [];

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
      return;
    }
    if (message.method === "Runtime.exceptionThrown") {
      const details = message.params.exceptionDetails;
      consoleErrors.push(details.exception?.description || details.text);
    }
    if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
      consoleErrors.push(message.params.args.map((a) => a.value ?? a.description).join(" "));
    }
  });

  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve);
    socket.addEventListener("error", () => reject(new Error("cannot reach the debugger")));
  });

  function call(method, params = {}) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async function evaluate(expression) {
    const result = await call("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    }
    return result.result.value;
  }

  async function waitFor(expression, label, timeout = 60000) {
    const deadline = Date.now() + timeout;
    for (;;) {
      if (await evaluate(expression)) return true;
      if (Date.now() > deadline) {
        check(label, false, "timed out");
        return false;
      }
      await sleep(120);
    }
  }

  await call("Runtime.enable");
  await call("Page.enable");
  await call("Log.enable");
  await call("Page.navigate", { url: pageUrl });

  // start from a clean slate: no remembered settings from an earlier run
  await waitFor(`document.readyState === "complete" && typeof ui !== "undefined"`, "the page loads");
  await evaluate(`localStorage.clear()`);
  await call("Page.reload", { ignoreCache: true });

  // ---- the page must come up with a puzzle --------------------------------
  const ready = await waitFor(
    `document.readyState === "complete" && typeof ui !== "undefined" && ui.state && ui.state.w > 0 && !ui.busy`,
    "the page generates a puzzle",
  );
  if (!ready) {
    const diagnosis = await evaluate(`({
      status: document.getElementById("status").textContent,
      canvas: [document.getElementById("board").width, document.getElementById("board").height],
      hasUi: typeof ui !== "undefined",
      state: typeof ui !== "undefined" && ui.state ? { w: ui.state.w, seed: ui.state.seed } : null,
      workers: typeof Worker !== "undefined",
    })`);
    console.error("page state:", JSON.stringify(diagnosis));
    console.error("console errors:", consoleErrors.join(" | ") || "(none)");
    throw new Error("the page never produced a puzzle");
  }

  const summary = await evaluate(`({
    title: document.title,
    heading: document.getElementById("title").textContent,
    status: document.getElementById("status").textContent,
    info: document.getElementById("info").textContent,
    stats: document.getElementById("stats").textContent,
    cols: ui.cols, rows: ui.rows, cell: ui.cell,
    clues: ui.clues.size,
    seed: ui.state.seed,
    canvasW: document.getElementById("board").width,
    fields: ui.fields.length,
    hasWallCount: /wall/i.test(document.getElementById("info").textContent)
  })`);
  console.log(
    `  board: ${summary.heading}, ${summary.clues} clues, seed ${summary.seed}, ` +
      `${summary.cols}x${summary.rows} at ${summary.cell}px, ${summary.fields} settings fields`,
  );
  check("title is Tapa", summary.title === "Tapa", summary.title);
  check("heading shows the board size", /^Tapa \d+x\d+$/.test(summary.heading), summary.heading);
  check("status reports cells left", /cell/.test(summary.status), summary.status);
  check("clues are present", summary.clues > 0, String(summary.clues));
  check("footer does not leak the wall count", !summary.hasWallCount, summary.info);
  check("footer shows the seed and clue count", /seed \d+\s+\d+ clues/.test(summary.info), summary.info);
  check("generation stats are shown", /clues, \d+x\d+/.test(summary.stats), summary.stats);
  check("canvas has a backing store", summary.canvasW > 100, String(summary.canvasW));
  check("settings column has every field", summary.fields === 9, String(summary.fields));

  // the canvas must actually be painted: look for wall-coloured pixels
  const painted = await evaluate(`(() => {
    const canvas = document.getElementById("board");
    const ctx = canvas.getContext("2d");
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    let wall = 0, clue = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i] === 42 && data[i + 1] === 45 && data[i + 2] === 60) wall += 1;
      if (data[i] === 253 && data[i + 1] === 248 && data[i + 2] === 226) clue += 1;
    }
    return { wall, clue };
  })()`);
  check("clue cells are drawn", painted.clue > 0, JSON.stringify(painted));

  // ---- helpers to turn a cell index into viewport coordinates -------------
  const geometry = await evaluate(`(() => {
    const r = document.getElementById("board").getBoundingClientRect();
    return { left: r.left, top: r.top, cell: ui.cell, cols: ui.cols };
  })()`);
  const point = (index) => ({
    x: geometry.left + (index % geometry.cols) * geometry.cell + geometry.cell / 2,
    y: geometry.top + Math.floor(index / geometry.cols) * geometry.cell + geometry.cell / 2,
  });

  async function mouse(type, index, button = "left") {
    const { x, y } = point(index);
    const buttons =
      type === "mouseReleased"
        ? 0
        : button === "left"
          ? 1
          : button === "right"
            ? 2
            : button === "middle"
              ? 4
              : 0;
    await call("Input.dispatchMouseEvent", {
      type,
      x,
      y,
      button,
      buttons,
      clickCount: 1,
      pointerType: "mouse",
    });
  }

  async function key(text) {
    const code = `Key${text.toUpperCase()}`;
    const vk = text.toUpperCase().charCodeAt(0);
    for (const type of ["keyDown", "keyUp"]) {
      await call("Input.dispatchKeyEvent", {
        type,
        key: text,
        code,
        windowsVirtualKeyCode: vk,
        nativeVirtualKeyCode: vk,
      });
    }
  }

  const freeCells = await evaluate(`(() => {
    const out = [];
    for (let i = 0; i < ui.cells.length; i += 1) if (!ui.clues.has(i)) out.push(i);
    return out.slice(0, 40);
  })()`);
  check("there are paintable cells", freeCells.length >= 6, String(freeCells.length));

  // ---- left click paints a wall, clicking it again clears it --------------
  const a = freeCells[0];
  await mouse("mousePressed", a, "left");
  await mouse("mouseReleased", a, "left");
  await sleep(80);
  check("left click paints a wall", (await evaluate(`ui.cells[${a}]`)) === 1);
  check(
    "the new wall group is spotlighted",
    await evaluate(`!!highlightMask() && highlightMask()[${a}] === 1`),
  );

  await mouse("mousePressed", a, "left");
  await mouse("mouseReleased", a, "left");
  await sleep(80);
  check("clicking the same wall clears it", (await evaluate(`ui.cells[${a}]`)) === 0);

  // ---- right click paints an empty mark ----------------------------------
  const b = freeCells[1];
  await mouse("mousePressed", b, "right");
  await mouse("mouseReleased", b, "right");
  await sleep(80);
  check("right click marks a cell empty", (await evaluate(`ui.cells[${b}]`)) === 2);
  await mouse("mousePressed", b, "right");
  await mouse("mouseReleased", b, "right");
  await sleep(80);
  check("clicking the same empty mark clears it", (await evaluate(`ui.cells[${b}]`)) === 0);

  // ---- drag paints a whole stroke ----------------------------------------
  const from = await evaluate(`(() => {
    for (let i = 0; i < ui.cells.length - 3; i += 1) {
      if (i % ui.cols > ui.cols - 4) continue;
      if ([i, i + 1, i + 2, i + 3].some((k) => ui.clues.has(k))) continue;
      return i;
    }
    return -1;
  })()`);
  check("found four free cells in a row", from >= 0, String(from));
  const to = from + 3;
  const start = point(from);
  const end = point(to);
  await call("Input.dispatchMouseEvent", { type: "mousePressed", ...start, button: "left", buttons: 1, clickCount: 1 });
  for (let step = 1; step <= 6; step += 1) {
    await call("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: start.x + ((end.x - start.x) * step) / 6,
      y: start.y + ((end.y - start.y) * step) / 6,
      button: "left",
      buttons: 1,
    });
  }
  await call("Input.dispatchMouseEvent", { type: "mouseReleased", ...end, button: "left", buttons: 0, clickCount: 1 });
  await sleep(120);
  const dragged = await evaluate(
    `[${from}, ${from + 1}, ${from + 2}, ${from + 3}].map((i) => ui.cells[i]).join("")`,
  );
  check("dragging paints every cell it crosses", dragged === "1111", dragged);

  // ---- undo takes the stroke back ----------------------------------------
  await key("z");
  await sleep(80);
  const afterUndo = await evaluate(`ui.cells[${from + 3}]`);
  check("Z undoes the last painted cell", afterUndo === 0, String(afterUndo));

  // ---- stepping a single clue: middle click, Alt + click, and the button ---
  const clueIndex = await evaluate(`[...ui.clues.keys()][0]`);
  const clueText = await evaluate(`ui.clues.get(${clueIndex})`);
  const cluePattern = new RegExp(`Clue \\(\\d+,\\d+\\)`);
  const clearStatus = () => evaluate(`setStatus("(cleared)", "")`);

  await clearStatus();
  await mouse("mousePressed", clueIndex, "middle");
  await mouse("mouseReleased", clueIndex, "middle");
  await sleep(250);
  const middleStatus = await evaluate(`document.getElementById("status").textContent`);
  check(
    "middle click steps the clue under the cursor",
    cluePattern.test(middleStatus),
    `${middleStatus} (clue ${clueText})`,
  );

  await clearStatus();
  const alt = { ...point(clueIndex), button: "left", clickCount: 1, modifiers: 1 };
  await call("Input.dispatchMouseEvent", { type: "mousePressed", ...alt, buttons: 1 });
  await call("Input.dispatchMouseEvent", { type: "mouseReleased", ...alt, buttons: 0 });
  await sleep(250);
  const altStatus = await evaluate(`document.getElementById("status").textContent`);
  check("Alt + click steps the clue too", cluePattern.test(altStatus), altStatus);

  await clearStatus();
  await mouse("mouseMoved", clueIndex, "none");
  await sleep(80);
  await evaluate(`document.getElementById("btn-cluestep").click()`);
  await sleep(250);
  const buttonStatus = await evaluate(`document.getElementById("status").textContent`);
  check(
    "the Step one clue button steps the hovered clue",
    cluePattern.test(buttonStatus),
    buttonStatus,
  );

  // a cell that is not a clue must be refused politely
  const notClueCell = await evaluate(`(() => {
    for (let i = 0; i < ui.cells.length; i += 1) if (!ui.clues.has(i)) return i;
    return -1;
  })()`);
  await clearStatus();
  await mouse("mousePressed", notClueCell, "middle");
  await mouse("mouseReleased", notClueCell, "middle");
  await sleep(200);
  const refused = await evaluate(`document.getElementById("status").textContent`);
  check("stepping a non-clue cell is refused", /Middle-click a clue/.test(refused), refused);

  // ---- one step, check, solution -----------------------------------------
  // start from a clean board so every mark on it comes from the deduction
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "clear finishes");
  await evaluate(`document.getElementById("btn-onestep").click()`);
  await waitFor(`!ui.busy`, "one step finishes");
  await sleep(80);
  const oneStep = await evaluate(`document.getElementById("status").textContent`);
  check("one step fills the clues' deductions", /One step: filled \d+ cell/.test(oneStep), oneStep);

  // every deduction must agree with the solution: ask the engine to reveal it
  const consistency = await evaluate(`(async () => {
    const marks = Array.from(ui.cells);
    const clueCells = new Set(ui.clues.keys());
    const revealed = await send("solution");
    const solution = Array.from(decodeDigits(revealed.solution));
    await send("solution");
    let bad = 0;
    for (let i = 0; i < marks.length; i += 1) {
      if (clueCells.has(i)) continue;
      if (marks[i] !== 0 && marks[i] !== solution[i]) bad += 1;
    }
    return bad;
  })()`);
  check("no mark contradicts the solution", consistency === 0, String(consistency));

  await evaluate(`document.getElementById("btn-check").click()`);
  await waitFor(`!ui.busy`, "check finishes");
  await sleep(80);
  const checked = await evaluate(`document.getElementById("status").textContent`);
  check("check reports on the marks", /wrong cell|No mistakes/.test(checked), checked);

  await evaluate(`document.getElementById("btn-solution").click()`);
  await waitFor(`!ui.busy`, "solution shows");
  await sleep(120);
  check("solution button reveals the solution", await evaluate(`ui.state.showSolution === true`));
  const solvedPixels = await evaluate(`(() => {
    const canvas = document.getElementById("board");
    const ctx = canvas.getContext("2d");
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    let grey = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i] === 150 && data[i + 1] === 158 && data[i + 2] === 178) grey += 1;
    }
    return grey;
  })()`);
  check("the revealed walls are drawn", solvedPixels > 0, String(solvedPixels));
  await evaluate(`document.getElementById("btn-solution").click()`);
  await waitFor(`!ui.busy`, "solution hides");
  check("solution button hides it again", await evaluate(`ui.state.showSolution === false`));

  // ---- shift-hover spotlight ---------------------------------------------
  // use a cell that is still undecided, so one click definitely makes a wall
  const wallCell = await evaluate(`(() => {
    for (let i = 0; i < ui.cells.length; i += 1) if (!ui.clues.has(i) && ui.cells[i] === 0) return i;
    return -1;
  })()`);
  await mouse("mousePressed", wallCell, "left");
  await mouse("mouseReleased", wallCell, "left");
  await sleep(80);
  check("the cell under test became a wall", (await evaluate(`ui.cells[${wallCell}]`)) === 1);
  await call("Input.dispatchKeyEvent", { type: "keyDown", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, modifiers: 8 });
  await mouse("mouseMoved", wallCell, "none");
  await sleep(150);
  const shiftHighlight = await evaluate(`(() => {
    const mask = highlightMask();
    return mask ? mask[${wallCell}] : -1;
  })()`);
  check("Shift + hover spotlights the wall group", shiftHighlight === 1, String(shiftHighlight));
  await call("Input.dispatchKeyEvent", { type: "keyUp", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16 });
  await sleep(80);
  check("releasing Shift drops the spotlight", await evaluate(`highlightMask() === null`));

  // ---- settings are applied and remembered --------------------------------
  await evaluate(`(() => {
    const input = document.getElementById("f-size");
    input.value = "12";
    document.getElementById("btn-apply").click();
  })()`);
  await waitFor(`!ui.busy && ui.cols === 12`, "applying a new size regenerates the board", 90000);
  const resized = await evaluate(`({ cols: ui.cols, rows: ui.rows, heading: document.getElementById("title").textContent, saved: localStorage.getItem("tapa.settings") })`);
  check("a new board size is applied", resized.cols === 12 && resized.rows === 12, JSON.stringify(resized));
  check("the heading follows the size", resized.heading === "Tapa 12x12", resized.heading);
  check("settings are remembered in localStorage", /size = 12/.test(resized.saved || ""), resized.saved);

  // ---- the zoom slider changes how big the board is drawn ------------------
  const beforeZoom = await evaluate(`({ cell: ui.cell, cols: ui.cols, slider: document.getElementById("zoom-slider").value })`);
  check(
    "the zoom slider shows the current cell size",
    Number(beforeZoom.slider) === beforeZoom.cell,
    JSON.stringify(beforeZoom),
  );

  await evaluate(`(() => {
    const slider = document.getElementById("zoom-slider");
    slider.value = "64";
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await sleep(120);
  const zoomedIn = await evaluate(`({
    cell: ui.cell,
    cols: ui.cols,
    canvas: document.getElementById("board").style.width,
    zoom: ui.zoom,
  })`);
  check("dragging the zoom slider enlarges the cells", zoomedIn.cell === 64, JSON.stringify(zoomedIn));
  check("zooming does not change the board size", zoomedIn.cols === beforeZoom.cols, JSON.stringify(zoomedIn));
  check("the canvas grows with the zoom", zoomedIn.canvas === "768px", zoomedIn.canvas);

  await evaluate(`(() => {
    const slider = document.getElementById("zoom-slider");
    slider.value = "18";
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await sleep(120);
  const zoomedOut = await evaluate(`({ cell: ui.cell, canvas: document.getElementById("board").style.width })`);
  check("dragging the zoom slider shrinks the cells", zoomedOut.cell === 18, JSON.stringify(zoomedOut));

  await evaluate(`document.getElementById("btn-fit").click()`);
  await sleep(150);
  const fitted = await evaluate(`({ cell: ui.cell, zoom: ui.zoom, slider: document.getElementById("zoom-slider").value })`);
  check("Fit goes back to filling the window", fitted.zoom === null && fitted.cell !== 18, JSON.stringify(fitted));
  check(
    "the slider follows the fitted size",
    Number(fitted.slider) === fitted.cell,
    JSON.stringify(fitted),
  );

  // ---- the board-size slider rebuilds the board ---------------------------
  await evaluate(`(() => {
    const input = document.getElementById("f-size");
    input.value = "16";
    document.getElementById("btn-apply").click();
  })()`);
  await waitFor(`!ui.busy && ui.cols === 16`, "a new board size rebuilds the board", 90000);
  const sliderState = await evaluate(`({
    cols: ui.cols,
    field: document.getElementById("f-size").value,
    heading: document.getElementById("title").textContent,
  })`);
  check("a new board size rebuilds the board", sliderState.cols === 16, JSON.stringify(sliderState));
  check("the size field keeps the applied value", sliderState.field === "16", sliderState.field);
  check("the heading follows the size", sliderState.heading === "Tapa 16x16", sliderState.heading);

  // ---- print and bench run in wasm and show their output ------------------
  await evaluate(`document.getElementById("btn-bench").click()`);
  // the worker is now crunching: the page must still answer instantly
  await sleep(250);
  const pingStart = Date.now();
  await evaluate(`document.title`);
  const latency = Date.now() - pingStart;
  check("the page stays responsive while benching", latency < 500, `${latency} ms`);
  await waitFor(`!ui.busy && !document.getElementById("output-wrap").hidden`, "bench finishes", 180000);
  await sleep(120);
  const benchText = await evaluate(`document.getElementById("output").textContent`);
  check("bench prints per-seed timings", /seed \d+:/.test(benchText), benchText.slice(0, 120));
  check("bench verifies its puzzles", /clues \d+\.\.\d+/.test(benchText), benchText.slice(-160));

  await evaluate(`document.getElementById("btn-print").click()`);
  await waitFor(`!ui.busy`, "print finishes", 180000);
  await sleep(200);
  const printText = await evaluate(`document.getElementById("output").textContent`);
  check("print shows a board and its solution", /solution:/.test(printText), printText.slice(0, 120));
  check("print verifies uniqueness", /solutions=1/.test(printText), printText.slice(-200));

  // ---- no stray JavaScript errors ----------------------------------------
  check("no uncaught JavaScript errors", consoleErrors.length === 0, consoleErrors.join(" | "));

  // ---- the layout is sane at this window size -----------------------------
  const layout = await evaluate(`(() => {
    const box = (selector) => {
      const rect = document.querySelector(selector).getBoundingClientRect();
      return {
        left: Math.round(rect.left), top: Math.round(rect.top),
        right: Math.round(rect.right), bottom: Math.round(rect.bottom),
        width: Math.round(rect.width), height: Math.round(rect.height),
      };
    };
    return {
      canvas: box("#board"), panel: box(".panel"), output: box("#output-wrap"),
      viewport: { w: innerWidth, h: innerHeight },
    };
  })()`);
  check(
    "the board sits left of the control column",
    layout.canvas.right <= layout.panel.left + 1,
    JSON.stringify(layout.canvas) + " vs " + JSON.stringify(layout.panel),
  );
  check(
    "the board fits the viewport",
    layout.canvas.bottom <= layout.viewport.h && layout.canvas.right <= layout.viewport.w,
    JSON.stringify(layout),
  );
  check(
    "the control column fits the viewport",
    layout.panel.bottom <= layout.viewport.h + 1,
    JSON.stringify(layout.panel),
  );
  check(
    "the output panel is below the board area",
    layout.output.top >= layout.canvas.bottom,
    JSON.stringify(layout.output),
  );

  // ---- a screenshot of the final state ------------------------------------
  const shot = await call("Page.captureScreenshot", { format: "png" });
  const file = path.join(__dirname, "..", "web-check.png");
  fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
  console.log(`  screenshot: ${file}`);

  console.log(failures === 0 ? "\nall browser checks passed" : `\n${failures} browser check(s) failed`);
  socket.close();
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error(`browser check crashed: ${err.message}`);
  process.exit(1);
});
