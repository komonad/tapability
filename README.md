# Tapa

A Tapa puzzle game in Rust, in two flavours that share one engine: a Win32
desktop window and a browser front end that runs the same code as WebAssembly.
The only external crate is `winapi`, and only the desktop window needs it. Both
contain a SAT solver, a human-oriented reasoner and a generator that only ever emits puzzles whose
solution is **provably unique**.

![a generated board](board.png)

The desktop window opens at a size taken from the desktop work area (taskbar
excluded) and centred, and it is resizable: the board takes whatever space is
left of the control column, so the cells grow and shrink with the window. Text
below and above the board is clipped to the board column, so it can never run
under the control column however small the window gets. The window cannot be
resized below the control column plus a 3x3 board at 11 pixels per cell.

```
cargo run --release                 # play in a Win32 window
cargo run --release -- --print 1    # print a puzzle + its solution
cargo run --release -- --bench 20   # time 20 generations
cargo test --workspace --release    # includes uniqueness, deduction soundness and minimality checks

pwsh -File build-web.ps1            # build web/tapa.wasm, then check the ABI
node tools/serve.cjs                # then open http://127.0.0.1:8080/
```

## Rules

Fill every cell black or white:

* a cell with a number is empty; the numbers are the lengths of the black runs
  around that cell (runs are separated by at least one white cell). The reading
  order is **not** fixed - a clue is a cycle, `1 2` and `2 1` are the same clue -
  so the digits are always shown **smallest first** and never hint at a starting
  point;
* all black cells form one orthogonally connected group;
* no 2x2 block is completely black.

## Controls

| input | effect |
|---|---|
| left click | mark wall (black) |
| right click | mark empty (white, shown as a small square dot) |
| left/right drag | paint a whole stroke; the first cell decides whether the stroke marks or clears |
| click the same mark again | clear the cell |
| left click on a white cell | switches it to wall, and vice versa |
| Z | undo (hold to repeat; one stroke = one entry per cell) |
| D | one snapshot round, or one local clue enumeration |
| H | highlight an area worth examining |
| middle click a clue | step just that one clue, without cascading |
| Alt + left click a clue | the same, for mice and trackpads without a middle button |
| drag the size slider | pick a board size; the puzzle is rebuilt when you release it |
| resize the window | the grid rescales to fill the space left of the control column |
| tap a cell (touch) | cycles the mark: wall -> empty -> unmarked; the outline stays on the last tapped cell, since a finger has no cursor |
| drag a finger (touch) | paints a stroke; the first cell decides whether it marks or clears |
| tap a clue (touch) | steps that one clue, like a middle click |
| Shift + hover | spotlight the wall group under the cursor |
| N | new puzzle |
| R | clear all marks (also undoable) |
| C | check: wrong cells get a red cross |
| S | show / hide the solution |
| Esc | quit |

Clue cells are given and cannot be edited; dragging across them simply skips
them. The board turns green when solved.

The wall count of the solution is never shown anywhere in the game - only the
seed and the number of clues are.

## Live feedback

The board tells you about broken rules while you play, without waiting for a
check:

* **spotlight** - adding a wall highlights the whole connected wall group it
  joined (blue, gold outline). Shift-hovering does the same for any group.
* **grey clue** - once a clue's numbers are all accounted for by the walls
  around it (so the remaining neighbours have to be white), its digits turn
  grey.
* **red clue** - if the walls and empty marks around a clue can no longer form
  its numbers, the clue turns red.
* **red walls** - a wall group that can never join the main wall group any more,
  because every cell between them is marked empty, is outlined in red. Groups
  that are only separate *for now* (undecided cells still connect them) are left
  alone, and a lone wall group is never flagged even when it is sealed in: the
  rule is about two groups that can never meet again, not about predicting where
  the rest of the solution's walls must be. Clue cells count as empty here,
  since they are given.
* **red 2x2** - any 2x2 block of walls is outlined in red.

Grid lines are never drawn between two cells that are already filled in, so a
wall group (and the spotlight) reads as one solid shape instead of a grid of
boxes. An undecided cell and a cell marked empty share the same white
background: the empty mark only adds a small square dot.

The status bar adds `red = rule broken` whenever something is flagged.

## One step

**Middle-clicking a clue cell** applies that clue once. Compatible neighbour
patterns must agree with every known black and white cell; cells with the same
value in every surviving pattern are confirmed.

The **One step (D)** button reads one immutable board snapshot. It checks
2x2 squares, mandatory connections between black components and the compatible
patterns of every clue against that same snapshot. A round may confirm several
cells, but new marks do not trigger more deductions until the next click. Each
round is one undoable action. Its changed cells remain selected, including both
black and white marks. Z restores the whole previous position and its selection;
manual edits clear the selection. Single-clue steps use the same grouped undo
behavior. Internal difficulty assessment uses a separate basic-rule closure that
repeats until stable.

When basic rules stall, D enumerates the feasible distributions around one clue.
Each distribution is placed on a copy of the current board and receives one
parallel round of basic rules. Immediate violations of any clue, the 2x2 rule
or potential black connectivity discard that distribution. Marks shared by every
surviving branch form one undoable round, including marks outside the clue's
neighborhood. Branches do not enumerate another clue, iterate deductions or call
SAT. A click stops at the first clue that yields a conclusion.

## Controls and settings in the UI

The window has two halves: the board on the left and an always-visible control
column on the right. Every action is a button there - New puzzle (N), Clear
(R), Check (C), Solution (S), Undo (Z) and One step (D) - and the generation
settings are editable in place. There is no menu to open: everything is on
screen, and hovering a setting shows what it means in the help line at the
bottom of the column.

The buttons and the keyboard shortcuts do the same thing. Changing a setting
and pressing **Apply & new puzzle** validates it with the same code the config
file uses, saves it, and starts a new puzzle; changing the board size re-fits
the grid in the current window (and grows the window if the new board would not
fit). **Defaults** refills the fields.

Sliders update their numeric fields while dragging. **Apply & new puzzle**
commits the generation settings. The browser includes sliders for size,
difficulty effort, black-cell coverage and clueless fraction; the desktop panel
shows the same generation settings. The browser black-cell target uses two handles on one track,
with a percentage readout. Other numeric fields remain editable. The separate
cell-size slider zooms the board immediately without generating a puzzle.

| setting | default | meaning |
|---|---|---|
| `size` | 20 | board is `size x size` (3..60) |
| `difficulty` | 3 | clue-removal proof effort; large boards return the first qualifying path |
| `seed` | clock | fixed seed; makes generation reproducible |
| `density` | `0.50-0.60` | approximate target share of black answer cells, chosen randomly within the range; independent of clue density |
| `max_clueless_fraction` | 0.15 | reject shapes whose biggest clue-free patch exceeds this fraction of the board |
| `limited_generation` | false | optional CLI/config compute limits; hidden from the ordinary control panel |
| `node_budget`, `time_budget_ms`, `max_attempts` | advanced | effective only when `limited_generation = true` |

The same settings can be set from a file or the command line:

```
tapa --config tapa.conf                  # play with those settings
tapa --config tapa.conf --set size=24    # override a single line
tapa --size 16 --density 0.35-0.45       # or set everything on the command line
tapa --print 3 --size 14 --seed 42       # generate without the GUI
```

`tapa.conf` in this directory is a documented example; `tapa --help` prints it.
Settings changed in the game are remembered in
`%APPDATA%\tapa\settings.conf` (falling back to `tapa-settings.conf` next to the
working directory when that is not writable).

### Optional compute limits

Interactive generation uses a 12-second response window and returns a
qualifying puzzle immediately on boards of at least 225 cells. The browser has
an independent 15-second worker watchdog, candidate progress and a Cancel
button. Timeout and cancellation preserve the current game and undo history.

For explicitly constrained CLI jobs, `--set limited_generation=true` enables
`node_budget`, `time_budget_ms` and `max_attempts`. With that opt-in, the budget
is a shared pool: each uniqueness proof
gets `remaining / proofs left * check_slack` SAT work units, so one that needs a little
more than its fair share can still finish. A proof that does run out keeps its
clue. Small boards retry unfinished proofs once at the end of the pass.
Running out of work never breaks uniqueness; it can leave a redundant clue.

Completion time varies with the requested shape and the number of candidates
needed to satisfy the workload targets. The browser remains responsive while
its separate generation worker searches.

## Web front end (WebAssembly)

The engine - generator, solver, rules, one-step deduction - compiles to
WebAssembly and runs inside a Web Worker, so the page never blocks, not even
while a 20x20 puzzle is being generated and its uniqueness proved. The
JavaScript is a pure view: it draws the board on a canvas, turns mouse and
keyboard into command strings, and renders whatever state the engine reports.

```
pwsh -File build-web.ps1     # cargo build --target wasm32-unknown-unknown + copy
node tools/serve.cjs         # static server on http://127.0.0.1:8080/
node tools/wasm-smoke.cjs    # drive the ABI from Node, no browser needed
node tools/browser-check.cjs # drive the real page in headless Chrome
```
`web/` is plain HTML/CSS/JS with no build step and no dependencies. The module
is loaded by hand rather than through `wasm-bindgen`, which is not available
offline: `tapa-wasm` exports a scratch buffer for the command text plus a JSON
reply buffer, and `web/worker.js` copies commands in and parses replies out. The
only requirement is the `wasm32-unknown-unknown` standard library, which
`build-web.ps1` adds if it is missing.

The browser layout places rules and contextual help on the left, the centered
board in the middle, and controls on the right. On narrow screens, the guide and
controls wrap below the board.

The page is translated: Simplified Chinese by default, English one click away in
the header, and the choice is remembered. The engine never sends prose for the
player to read - it reports a status *key* with its numbers (`statusKey`,
`statusArgs`) and structured generation statistics (`genStats`), so every line
in the UI, including the rules introduction under the title, comes from
`web/i18n.js`.

Two details keep a deploy from breaking a page that is already open. GitHub
Pages caches everything for ten minutes, so the workflow stamps the commit sha
into the asset URLs (`?v=...` on the stylesheet, both scripts and the wasm): a
freshly fetched page always asks for matching scripts instead of reusing ones
the browser cached from an earlier deploy. And every translatable element
carries its Chinese text inline in the HTML, so a page whose scripts are stale
or blocked still reads correctly instead of showing empty buttons. If a script
does end up older than the page, the two version stamps disagree and the page
reloads itself once with a cache-busting query; a repair pass then fills any
element that somehow ended up without text.

Everything the desktop game does is in the browser too, including a touch
build: a finger that taps a cell cycles it wall -> empty -> unmarked, a finger
that travels paints a stroke, and tapping a clue steps just that clue, so no
middle button is needed.

| feature | where |
|---|---|
| left click wall / right click empty / click again to clear | canvas pointer events |
| tap a cell to cycle wall -> empty -> clear, drag to paint | `tap` command, `TAP_MOVE_PX` in `web/app.js` |
| the last tapped cell keeps its outline (touch has no cursor) | `pointerleave` ignores touch pointers |
| tap a clue to step it | `tap` falls through to `cluestep` |
| drag a whole stroke, gaps filled in | `line_cells` in the engine |
| wall-group spotlight on paint, and on Shift + hover | `floodFill` in `web/app.js` |
| red clues, red sealed-off walls, red 2x2, red wrong cells | `live_errors` in the engine, drawn as outlines |
| a small square for an explicit empty mark | canvas |
| no grid line between two filled cells, so wall groups read as one shape | `solid()` in `web/app.js`, same rule in `src/render.rs` |
| N / R / C / S / D / H / Z keys, hold Z to repeat | window key handlers |
| middle click a clue to step only that clue | `cluestep` command |
| Alt + click, or the "Step one clue" button, for the same thing | `stepClue` in `web/app.js` |
| size slider, window resize rescales the canvas | `layout()` in `web/app.js` |
| settings with hover help, remembered between visits | `localStorage`, fields built from the engine's `FIELDS` |
| no wall count anywhere in the UI | the state the engine reports never contains one |
| `--print` / `--bench` | Print and Bench buttons, same text as the CLI |

`tools/browser-check.cjs` drives the real page in headless Chrome with real
input events: it paints, drags, undoes, steps a clue, checks every deduction
against the solution, spotlights a group, resizes the board, checks that
settings survive a reload, and confirms the page still answers in milliseconds
while the worker is benching. It writes `web-check.png` so the layout can be
inspected by eye.

The board is only covered while an operation is genuinely slow (generation,
bench, print): instant commands such as undo, check or a clue step repaint
immediately instead of flashing a "working" veil over the board.

## Deploying the web build

`web/` is a static site: `index.html`, `style.css`, `app.js`, `worker.js` and
the compiled `tapa.wasm`. Any static host will do. For GitHub Pages,
`.github/workflows/pages.yml` rebuilds the module from source, runs the tests
that do not need Windows, checks the ABI, and publishes `web/`:

1. push `main` (the workflow runs on every push and can also be started by hand),
2. **Settings -> Pages -> Build and deployment -> Source: GitHub Actions**,
3. open `https://<user>.github.io/<repo>/`.

Every path in the page is relative, so it works from a project subpath as well
as from a domain root. Pages serves `.wasm` as `application/wasm`, and the
loader falls back to `WebAssembly.instantiate` on a plain byte array if a host
gets the type wrong.

GitHub Pages on a **private** repository needs GitHub Pro; on a free account the
repository has to be public. Serving it locally needs neither:

```
node tools/serve.cjs                 # http://127.0.0.1:8080/
node tools/serve.cjs 8081 some/dir   # serve another directory on another port
```

## Code map

| file | contents |
|---|---|
| `core/src/model.rs` | grid geometry, cell states, clue encoding (`mask -> run lengths`), rule validation, live rule analysis, one-step deductions |
| `core/src/solver.rs` | incremental CDCL SAT, connectivity cuts, complete solution and uniqueness checks |
| `core/src/reasoning.rs` | deterministic clue, square and connectivity deductions |
| `core/src/difficulty.rs` | basic progress and bounded contradiction assessment |
| `core/src/generator.rs` | tree-shaped solution, maximal clue set, minimisation to a minimal set |
| `core/src/config.rs` | generation settings (file, command line, in-game controls) |
| `core/src/clock.rs` | monotonic milliseconds: `Instant` natively, a host import in wasm |
| `core/src/report.rs` | the text of `--print` / `--bench`, shared with the browser |
| `core/src/text.rs` | board rendering as text |
| `core/src/rng.rs` | splitmix64 PRNG (no dependency, reproducible seeds) |
| `src/render.rs` | GDI drawing, double buffered |
| `src/window.rs` | Win32 window, app state, undo stack, input handling |
| `src/settings_ui.rs` | the always-visible control column |
| `src/cli.rs` | `--print` / `--bench` |
| `wasm/src/lib.rs` | game state plus the command/JSON bridge, compiled to WebAssembly |
| `web/index.html`, `web/style.css`, `web/app.js`, `web/worker.js` | the browser front end |
| `web/i18n.js` | the Simplified Chinese and English strings |
| `tools/*.cjs` | static server, wasm ABI smoke test, headless-browser check |
| `tools/native-check.ps1` | desktop GUI check: posts real mouse messages and compares screenshots |
| `build-web.ps1` | builds and installs `web/tapa.wasm` |

The browser and native renderer bundle Inter at regular and semibold weights.
Clue digits are centred using their rendered ink bounds. Font subsets and the
SIL Open Font License are in `web/fonts/`; rendering requires no system font
installation or external font service.

## How the solver works

Complete solving uses [BatSat](https://github.com/c-cube/batsat), a pure Rust
CDCL SAT solver based on MiniSat. The same backend runs natively and in WebAssembly.
Each board cell has a Boolean black variable. Clue pattern selectors enforce the
cyclic neighbour runs and white clue cells; each 2x2 window contributes a clause
forbidding four black cells.

Connectivity uses lazy cuts: when a SAT model contains separate black components,
a surviving black cell inside a component and one outside require at least one
black cell on its boundary. These globally valid clauses exclude disconnected
models without excluding a connected solution. Model blocking counts distinct
boards, independently of auxiliary pattern variables.

During generation, all candidate clues have activation literals. The known valid
solution is blocked once, and each removal changes only the activation assumptions.
UNSAT proves no alternative solution exists. Learned clauses and connectivity cuts
remain valid between checks. An interrupted check never proves uniqueness.

The separate `reasoning` module performs deterministic clue-pattern intersection,
2x2 deductions and black-connectivity deductions. The `difficulty` module records
basic progress and bounded one-cell contradiction progress using those rules.
It does not call SAT. SAT search effort is diagnostic data, not a human difficulty
score; an unfinished reasoning assessment does not imply an invalid puzzle.

## Hints

Press **H** or **Hint** to highlight an area worth examining in orange. Hints
show positions only: no black or white conclusions, proof text or automatic
marks. The browser offers a dismiss button; native H toggles the highlight.
Connectivity hints outline the complete referenced black components, with a
separate outline around the relevant undecided area.
Editing, resetting, undoing, revealing the solution or replacing the board clears
the hint. Requesting or dismissing a hint leaves the undo history unchanged.

Hint search first uses direct rules and local clue enumeration, then one-cell
contradiction proofs, then incremental SAT verification. It checks that the current marks have a satisfying
completion and verifies a forced move before returning its focus positions.
Inconsistent, ambiguous and interrupted searches never turn an arbitrary model
into a hint. The search uses a 2.5-second response window and preserves the board
on interruption. Verification uses the player's marks and puzzle constraints,
without reading the stored answer. Proof details remain internal to the engine;
the browser response contains only focus indices. The D shortcut keeps its
single-snapshot round semantics.

## How the generator works

1. **grow a random solution whose black cells form a tree.** Each new black cell
   must touch the existing region on exactly one side, so the black cells are an
   acyclic connected set. That single rule gives every structural constraint
   Tapa needs for free: a 2x2 block, a black cell surrounded by black cells and
   a white cell ringed by black cells (a clue of 8) all contain a cycle.
   Growth is aimed at the white area currently furthest from the tree, and while
   any border is still untouched it is aimed at that border instead, so the
   black region always reaches all four edges rather than floating in a white
   margin. Equally suitable growth goals and extensions use reservoir sampling,
   giving every tied candidate the same probability regardless of iteration
   order. Black density is 50-60%, which keeps leftover white patches small.
2. **take the maximal clue set**: every white cell that touches black becomes a
   clue, except cells whose clue would be 8. If this set does not already have a
   unique solution, the shape is unusable (subsets are only weaker) and a new
   shape is grown.
3. **minimise each candidate's clues**: drop a clue only after a successful
   uniqueness proof. Large-board proofs use short work slices scaled by area
   and effort; a slower proof retains its clue. Aborted large-board proofs are
   not retried in the same removal pass.
4. **assess reasoning independently**: basic rules and single-cell contradiction
   propagation record opening deductions, remaining cells and successful steps.
   Assessment may complete, stall, or stop at its time allowance. Generation does
   not add givens to force completion by this rule set.
5. **screen workload and composition**: boards of at least 225 cells retain an
   opening-workload floor scaled to at most 625 reference cells, at most 80% of
   playable cells unresolved by basic rules, immediate clue deductions covering
   at least 3% of playable cells, and 12–23% clue density. These are heuristics,
   not calibrated human difficulty grades. Neither a complete single-layer path
   nor a minimum contradiction-step count is required.
6. **deliver a qualifying puzzle**: large boards return the first qualifying
   puzzle. Effort controls SAT work per removal. Small boards compare up to
   `difficulty` candidates using measured reasoning workload and clue count.

Every delivered puzzle has a validated stored solution and a completed SAT
uniqueness proof. Assessment completion is reported separately. The browser
retains the current puzzle and marks when generation fails or times out.
Fixed-seed results are reproducible when the response window allows the same
work to finish; a timeout can change which removals finish.

A native assessment example accepts tab-separated puzzle files with four fields:
width, height, label and `cell=digits;cell=digits` clues. A digit `0` denotes no
black neighbours. It reports basic progress, bounded logical progress, full
SAT work units, uniqueness and generation time using the same rules for each board:

```
cargo run --release -p tapa-core --example assess -- puzzles.tsv 12
```

The count chooses how many fixed seeds to generate at each comparison size
(20x20 and 25x25). An optional following comma-separated size list overrides
those sizes, e.g. `-- - 12 25`. Use `-` as the input path to assess generated
puzzles only. The output includes the acceptance result for every assessed board.

### Why not "start blank and add clues until the solution is unique"?

That construction is natural and it is what the generator effectively checks for
every clue it keeps, but the adding direction does not give a minimal set: it
only guarantees that each clue was needed *at the moment it was added*, and
later clues can make earlier ones redundant. Measured on the same shapes, adding
random clues until unique needs **117-164** clues, while removing from the
maximal set down to minimal needs **69-83**. The adding direction is also slower
here, because its intermediate puzzles are nearly unique - exactly where the
solver has to search hard - whereas the removing direction mostly works on dense
puzzles that are refuted quickly.

Every puzzle is *verified* unique, the stored solution is the one the shape
started from, and clues of 8 never appear (a 0 cannot occur either: a clue is
only placed where black already touches).

## Measuring generation

Generation time includes shape sampling, incremental SAT uniqueness proofs,
bounded reasoning assessment and comparison of qualifying candidates. Measure a fixed seed set with the
assessment example above to report completion times alongside logical-path
statistics. The WebAssembly engine runs in a worker so the page remains
interactive during generation.

## Limitations

* The generator is tuned for 20x20; other sizes work but were only spot-checked.
* Initial removal seeks an inclusion-minimal unique clue set. Bounded proofs
  can retain a redundant clue. Uniqueness is guaranteed even when minimality
  has not been proved.
* Reasoning assessment uses a limited rule set. Unresolved cells indicate an
  assessment limit, not a calibrated human difficulty level. Human difficulty
  and visual composition still need playtesting.
* Individual SAT removal proofs use work slices so one proof cannot monopolize
  generation. Some settings can still reach the response deadline.
* The desktop build is Windows only (Win32 + GDI). The engine and the browser
  front end are not: they only need `wasm32-unknown-unknown`.
* The browser build needs a static server; `file://` cannot fetch WebAssembly.
  `tools/serve.cjs` is that server, with no dependencies.
* The browser bridge uses a small hand-written WebAssembly ABI. BatSat is a
  shared engine dependency; Win32 APIs are confined to the desktop crate.

