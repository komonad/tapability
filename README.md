# Tapa

A Tapa puzzle game for Windows, written in Rust with no external crates except
`winapi`. It contains a rule-based solver and a generator that only ever emits
puzzles whose solution is **provably unique**.

![a generated board](board.png)

The window is sized from the desktop work area (taskbar excluded) and centred,
so the whole board and the control column are always visible.

```
cargo run --release                 # play
cargo run --release -- --print 1    # print a puzzle + its solution
cargo run --release -- --bench 20   # time 20 generations
cargo test --release                # 31 tests, incl. uniqueness + minimality
```

## Rules

Fill every cell black or white:

* a cell with a number is empty; the numbers are the lengths of the black runs
  around that cell, read **clockwise** starting anywhere (runs are separated by
  at least one white cell);
* all black cells form one orthogonally connected group;
* no 2x2 block is completely black.

## Controls

| input | effect |
|---|---|
| left click | mark wall (black) |
| right click | mark empty (white, shown as a small square) |
| left/right drag | paint a whole stroke; the first cell decides whether the stroke marks or clears |
| click the same mark again | clear the cell |
| left click on a white cell | switches it to wall, and vice versa |
| Z | undo (hold to repeat; one stroke = one entry per cell) |
| D | one step: fill in everything the clues alone force |
| middle click a clue | step just that one clue, without cascading |
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
* **red clue** - if the walls and empty marks around a clue can no longer form
  its numbers, the clue turns red.
* **red walls** - a wall group that can never join the main wall group any more,
  because every cell between them is marked empty, is outlined in red. Groups
  that are only separate *for now* (undecided cells still connect them) are left
  alone.
* **red 2x2** - any 2x2 block of walls is outlined in red.

The status bar adds `red = rule broken` whenever something is flagged.

## One step

There are two granularities. **Middle-clicking a clue cell** applies the rule to
that single clue once - no cascading - which is handy for poking at one number.
The **One step (D)** button applies only the rule *"the black runs around a clue
must match its numbers"* to the cells around each clue, to a fixpoint, and marks
everything that follows from it. It never uses search or the connectivity rule,
so it is always sound: every cell it fills is forced. On a dense 20x20 board
this typically fills 100-250 cells; press it again and it usually finds nothing
new, because the first press already reached the fixpoint.

## Controls and settings in the UI

The window has two halves: the board on the left and an always-visible control
column on the right. Every action is a button there - New puzzle (N), Clear
(R), Check (C), Solution (S), Undo (Z) and One step (D) - and the generation
settings are editable in place. There is no menu to open: everything is on
screen, and hovering a setting shows what it means in the help line at the
bottom of the column.

The buttons and the keyboard shortcuts do the same thing. Changing a setting
and pressing **Apply & new puzzle** validates it with the same code the config
file uses, saves it, and starts a new puzzle; changing the board size rebuilds
the layout and resizes the window. **Defaults** refills the fields.

| setting | default | meaning |
|---|---|---|
| `size` | 20 | board is `size x size` (3..60) |
| `seed` | clock | fixed seed; makes generation reproducible |
| `density` | `0.40-0.50` | fraction of black cells, picked randomly in this range |
| `max_clueless_fraction` | 0.15 | reject shapes whose biggest clue-free patch exceeds this fraction of the board |
| `node_budget` | 150000 | nodes for one whole generation, shared adaptively |
| `time_budget_ms` | 2000 | milliseconds for one whole generation |
| `check_min_nodes` | 3000 | floor for a single uniqueness proof |
| `check_slack` | 3 | how much one proof may borrow over its fair share |
| `max_attempts` | 200 | how many shapes to try before giving up |

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

### How the budget behaves

The budget is a **shared pool, not a hard per-proof cap**: each uniqueness proof
gets `remaining / proofs left * check_slack` nodes, so one that needs a little
more than its fair share can still finish. A proof that does run out keeps its
clue and is retried once at the end with whatever budget is left. Running out of
budget never breaks uniqueness - it only leaves a redundant clue in place.

Measured on this machine: 12x12 generates in ~15 ms, 20x20 in ~1.0 s average,
30x30 in 5-10 s (it needs a larger `node_budget` to stay minimal).

## Code map

| file | contents |
|---|---|
| `src/model.rs` | grid geometry, cell states, clue encoding (`mask -> run lengths`), rule validation, live rule analysis, one-step deductions |
| `src/solver.rs` | propagation + backtracking search, capped solution counting |
| `src/generator.rs` | tree-shaped solution, maximal clue set, minimisation to a minimal set |
| `src/config.rs` | generation settings (file, command line, in-game controls) |
| `src/settings_ui.rs` | the always-visible control column |
| `src/rng.rs` | splitmix64 PRNG (no dependency, reproducible seeds) |
| `src/render.rs` | GDI drawing, double buffered |
| `src/window.rs` | Win32 window, app state, undo stack, input handling |
| `src/cli.rs` | `--print` / `--bench` |

## How the solver works

Search is depth-first backtracking, but almost all the work is done by
propagation, which runs to a fixpoint before every branch:

1. **clue cells are empty** - forced;
2. **arc consistency per clue** - every clue has a precomputed set of legal
   8-neighbour bitmasks (filtered for cells outside the grid, which can never
   be black). The surviving set is kept as a 256-bit bitset, so "which patterns
   still fit" is a handful of AND instructions; a neighbour that is black
   (white) in *all* surviving patterns is assigned. If nothing survives, the
   branch is dead;
3. **no 2x2 black** - three black cells force the fourth white. Only the four
   windows around a cell that just turned black are examined;
4. **connectivity** - the cells that could still become black are split into
   orthogonal components. All black cells must end up in one component, so once
   one component contains a black cell, every other component is forced white.
   Two components containing black cells is a contradiction.

Branching is **not** on single cells: the solver picks the clue with the fewest
surviving patterns and branches on those patterns, which assigns up to eight
cells at once. Only when every clue is fully determined does it fall back to
picking the most constrained single cell.

Uniqueness is decided by counting solutions with the cap set to 2: exactly one
solution *found and proven* (not "aborted by budget") means unique.

## How the generator works

1. **grow a random solution whose black cells form a tree.** Each new black cell
   must touch the existing region on exactly one side, so the black cells are an
   acyclic connected set. That single rule gives every structural constraint
   Tapa needs for free: a 2x2 block, a black cell surrounded by black cells and
   a white cell ringed by black cells (a clue of 8) all contain a cycle.
   Growth is aimed at the white area currently furthest from the tree, so the
   tree reaches into every corner instead of leaving one big empty patch. Black
   density is 40-50%, which keeps the leftover white patches small.
2. **take the maximal clue set**: every white cell that touches black becomes a
   clue, except cells whose clue would be 8. If this set does not already have a
   unique solution, the shape is unusable (subsets are only weaker) and a new
   shape is grown.
3. **minimise to a minimal clue set**: walk the candidates in random order and
   drop a clue whenever the puzzle still has exactly one solution. Every kept
   clue was tested against a superset of the final set, so the result is
   *inclusion-minimal*: no single clue of it can be removed any more.

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

## Measured performance

Windows, `--release`, 20x20, 20 seeds:

| metric | result |
|---|---|
| generation time | 120-3500 ms, ~1000 ms average, 0 failures |
| search nodes per uniqueness proof | 35-22000, adaptive pool |
| clues per puzzle | 69-83 (inclusion-minimal) |
| black cells | 165-199 of 400 (41-50%) |
| biggest clue-free patch | 8-39 cells (2-10% of the board) |
| clue-free rows / columns | 0-1 of 20 |
| test suite | 31 tests, ~0.6 s release |

The clue count is high because the numbers have to cover the whole board *and*
the set has to stay minimal. Dense boards are also what make the solver fast:
the harder a puzzle is constrained, the quicker a uniqueness proof finishes, so
the tail latency went down as the density went up.

## Limitations

* The generator is tuned for 20x20; other sizes work but were only spot-checked.
* "Minimal" here means *inclusion*-minimal (no single clue is removable), which
  greedy removal guarantees; it is not proven to be the smallest possible clue
  set for that solution. When the budget runs out, a removable clue can also
  survive. The unit tests check strict minimality with the budgets disabled.
* One step only uses the clue rule, so it can leave cells that the connectivity
  or 2x2 rules would force.
* The solver has no clause learning or restarts; hard instances are bounded by
  the shared budget rather than solved quickly.
* Windows only (Win32 + GDI). No network access was available, so the only
  dependency is `winapi`, which was already in the local cargo cache.

