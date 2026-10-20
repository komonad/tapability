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
    statusKey: ui.state.statusKey,
    info: document.getElementById("info").textContent,
    stats: document.getElementById("stats").textContent,
    genStats: ui.state.genStats,
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
  check(
    "status reports cells left",
    summary.statusKey === "cells_left",
    summary.statusKey,
  );
  check("clues are present", summary.clues > 0, String(summary.clues));
  const clueOrder = await evaluate(`(() => {
    let bad = 0;
    for (const text of ui.clues.values()) {
      const digits = [...text].map(Number);
      if (digits.some((n) => n < 1 || n > 8)) bad += 1;
      for (let i = 1; i < digits.length; i += 1) if (digits[i] < digits[i - 1]) bad += 1;
    }
    return bad;
  })()`);
  check("clue numbers are shown smallest first", clueOrder === 0, String(clueOrder));
  check("footer does not leak the wall count", !summary.hasWallCount, summary.info);
  check(
    "footer shows the seed and clue count",
    summary.info.includes(String(summary.seed)) && summary.info.includes(String(summary.clues)),
    summary.info,
  );
  check(
    "generation stats are structured",
    summary.genStats && summary.genStats.clues === summary.clues && summary.genStats.w > 0,
    JSON.stringify(summary.genStats),
  );
  check("canvas has a backing store", summary.canvasW > 100, String(summary.canvasW));
  check("settings column has every field", summary.fields === 5, String(summary.fields));

  const guideLayout = await evaluate(`(() => {
    const guide = document.querySelector('.guide');
    const stage = document.getElementById('stage').getBoundingClientRect();
    const board = document.getElementById('board').getBoundingClientRect();
    const bounds = guide.getBoundingClientRect();
    return { left: bounds.right <= stage.left, hasIntro: guide.contains(document.getElementById('intro')),
      hasHelp: guide.contains(document.getElementById('help')),
      centered: Math.abs((board.left + board.right) - (stage.left + stage.right)) < 3 };
  })()`);
  check("rules and help occupy the left column", guideLayout.left && guideLayout.hasIntro && guideLayout.hasHelp, JSON.stringify(guideLayout));
  check("the board is centered in its stage", guideLayout.centered, JSON.stringify(guideLayout));
  const sliderSync = await evaluate(`(() => {
    const slider = document.querySelector('[data-setting-slider="difficulty"]');
    slider.value = '5'; slider.dispatchEvent(new Event('input', {bubbles:true}));
    const changed = document.getElementById('f-difficulty').value;
    fillFromText(ui.state.settingsText);
    return {changed, restored: slider.value};
  })()`);
  check("difficulty slider updates its field and restores settings", sliderSync.changed === '5' && sliderSync.restored === '3', JSON.stringify(sliderSync));

  const rangeControl = await evaluate(`(() => {
    const row = document.querySelector('[data-field=density]');
    const handles = [...row.querySelectorAll('.density-range input[type=range]')];
    const sameTrack = handles.length === 2 && handles[0].parentNode === handles[1].parentNode;
    handles[0].value = '0.4'; handles[0].dispatchEvent(new Event('input', {bubbles:true}));
    handles[1].value = '0.7'; handles[1].dispatchEvent(new Event('input', {bubbles:true}));
    const changed = row.querySelector('input[data-key]').value;
    const display = row.querySelector('output').value;
    handles[0].value = '0.75'; handles[0].dispatchEvent(new Event('input', {bubbles:true}));
    const ordered = Number(handles[0].value) <= Number(handles[1].value);
    fillFromText(ui.state.settingsText);
    return {sameTrack,changed,display,ordered,restored:row.querySelector('output').value};
  })()`);
  check("density has one shared track with ordered handles and percentages", rangeControl.sameTrack && rangeControl.ordered && rangeControl.changed === '0.40-0.70' && rangeControl.display === '40%–70%' && rangeControl.restored === '50%–60%', JSON.stringify(rangeControl));

  const densityBox = await evaluate(`(() => {
    const range = document.querySelector('.density-range');
    range.scrollIntoView({block:'nearest'});
    const r = range.getBoundingClientRect();
    return {x:r.left,y:r.top+r.height/2,w:r.width};
  })()`);
  const dragDensity = async (from, to) => {
    const point = value => ({x:densityBox.x+(value-0.05)/0.70*densityBox.w,y:densityBox.y});
    await call("Input.dispatchMouseEvent", {type:"mouseMoved",...point(from)});
    await call("Input.dispatchMouseEvent", {type:"mousePressed",...point(from),button:"left",buttons:1,clickCount:1});
    await call("Input.dispatchMouseEvent", {type:"mouseMoved",...point(to),buttons:1});
    await call("Input.dispatchMouseEvent", {type:"mouseReleased",...point(to),button:"left",buttons:0,clickCount:1});
  };
  await dragDensity(0.5,0.4);
  check("density endpoints respond to pointer drags on the shared track",
    (await evaluate(`document.getElementById('f-density').value`)) === '0.40-0.60');
  await evaluate(`fillFromText(ui.state.settingsText.replace(/^density = .*$/m,'density = 0.50-0.50'))`);
  await dragDensity(0.5,0.45);
  check("overlapping range handles can separate to the left",
    (await evaluate(`document.getElementById('f-density').value`)) === '0.45-0.50');
  await evaluate(`fillFromText(ui.state.settingsText); document.querySelector('.panel').scrollTop=0`);

  // ---- language: Simplified Chinese by default, switchable, remembered ----
  const zh = await evaluate(`({
    htmlLang: document.documentElement.lang,
    select: document.getElementById("lang").value,
    intro: document.getElementById("intro").textContent.trim(),
    newButton: document.querySelector("#btn-new span").textContent.trim(),
    panelTitle: document.querySelector(".panel .panel-title").textContent.trim(),
    status: document.getElementById("status").textContent,
    stats: document.getElementById("stats").textContent,
    saved: localStorage.getItem("tapa.lang"),
  })`);
  check(
    "the page starts in Simplified Chinese",
    zh.htmlLang === "zh-CN" && zh.select === "zh-CN",
    JSON.stringify({ lang: zh.htmlLang, select: zh.select }),
  );
  check("the rules introduction is available", zh.intro.length > 40, zh.intro);
  check("the introduction is in Chinese", /[\u4e00-\u9fff]/.test(zh.intro), zh.intro);
  check("the introduction describes the rules", /数字|黑格/.test(zh.intro), zh.intro);
  check("buttons are translated", zh.newButton === "新题目", zh.newButton);
  check("panel headings are translated", zh.panelTitle === "操作", zh.panelTitle);
  check("the status line is translated", /[\u4e00-\u9fff]/.test(zh.status), zh.status);
  check("the generation stats line is translated", /[\u4e00-\u9fff]/.test(zh.stats), zh.stats);

  await evaluate(`(() => {
    const select = document.getElementById("lang");
    select.value = "en";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  })()`);
  await sleep(200);
  const en = await evaluate(`({
    htmlLang: document.documentElement.lang,
    intro: document.getElementById("intro").textContent.trim(),
    newButton: document.querySelector("#btn-new span").textContent.trim(),
    status: document.getElementById("status").textContent,
    stats: document.getElementById("stats").textContent,
    help: document.getElementById("help").textContent.trim(),
    sizeLabel: document.querySelector('[data-field="size"] label').textContent.trim(),
    saved: localStorage.getItem("tapa.lang"),
  })`);
  check("switching to English translates the introduction", /^Fill every cell/.test(en.intro), en.intro);
  check("switching to English translates the buttons", en.newButton === "New puzzle", en.newButton);
  check("switching to English translates the settings labels", en.sizeLabel === "Board size (3-60)", en.sizeLabel);
  check("switching to English translates the status", /cell/.test(en.status), en.status);
  check("switching to English translates the stats line", /last generation/.test(en.stats), en.stats);
  check("the language choice is remembered", en.saved === "en", String(en.saved));

  // back to Chinese for the rest of the run and the screenshot
  await evaluate(`(() => {
    const select = document.getElementById("lang");
    select.value = "zh-CN";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  })()`);
  await sleep(200);
  check(
    "switching back restores Chinese",
    await evaluate(`document.getElementById("intro").textContent.includes("黑格")`),
  );

  // a half-cached page must repair itself instead of waiting for a click
  const repaired = await evaluate(`(() => {
    for (const element of document.querySelectorAll("[data-i18n]")) element.textContent = "";
    for (const label of document.querySelectorAll(".field label")) label.textContent = "";
    const fixed = ensureTranslated();
    const stillEmpty = [...document.querySelectorAll("[data-i18n], .field label")].filter(
      (element) => element.textContent.trim().length === 0,
    ).length;
    return { fixed, stillEmpty, button: document.querySelector("#btn-new span").textContent.trim() };
  })()`);
  check(
    "the page repairs missing button text by itself",
    repaired.stillEmpty === 0 && repaired.button.length > 0,
    JSON.stringify(repaired),
  );

  check("the bundled Inter font is loaded",await evaluate(`document.fonts.check('600 24px "Inter"') && getComputedStyle(document.body).fontFamily.includes("Inter")`));

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

  // ---- undecided and empty cells share a background -----------------------
  const bgCells = await evaluate(`(() => {
    const free = [];
    for (let i = 0; i < ui.cells.length; i += 1) {
      if (!ui.clues.has(i) && ui.cells[i] === 0) free.push(i);
    }
    return { unknown: free[0], empty: free[1] };
  })()`);
  await evaluate(`(async () => {
    const x = ${bgCells.empty} % ui.cols;
    const y = Math.floor(${bgCells.empty} / ui.cols);
    const state = await send(\`paint begin \${x} \${y} 0\`);
    applyState(state);
    await send("paint end");
  })()`);
  await sleep(200);
  const bg = await evaluate(`(() => {
    const ctx = document.getElementById("board").getContext("2d");
    const dpr = ui.dpr, cell = ui.cell;
    const at = (i, ox, oy) => {
      const x = (i % ui.cols) * cell, y = Math.floor(i / ui.cols) * cell;
      const d = ctx.getImageData(Math.round((x + ox) * dpr), Math.round((y + oy) * dpr), 1, 1).data;
      return [d[0], d[1], d[2]];
    };
    return {
      unknown: at(${bgCells.unknown}, 3, 3),
      empty: at(${bgCells.empty}, 3, 3),
      dot: at(${bgCells.empty}, cell / 2, cell / 2),
    };
  })()`);
  check(
    "an empty mark keeps the undecided background",
    JSON.stringify(bg.unknown) === JSON.stringify(bg.empty),
    JSON.stringify(bg),
  );
  check(
    "an empty mark is just a dot",
    JSON.stringify(bg.dot) !== JSON.stringify(bg.empty),
    JSON.stringify(bg),
  );

  // ---- no grid line between two touching walls ----------------------------
  const wallPair = await evaluate(`(() => {
    for (let y = 0; y < ui.rows; y += 1) {
      for (let x = 0; x + 1 < ui.cols; x += 1) {
        const a = y * ui.cols + x, b = a + 1;
        if (!ui.clues.has(a) && !ui.clues.has(b) && ui.cells[a] === 0 && ui.cells[b] === 0) {
          return { a, b, x, y };
        }
      }
    }
    return null;
  })()`);
  await evaluate(`(async () => {
    const state = await send(\`paint begin ${wallPair.x} ${wallPair.y} 1\`);
    applyState(state);
    const second = await send(\`paint begin ${wallPair.x + 1} ${wallPair.y} 1\`);
    applyState(second);
    await send("paint end");
  })()`);
  await sleep(250);
  const edges = await evaluate(`(() => {
    const ctx = document.getElementById("board").getContext("2d");
    const dpr = ui.dpr, cell = ui.cell;
    const at = (px, py) => {
      const d = ctx.getImageData(Math.round(px * dpr), Math.round(py * dpr), 1, 1).data;
      return [d[0], d[1], d[2]];
    };
    const sharedX = (${wallPair.x} + 1) * cell;
    const rowY = (${wallPair.y} + 0.5) * cell;
    // a plain/plain edge well away from the spotlighted group
    let plainPixel = null;
    for (let y = 0; y < ui.rows && !plainPixel; y += 1) {
      for (let x = 0; x + 1 < ui.cols && !plainPixel; x += 1) {
        const a = y * ui.cols + x, b = a + 1;
        if (ui.clues.has(a) || ui.clues.has(b)) continue;
        if (ui.cells[a] !== 0 || ui.cells[b] !== 0) continue;
        if (Math.abs(x - ${wallPair.x}) + Math.abs(y - ${wallPair.y}) < 3) continue;
        plainPixel = at((x + 1) * cell, (y + 0.5) * cell);
      }
    }
    return { shared: at(sharedX, rowY), plain: plainPixel };
  })()`);
  const isGridLine = (c) =>
    c && Math.abs(c[0] - 168) < 20 && Math.abs(c[1] - 174) < 20 && Math.abs(c[2] - 188) < 20;
  check(
    "no grid line between two touching walls",
    !isGridLine(edges.shared),
    JSON.stringify(edges.shared),
  );
  check(
    "grid lines are still drawn next to plain cells",
    isGridLine(edges.plain),
    JSON.stringify(edges.plain),
  );

  // leave the board clean for the painting tests that follow
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "the board is cleared again");
  await sleep(100);

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

  // ---- a block of walls has no grid lines inside --------------------------
  const blockAt = await evaluate(`(() => {
    for (let y = 0; y + 2 < ui.rows; y += 1) {
      for (let x = 0; x + 2 < ui.cols; x += 1) {
        let ok = true;
        for (let dy = 0; dy < 3 && ok; dy += 1) {
          for (let dx = 0; dx < 3 && ok; dx += 1) {
            const i = (y + dy) * ui.cols + x + dx;
            if (ui.clues.has(i) || ui.cells[i] !== 0) ok = false;
          }
        }
        if (ok) return { x, y };
      }
    }
    return null;
  })()`);
  await evaluate(`(async () => {
    for (let dy = 0; dy < 3; dy += 1) {
      for (let dx = 0; dx < 3; dx += 1) {
        const state = await send(\`paint begin \${${blockAt.x} + dx} \${${blockAt.y} + dy} 1\`);
        applyState(state);
      }
    }
    await send("paint end");
  })()`);
  await sleep(300);
  const blockScan = await evaluate(`(() => {
    const ctx = document.getElementById("board").getContext("2d");
    const dpr = ui.dpr, cell = ui.cell;
    const isGrid = (d) => Math.abs(d[0] - 168) < 22 && Math.abs(d[1] - 174) < 22 && Math.abs(d[2] - 188) < 22;
    const at = (px, py) => {
      const d = ctx.getImageData(Math.round(px * dpr), Math.round(py * dpr), 1, 1).data;
      return [d[0], d[1], d[2]];
    };
    const bad = [];
    for (let dy = 0; dy < 3; dy += 1) {
      for (let dx = 1; dx < 3; dx += 1) {
        const px = (${blockAt.x} + dx) * cell, py = (${blockAt.y} + dy + 0.5) * cell;
        if (isGrid(at(px, py))) bad.push("v" + dx + "," + dy);
      }
    }
    for (let dy = 1; dy < 3; dy += 1) {
      for (let dx = 0; dx < 3; dx += 1) {
        const px = (${blockAt.x} + dx + 0.5) * cell, py = (${blockAt.y} + dy) * cell;
        if (isGrid(at(px, py))) bad.push("h" + dx + "," + dy);
      }
    }
    return bad;
  })()`);
  check(
    "a 3x3 wall block has no grid lines inside",
    blockScan.length === 0,
    JSON.stringify(blockScan),
  );
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "the block is cleared again");
  await sleep(100);

  // ---- isolation: only two groups that can never meet are flagged ---------
  const sealed = await evaluate(`(async () => {
    let target = null;
    for (let y = 1; y + 1 < ui.rows && !target; y += 1) {
      for (let x = 1; x + 1 < ui.cols && !target; x += 1) {
        const i = y * ui.cols + x;
        if (!ui.clues.has(i) && ui.cells[i] === 0) target = { i, x, y };
      }
    }
    let state = await send(\`paint begin \${target.x} \${target.y} 1\`);
    for (let dy = -1; dy <= 1; dy += 1) {
      for (let dx = -1; dx <= 1; dx += 1) {
        if (!dx && !dy) continue;
        const j = (target.y + dy) * ui.cols + (target.x + dx);
        if (ui.clues.has(j)) continue;   // clue cells are empty already
        state = await send(\`paint begin \${j % ui.cols} \${Math.floor(j / ui.cols)} 0\`);
      }
    }
    state = await send("paint end");
    applyState(state);
    return { i: target.i, mark: state.cells[target.i], error: state.errorsCells[target.i] };
  })()`);
  check(
    "a lone wall group is not flagged, even sealed in",
    sealed.mark === "1" && sealed.error === "0",
    JSON.stringify(sealed),
  );

  // a second wall group elsewhere makes the sealed one stranded
  const second = await evaluate(`(async () => {
    const sr = Math.floor(${sealed.i} / ui.cols);
    const sc = ${sealed.i} % ui.cols;
    let pick = null;
    for (let i = ui.cells.length - 1; i >= 0 && !pick; i -= 1) {
      if (ui.clues.has(i) || ui.cells[i] !== 0) continue;
      const x = i % ui.cols, y = Math.floor(i / ui.cols);
      if (Math.abs(x - sc) + Math.abs(y - sr) < 4) continue;
      // it has to sit in the open area: a wall boxed in by clue cells would be
      // stranded on its own account, which is not what this check is about
      let open = 0;
      for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const nx = x + dx, ny = y + dy;
        if (nx < 0 || ny < 0 || nx >= ui.cols || ny >= ui.rows) continue;
        const nb = ny * ui.cols + nx;
        if (!ui.clues.has(nb) && ui.cells[nb] === 0) open += 1;
      }
      if (open >= 2) pick = { i, x, y, open };
    }
    if (!pick) return { i: -1, open: 0, sealed: "0", far: "1" };
    let state = await send(\`paint begin \${pick.x} \${pick.y} 1\`);
    state = await send("paint end");
    applyState(state);
    return {
      i: pick.i,
      open: pick.open,
      sealed: state.errorsCells[${sealed.i}],
      far: state.errorsCells[pick.i],
    };
  })()`);
  check(
    "the sealed group is flagged once another wall group exists",
    second.sealed === "1" && second.far === "0",
    JSON.stringify(second),
  );
  const redOutline = await evaluate(`(() => {
    const ctx = document.getElementById("board").getContext("2d");
    const dpr = ui.dpr, cell = ui.cell;
    const x = (${sealed.i} % ui.cols) * cell, y = Math.floor(${sealed.i} / ui.cols) * cell;
    const data = ctx.getImageData(
      Math.round(x * dpr), Math.round(y * dpr), Math.round(cell * dpr), Math.round(cell * dpr),
    ).data;
    let n = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (Math.abs(data[i] - 226) < 40 && Math.abs(data[i + 1] - 52) < 40 && Math.abs(data[i + 2] - 52) < 40) {
        n += 1;
      }
    }
    return n;
  })()`);
  check(
    "the stranded wall is outlined in red",
    redOutline > 0,
    `${redOutline} red pixels`,
  );
  const beforeClearGroup=await evaluate(`ui.state.cells`);
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "the sealed wall is cleared again");
  await sleep(100);

  // ---- undo restores the cleared board ----------------------------------
  await evaluate(`els.canvas.focus()`);
  check("the board can take keyboard focus",await evaluate(`document.activeElement===els.canvas`));
  await key("z");
  await sleep(80);
  const afterUndo = await evaluate(`ui.cells[${from + 3}]`);
  const afterClearUndo=await evaluate(`ui.state.cells`);
  check("Z undoes the complete reset",afterClearUndo===beforeClearGroup,JSON.stringify({diffs:[...afterClearUndo].flatMap((v,i)=>v!==beforeClearGroup[i]?[[i,beforeClearGroup[i],v]]:[])}));

  // ---- instant commands never veil the board ------------------------------
  // sample the veil flag on every repaint while undo runs
  await evaluate(`(() => {
    window.__veilSamples = [];
    window.__originalRender = window.render;
    window.render = function () {
      window.__veilSamples.push(Boolean(ui.veil));
      window.__originalRender();
    };
  })()`);
  await evaluate(`document.getElementById("btn-undo").click()`);
  await waitFor(`!ui.busy`, "undo finishes");
  await sleep(200);
  const veilSamples = await evaluate(`window.__veilSamples`);
  await evaluate(`(() => {
    window.render = window.__originalRender;
    delete window.__originalRender;
  })()`);
  check(
    "undo never covers the board",
    veilSamples.length > 0 && !veilSamples.includes(true),
    JSON.stringify(veilSamples),
  );

  // a genuinely slow operation does cover it, after a short delay
  await evaluate(`setBusy(true, { veil: true })`);
  await sleep(400);
  const veiled = await evaluate(`ui.veil`);
  await evaluate(`setBusy(false)`);
  await sleep(100);
  check("a slow operation covers the board", veiled === true, String(veiled));
  check("the cover goes away again", (await evaluate(`ui.veil`)) === false);

  await evaluate(`run("reset")`);

  // ---- stepping a single clue: middle click, Alt + click, and the button ---
  const clueIndex = await evaluate(`[...ui.clues.keys()][0]`);
  const clueText = await evaluate(`ui.clues.get(${clueIndex})`);
  const clueKeys = ["clue_filled", "clue_forces_nothing"];
  const clearStatus = () => evaluate(`setStatus("(cleared)", "")`);

  await clearStatus();
  await mouse("mousePressed", clueIndex, "middle");
  await mouse("mouseReleased", clueIndex, "middle");
  await sleep(250);
  const middleKey = await evaluate(`ui.state.statusKey`);
  check(
    "middle click steps the clue under the cursor",
    clueKeys.includes(middleKey),
    `${middleKey} (clue ${clueText})`,
  );

  await clearStatus();
  const alt = { ...point(clueIndex), button: "left", clickCount: 1, modifiers: 1 };
  await call("Input.dispatchMouseEvent", { type: "mousePressed", ...alt, buttons: 1 });
  await call("Input.dispatchMouseEvent", { type: "mouseReleased", ...alt, buttons: 0 });
  await sleep(250);
  check(
    "Alt + click steps the clue too",
    clueKeys.includes(await evaluate(`ui.state.statusKey`)),
    await evaluate(`ui.state.statusKey`),
  );

  await clearStatus();
  await mouse("mouseMoved", clueIndex, "none");
  await sleep(80);
  await evaluate(`document.getElementById("btn-cluestep").click()`);
  await sleep(250);
  check(
    "the Step one clue button steps the hovered clue",
    clueKeys.includes(await evaluate(`ui.state.statusKey`)),
    await evaluate(`ui.state.statusKey`),
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
  check(
    "stepping a non-clue cell is refused",
    (await evaluate(`ui.state.statusKey`)) === "clue_only",
    await evaluate(`ui.state.statusKey`),
  );

  // ---- one step, check, solution -----------------------------------------
  // start from a clean board so every mark on it comes from the deduction
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "clear finishes");
  await evaluate(`document.getElementById("btn-onestep").click()`);
  await waitFor(`!ui.busy`, "one step finishes");
  await sleep(80);
  const oneStepKey = await evaluate(`ui.state.statusKey`);
  check("one step fills the clues' deductions", oneStepKey === "one_step_filled", oneStepKey);

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
  const checkedKey = await evaluate(`ui.state.statusKey`);
  check("check reports on the marks", ["check_ok", "check_wrong"].includes(checkedKey), checkedKey);

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

  // ---- clue digits grey out once their numbers are met --------------------
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "clear finishes");
  // a big cell size, so the digits are wide enough to sample
  await evaluate(`(() => {
    const slider = document.getElementById("zoom-slider");
    slider.value = "64";
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await sleep(150);

  /** Grey / dark digit pixels inside one cell of the canvas. */
  const clueInk = (index) => `(() => {
    const canvas = document.getElementById("board");
    const ctx = canvas.getContext("2d");
    const dpr = window.devicePixelRatio || 1;
    const idx = ${index};
    const x = (idx % ui.cols) * ui.cell;
    const y = Math.floor(idx / ui.cols) * ui.cell;
    const d = ctx.getImageData(
      Math.round(x * dpr),
      Math.round(y * dpr),
      Math.round(ui.cell * dpr),
      Math.round(ui.cell * dpr),
    ).data;
    let grey = 0;
    let dark = 0;
    for (let i = 0; i < d.length; i += 4) {
      if (Math.abs(d[i] - 143) < 12 && Math.abs(d[i + 1] - 146) < 12 && Math.abs(d[i + 2] - 156) < 12) grey += 1;
      if (Math.abs(d[i] - 48) < 12 && Math.abs(d[i + 1] - 48) < 12 && Math.abs(d[i + 2] - 40) < 12) dark += 1;
    }
    return { grey, dark };
  })()`;

  // the clue with the fewest walls around it in the solution, so the check has
  // as few cells to paint as possible
  const met = await evaluate(`(async () => {
    const revealed = await send("solution");
    const solution = Array.from(decodeDigits(revealed.solution));
    await send("solution");
    let best = { idx: -1, walls: [] };
    for (const idx of ui.clues.keys()) {
      const x = idx % ui.cols;
      const y = Math.floor(idx / ui.cols);
      const walls = [];
      for (let dy = -1; dy <= 1; dy += 1) {
        for (let dx = -1; dx <= 1; dx += 1) {
          if (dx === 0 && dy === 0) continue;
          const nx = x + dx;
          const ny = y + dy;
          if (nx < 0 || ny < 0 || nx >= ui.cols || ny >= ui.rows) continue;
          const nb = ny * ui.cols + nx;
          if (solution[nb] === 1) walls.push(nb);
        }
      }
      if (walls.length && (best.idx < 0 || walls.length < best.walls.length)) best = { idx, walls };
    }
    return best;
  })()`);
  check("a clue with walls around it was found", met.idx >= 0, JSON.stringify(met));

  const inkBefore = await evaluate(clueInk(met.idx));
  await evaluate(`(async () => {
    for (const nb of ${JSON.stringify(met.walls)}) {
      await run("paint begin " + (nb % ui.cols) + " " + Math.floor(nb / ui.cols) + " 1");
      await run("paint end");
    }
  })()`);
  await sleep(150);
  const inkAfter = await evaluate(clueInk(met.idx));
  check("the engine calls that clue met", (await evaluate(`ui.doneClues[${met.idx}]`)) === 1);
  check(
    "a clue whose numbers are met turns grey",
    inkBefore.dark > 0 && inkBefore.grey === 0 && inkAfter.grey > 0 && inkAfter.dark === 0,
    `${JSON.stringify(inkBefore)} -> ${JSON.stringify(inkAfter)}`,
  );
  const stillOpen = await evaluate(`(() => {
    let open = 0;
    for (const idx of ui.clues.keys()) if (!ui.doneClues[idx]) open += 1;
    return open;
  })()`);
  check("clues that are still open stay dark", stillOpen > 0, String(stillOpen));

  // ---- a three-number clue centres the digit underneath -------------------
  const threeDigit = await evaluate(`(async () => {
    for (let attempt = 0; attempt < 6; attempt += 1) {
      for (const [idx, text] of ui.clues) if (text.length === 3) return idx;
      document.getElementById("btn-new").click();
      for (let i = 0; i < 300 && ui.busy; i += 1) await new Promise((r) => setTimeout(r, 50));
      await new Promise((r) => setTimeout(r, 80));
    }
    return -1;
  })()`);
  if (threeDigit >= 0) {
    const lower = await evaluate(`(() => {
      const canvas = document.getElementById("board");
      const ctx = canvas.getContext("2d");
      const dpr = window.devicePixelRatio || 1;
      const idx = ${threeDigit};
      const whole = Math.round(ui.cell * dpr);
      const pad = Math.round(3 * dpr);
      const x0 = Math.round((idx % ui.cols) * ui.cell * dpr) + pad;
      const y0 = Math.round(Math.floor(idx / ui.cols) * ui.cell * dpr) + Math.round(whole / 2) + pad;
      const w = whole - 2 * pad;
      const h = whole - Math.round(whole / 2) - 2 * pad;
      const d = ctx.getImageData(x0, y0, w, h).data;
      let minX = w;
      let maxX = -1;
      for (let py = 0; py < h; py += 1) {
        for (let px = 0; px < w; px += 1) {
          const i = (py * w + px) * 4;
          if (d[i] < 200 && d[i + 2] < 200) { if (px < minX) minX = px; if (px > maxX) maxX = px; }
        }
      }
      return { w, h, minX, maxX };
    })()`);
    const offset = lower.maxX < 0 ? Number.POSITIVE_INFINITY : Math.abs((lower.minX + lower.maxX) / 2 - lower.w / 2);
    check(
      "a three-number clue centres the digit below",
      lower.maxX >= 0 && offset <= lower.w * 0.12,
      JSON.stringify(lower),
    );
  } else {
    console.log("  skip a three-number clue centres the digit below (no such clue in six puzzles)");
  }
  await evaluate(`document.getElementById("btn-clear").click()`);
  await waitFor(`!ui.busy`, "clear finishes");
  // back to the fitted cell size the later checks expect
  await evaluate(`document.getElementById("btn-fit").click()`);
  await sleep(150);

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

  // ---- touch: a tap cycles the mark, a drag paints ------------------------
  // the board was resized and zoomed since `point()` was built, so re-measure
  const geom2 = await evaluate(`(() => {
    const r = document.getElementById("board").getBoundingClientRect();
    return { left: r.left, top: r.top, cell: ui.cell, cols: ui.cols };
  })()`);
  const point2 = (index) => ({
    x: geom2.left + (index % geom2.cols) * geom2.cell + geom2.cell / 2,
    y: geom2.top + Math.floor(index / geom2.cols) * geom2.cell + geom2.cell / 2,
  });

  await call("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
  const touchCell = await evaluate(`(() => {
    for (let i = 0; i < ui.cells.length; i += 1) {
      if (!ui.clues.has(i) && ui.cells[i] === 0) return i;
    }
    return -1;
  })()`);
  const touchPoint = point2(touchCell);
  const touch = (type, x, y) =>
    call("Input.dispatchTouchEvent", {
      type,
      touchPoints: type === "touchEnd" ? [] : [{ x, y, id: 1 }],
    });

  await touch("touchStart", touchPoint.x, touchPoint.y);
  await touch("touchEnd");
  await sleep(150);
  check(
    "the first tap marks a wall",
    (await evaluate(`ui.cells[${touchCell}]`)) === 1,
    String(await evaluate(`ui.cells[${touchCell}]`)),
  );

  await touch("touchStart", touchPoint.x, touchPoint.y);
  await touch("touchEnd");
  await sleep(150);
  check(
    "the second tap marks empty",
    (await evaluate(`ui.cells[${touchCell}]`)) === 2,
    String(await evaluate(`ui.cells[${touchCell}]`)),
  );

  await touch("touchStart", touchPoint.x, touchPoint.y);
  await touch("touchEnd");
  await sleep(150);
  check(
    "the third tap clears the cell",
    (await evaluate(`ui.cells[${touchCell}]`)) === 0,
    String(await evaluate(`ui.cells[${touchCell}]`)),
  );
  check(
    "a tap leaves the outline on the tapped cell",
    (await evaluate(`ui.hover`)) === touchCell,
    `${await evaluate(`ui.hover`)} vs ${touchCell}`,
  );
  const outline = await evaluate(`(() => {
    const ctx = document.getElementById("board").getContext("2d");
    const dpr = ui.dpr, cell = ui.cell;
    const x = (${touchCell} % ui.cols) * cell, y = Math.floor(${touchCell} / ui.cols) * cell;
    const data = ctx.getImageData(Math.round(x * dpr), Math.round(y * dpr), Math.round(cell * dpr), Math.round(cell * dpr)).data;
    let found = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (Math.abs(data[i] - 58) < 30 && Math.abs(data[i + 1] - 130) < 30 && Math.abs(data[i + 2] - 226) < 40) {
        found += 1;
      }
    }
    return { found, cell };
  })()`);
  check(
    "the outline is drawn around the tapped cell",
    outline.found > outline.cell,
    JSON.stringify(outline),
  );

  // a finger that travels is a stroke, not a tap
  const dragFrom = await evaluate(`(() => {
    for (let i = 0; i < ui.cells.length - 3; i += 1) {
      if (i % ui.cols > ui.cols - 4) continue;
      if ([i, i + 1, i + 2, i + 3].some((k) => ui.clues.has(k) || ui.cells[k] !== 0)) continue;
      return i;
    }
    return -1;
  })()`);
  check("found four free cells for a touch stroke", dragFrom >= 0, String(dragFrom));
  const dragStart = point2(dragFrom);
  const dragEnd = point2(dragFrom + 3);
  await touch("touchStart", dragStart.x, dragStart.y);
  for (let step = 1; step <= 4; step += 1) {
    await call("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [
        {
          x: dragStart.x + ((dragEnd.x - dragStart.x) * step) / 4,
          y: dragStart.y,
          id: 1,
        },
      ],
    });
  }
  await touch("touchEnd");
  await sleep(200);
  const touched = await evaluate(
    `[${dragFrom}, ${dragFrom + 1}, ${dragFrom + 2}, ${dragFrom + 3}].map((i) => ui.cells[i]).join("")`,
  );
  check("a travelling finger paints a stroke", touched === "1111", touched);

  // tapping a clue steps it, so the feature is reachable without a middle button
  const touchClue = await evaluate(`[...ui.clues.keys()][0]`);
  const touchCluePoint = point2(touchClue);
  await evaluate(`setStatus("(cleared)", "")`);
  await touch("touchStart", touchCluePoint.x, touchCluePoint.y);
  await touch("touchEnd");
  await sleep(250);
  const touchClueStatus = await evaluate(`ui.state.statusKey`);
  check(
    "tapping a clue steps it",
    ["clue_filled", "clue_forces_nothing"].includes(touchClueStatus),
    touchClueStatus,
  );
  check(
    "tapping a clue leaves the clue cell itself alone",
    (await evaluate(`ui.cells[${touchClue}]`)) === 2,
  );
  await call("Emulation.setTouchEmulationEnabled", { enabled: false });

  // ---- a phone-sized viewport keeps every control readable ----------------
  await call("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 844,
    deviceScaleFactor: 2,
    mobile: true,
  });
  await sleep(500);
  const phone = await evaluate(`(() => {
    const visible = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return r.width > 8 && r.height > 8 && cs.visibility === "visible" &&
        cs.display !== "none" && Number(cs.opacity) > 0.5;
    };
    return {
      buttons: [...document.querySelectorAll(".panel button:not([hidden])")].map((b) => ({
        id: b.id, text: (b.textContent || "").trim(), visible: visible(b),
      })),
      labels: [...document.querySelectorAll(".field label")].map((l) => ({
        text: (l.textContent || "").trim(), visible: visible(l),
      })),
      inputs: [...document.querySelectorAll(".field input:not([type=hidden])")].map((i) => visible(i)),
      intro: (document.getElementById("intro").textContent || "").trim(),
      panel: visible(document.querySelector(".panel")),
    };
  })()`);
  check("the control column is visible on a phone viewport", phone.panel);
  check(
    "every button is visible and labelled on a phone viewport",
    phone.buttons.length >= 10 && phone.buttons.every((b) => b.visible && b.text.length > 0),
    JSON.stringify(phone.buttons.filter((b) => !b.visible || !b.text)),
  );
  check(
    "every setting label is visible on a phone viewport",
    phone.labels.length >= 5 && phone.labels.every((l) => l.visible && l.text.length > 0),
    JSON.stringify(phone.labels.filter((l) => !l.visible || !l.text)),
  );
  check(
    "every setting input is visible on a phone viewport",
    phone.inputs.length >= 5 && phone.inputs.every(Boolean),
    JSON.stringify(phone.inputs),
  );
  check("the rules introduction survives a phone viewport", phone.intro.length > 40, phone.intro);
  await call("Emulation.clearDeviceMetricsOverride");
  await sleep(300);


  await evaluate(`send('set size 20')`); await evaluate(`generate(42)`);
  const roundStart=await evaluate(`ui.state.cells`);
  await evaluate(`run('onestep')`);
  const roundOne=await evaluate(`({cells:ui.state.cells,selection:ui.state.deductionCells})`);
  check("one-step selection contains exactly this round's changes",JSON.stringify(roundOne.selection)===JSON.stringify([...roundOne.cells].flatMap((v,i)=>v!==roundStart[i]?[i]:[])));
  await evaluate(`run('onestep')`);
  const roundTwo=await evaluate(`({cells:ui.state.cells,selection:ui.state.deductionCells})`);
  check("the next round replaces the deduction selection",JSON.stringify(roundTwo.selection)===JSON.stringify([...roundTwo.cells].flatMap((v,i)=>v!==roundOne.cells[i]?[i]:[])));
  const selectionPixels=await evaluate(`(() => {
    const ctx=els.canvas.getContext('2d'),counts={black:0,white:0,outlined:0};
    for(const i of ui.state.deductionCells){
      counts[ui.cells[i]===1?'black':'white']++;
      const pixels=ctx.getImageData(Math.round(i%ui.cols*ui.cell*ui.dpr),Math.round(Math.floor(i/ui.cols)*ui.cell*ui.dpr),Math.floor(ui.cell*ui.dpr),Math.floor(ui.cell*ui.dpr)).data;
      let found=false;
      for(let k=0;k<pixels.length;k+=4)if(Math.abs(pixels[k]-255)<12 && Math.abs(pixels[k+1]-196)<12 && Math.abs(pixels[k+2]-72)<12){found=true;break;}
      counts.outlined+=found?1:0;
    }
    return counts;
  })()`);
  check("both black and white deductions are visibly selected",selectionPixels.black>0 && selectionPixels.white>0 && selectionPixels.outlined===roundTwo.selection.length,JSON.stringify(selectionPixels));
  await evaluate(`run('undo')`);
  check("one undo restores the preceding round and selection",await evaluate(`ui.state.cells===${JSON.stringify(roundOne.cells)} && JSON.stringify(ui.state.deductionCells)===${JSON.stringify(JSON.stringify(roundOne.selection))}`));
  await evaluate(`run('undo')`);
  check("a second undo restores the board before any deduction",await evaluate(`ui.state.cells===${JSON.stringify(roundStart)} && ui.state.deductionCells.length===0 && !ui.state.canUndo`));

  // Hint previews are separate from player marks and survive only that position.
  await evaluate(`send('set size 6')`);
  await evaluate(`generate(42)`);
  const hintBefore=await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,canUndo:ui.state.canUndo})`);
  await evaluate(`run('hint')`);
  const hintPreview=await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,canUndo:ui.state.canUndo,hint:ui.state.hint,visible:!document.getElementById('hint-card').hidden,left:document.getElementById('hint-card').getBoundingClientRect().right<=document.getElementById('board').getBoundingClientRect().left,explanation:document.getElementById('hint-explanation').textContent,unknown:ui.state.solution})`);
  check("hint preview preserves marks and undo history",hintBefore.cells===hintPreview.cells && hintBefore.canUndo===hintPreview.canUndo && hintBefore.seed===hintPreview.seed);
  check("hint explanation appears on the left without revealing the solution",hintPreview.visible && hintPreview.left && hintPreview.explanation.length>0 && hintPreview.unknown==="");
  check("direct hint records human proof metrics",hintPreview.hint && hintPreview.hint.tier==='rule' && hintPreview.hint.proofMetrics.assumptionDepth===0);
  await evaluate(`run('hintapply')`);
  const hintAfter=await evaluate(`({cells:ui.state.cells,hint:ui.state.hint,hidden:document.getElementById('hint-card').hidden})`);
  check("applying a hint commits only its conclusions",hintPreview.hint.marks.every(([i,v])=>hintAfter.cells[i]===String(v)) && [...hintAfter.cells].every((v,i)=>v===hintBefore.cells[i] || hintPreview.hint.marks.some(([j])=>j===i)) && !hintAfter.hint && hintAfter.hidden);
  await evaluate(`run('undo')`);
  check("hinted marks can be undone",await evaluate(`ui.state.cells===${JSON.stringify(hintBefore.cells)}`));
  await evaluate(`run('hint')`); await evaluate(`run('hintdismiss')`);
  check("dismissing a hint preserves the board",await evaluate(`!ui.state.hint && ui.state.cells===${JSON.stringify(hintBefore.cells)}`));
  await evaluate(`run('hint')`);
  await evaluate(`(() => { const [i,v]=ui.state.hint.marks[0]; return run('paint begin '+(i%ui.cols)+' '+Math.floor(i/ui.cols)+' '+(v===1?1:0)); })()`);
  check("editing invalidates the hint preview",await evaluate(`!ui.state.hint && document.getElementById('hint-card').hidden`));
  await evaluate(`run('reset')`);
  const beforeCancel = await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,undo:ui.state.canUndo,size:ui.state.w})`);
  await evaluate(`send('set size 60')`);
  await evaluate(`void generate(${beforeCancel.seed + 1})`);
  await waitFor(`generationTask && ui.busy && !document.getElementById('btn-cancel-generation').hidden`, "generation can be cancelled");
  await evaluate(`document.getElementById('btn-cancel-generation').click()`);
  await waitFor(`!ui.busy && generationTask === null`, "cancelling ends the candidate worker");
  const afterCancel = await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,undo:ui.state.canUndo})`);
  check("cancelling preserves the current game and undo availability",
    beforeCancel.cells === afterCancel.cells && beforeCancel.seed === afterCancel.seed && beforeCancel.undo === afterCancel.undo,
    JSON.stringify(afterCancel));
  await evaluate(`send('set size ${beforeCancel.size}')`);
  check("ordinary settings do not expose resource caps", await evaluate(`!document.querySelector('input[data-key=node_budget],input[data-key=time_budget_ms],input[data-key=max_attempts]')`));
  check("generated puzzles carry a SAT uniqueness proof", await evaluate(`ui.state.genStats.uniqueVerified === true && ui.state.genStats.solverBackend === "batsat"`));

  const assessmentLabels = await evaluate(`(() => {
    const state = ui.state;
    const cases = [
      {logicalSolved:false, logicalRemaining:17, assessmentExhausted:false, assessmentConsistent:true, key:'quality.stalled'},
      {logicalSolved:false, logicalRemaining:17, assessmentExhausted:true, assessmentConsistent:true, key:'quality.partial'},
      {logicalSolved:false, logicalRemaining:17, assessmentExhausted:false, assessmentConsistent:false, key:'quality.inconsistent'},
    ];
    const labels = cases.map(c => {
      updateChrome({...state, genStats:{...state.genStats, ...c}});
      return document.getElementById('quality').textContent.endsWith(t(c.key));
    });
    updateChrome(state);
    return labels;
  })()`);
  check("assessment stalls, limits and conflicts have distinct labels", assessmentLabels.every(Boolean), JSON.stringify(assessmentLabels));

  const beforeWatchdog = await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,undo:ui.state.canUndo})`);
  await evaluate(`(() => {
    const original = createEngineWorker;
    const url = URL.createObjectURL(new Blob(['self.onmessage = () => {};'], {type:'text/javascript'}));
    createEngineWorker = () => new Worker(url);
    void generate(${beforeWatchdog.seed + 1});
    createEngineWorker = original;
    window.watchdogTestUrl = url;
  })()`);
  await waitFor(`!ui.busy && !generationTask && document.getElementById('status').textContent === t('status.generation_timed_out')`, "the worker watchdog ends an unresponsive search", 20000);
  const afterWatchdog = await evaluate(`({cells:ui.state.cells,seed:ui.state.seed,undo:ui.state.canUndo})`);
  check("the worker watchdog preserves the current game",
    beforeWatchdog.cells === afterWatchdog.cells && beforeWatchdog.seed === afterWatchdog.seed && beforeWatchdog.undo === afterWatchdog.undo,
    JSON.stringify(afterWatchdog));
  check("the worker watchdog releases pending requests", await evaluate(`waiting.size === 0 && requestOwners.size === 0`));
  await evaluate(`URL.revokeObjectURL(window.watchdogTestUrl)`);

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
