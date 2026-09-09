// Smoke test for the WebAssembly ABI, without a browser.
//
//   node tools/wasm-smoke.cjs [path/to/tapa.wasm]
//
// It loads the module exactly like web/worker.js does, runs every command the
// UI can send, and checks the replies. Exits non-zero on the first problem.

const fs = require("node:fs");
const path = require("node:path");

const wasmPath = process.argv[2] || path.join(__dirname, "..", "web", "tapa.wasm");
const module_ = new WebAssembly.Module(fs.readFileSync(wasmPath));
const instance = new WebAssembly.Instance(module_, {
  env: { tapa_now_ms: () => performance.now() },
});
const ex = instance.exports;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

let failures = 0;
function check(label, ok, detail) {
  if (ok) {
    console.log(`  ok   ${label}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${label}${detail ? `: ${detail}` : ""}`);
  }
}

function call(command) {
  const data = encoder.encode(command);
  const ptr = ex.scratch_ptr();
  new Uint8Array(ex.memory.buffer, ptr, data.length).set(data);
  const len = ex.tapa_command(data.length);
  const replyPtr = ex.reply_ptr();
  const json = decoder.decode(new Uint8Array(ex.memory.buffer, replyPtr, len));
  return JSON.parse(json);
}

console.log(`module: ${wasmPath} (${fs.statSync(wasmPath).size} bytes)`);

const init = call("init");
check("init replies ok", init.ok === true, JSON.stringify(init.error));
check("init has board size", init.w === 20 && init.h === 20, `${init.w}x${init.h}`);
check("init carries settings text", /size = 20/.test(init.settingsText), init.settingsText);
check("no wall count in the state", !("blackCount" in init) && !("walls" in init));

const small = call("set size 8");
check("set size", small.w === 8, String(small.w));
call("set node_budget 2000000");
call("set time_budget_ms 30000");

const puzzle = call("generate 12345");
check("generate ok", puzzle.ok === true && puzzle.w === 8, JSON.stringify(puzzle.error));
check("cells are the board size", puzzle.cells.length === 64, String(puzzle.cells.length));
check("clues are present", puzzle.clues.length > 0, puzzle.clues);
check("clues are sorted indices", /^\d+:\d+;/.test(puzzle.clues), puzzle.clues);
check("seed is echoed", puzzle.seed === 12345, String(puzzle.seed));
check("status reports cells left", /cell/.test(puzzle.status), puzzle.status);
check("generation stats shown", /clues/.test(puzzle.stats), puzzle.stats);

// clue cells start empty (2), everything else undecided (0)
const clueIndices = new Set(
  puzzle.clues
    .split(";")
    .filter(Boolean)
    .map((pair) => Number(pair.split(":")[0])),
);
let marksOk = true;
for (let i = 0; i < puzzle.cells.length; i += 1) {
  const want = clueIndices.has(i) ? "2" : "0";
  if (puzzle.cells[i] !== want) {
    marksOk = false;
    break;
  }
}
check("clue cells start empty, the rest blank", marksOk);

// marks that the player (or a deduction) actually decided, ignoring clue cells
function decided(state) {
  let n = 0;
  for (let i = 0; i < state.cells.length; i += 1) {
    if (clueIndices.has(i)) continue;
    if (state.cells[i] === "1" || state.cells[i] === "2") n += 1;
  }
  return n;
}

const step = call("onestep");
const filled = decided(step);
check("one step fills cells", filled > 0, `${filled} decided`);
check("one step status", /One step/.test(step.status), step.status);

const undo = call("undo");
check("undo takes one back", decided(undo) === filled - 1, `${decided(undo)} vs ${filled - 1}`);

const cleared = call("reset");
check(
  "reset clears every non clue cell",
  [...cleared.cells].every((c, i) => (clueIndices.has(i) ? c === "2" : c === "0")),
);

const firstClue = [...clueIndices][0];
const cx = firstClue % 8;
const cy = Math.floor(firstClue / 8);
const clueStep = call(`cluestep ${cx} ${cy}`);
check("cluestep answers", clueStep.ok === true, clueStep.error);
check(
  "cluestep either fills or explains",
  /filled|forces nothing/.test(clueStep.status),
  clueStep.status,
);
const freeCell = [...Array(64).keys()].find((i) => !clueIndices.has(i));
const notClue = call(`cluestep ${freeCell % 8} ${Math.floor(freeCell / 8)}`);
check("cluestep off a clue is rejected politely", /Middle-click/.test(notClue.status), notClue.status);

const wall = call("paint begin 0 0 1");
check("paint begin answers", wall.ok === true, wall.error);
const cell0IsClue = clueIndices.has(0);
check(
  "painting a wall sticks",
  cell0IsClue ? wall.cells[0] === "2" : wall.cells[0] === "1",
  wall.cells[0],
);
call("paint end");

const shown = call("solution");
check("solution is revealed", shown.showSolution === true && shown.solution.length === 64);
const hidden = call("solution");
check("solution hides again", hidden.showSolution === false && hidden.solution.length === 0);

const printed = call("print 1");
check("print produces a board", /solution:/.test(printed.output), printed.output.slice(0, 120));
check("print verifies uniqueness", /solutions=1/.test(printed.output), printed.output.slice(-200));

const bench = call("bench 2");
check("bench times two seeds", /seed \d+/.test(bench.output), bench.output.slice(0, 160));
check("bench summarises", /clues \d+\.\.\d+/.test(bench.output), bench.output.slice(-200));

const bad = call("nonsense");
check("unknown command reported", bad.ok === false && /unknown command/.test(bad.error), bad.error);

const badSetting = call("set size 999");
check("bad setting reported", /outside/.test(badSetting.settingsError), badSetting.settingsError);
check("bad setting did not stick", badSetting.w === 8, String(badSetting.w));

console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
