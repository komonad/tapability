//! WebAssembly bridge for the Tapa engine.
//!
//! The browser side (see `web/worker.js`) owns no game logic: it copies a
//! command string into [`scratch_ptr`], calls [`tapa_command`], and reads the
//! JSON reply from [`reply_ptr`]. Everything - generating a unique puzzle,
//! proving uniqueness, checking the board, one-step deduction, undo - happens
//! here, on a worker thread, so the page never blocks.
//!
//! The command language is deliberately tiny:
//!
//! ```text
//! init                                  reset the engine, keep no puzzle
//! set <key> <value>                     change one setting
//! defaults                              restore the built-in settings
//! generate [seed]                       build a new puzzle (seed optional)
//! paint begin <x> <y> <0|1>             start a stroke (1 = wall, 0 = empty)
//! paint move <x> <y>                    continue the stroke
//! paint end                             finish the stroke
//! undo | reset | check | solution | onestep
//! cluestep <x> <y>                      single-clue step (middle click)
//! tap <x> <y>                           one touch tap: cycle the cell's mark
//! print [count]                         generate and render puzzles as text
//! bench [count]                         time N generations
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;

use tapa_core::clock::Timer;
use tapa_core::config::Settings;
use tapa_core::generator::{generate_with, GenConfig};
use tapa_core::model::{
    clue_deductions, clue_satisfiable, clue_step, clue_text, line_cells, live_errors, validate,
    LiveErrors, Puzzle, BLACK, UNKNOWN, WHITE,
};
use tapa_core::rng::Rng;
use tapa_core::report;

/// How many marks can be undone; the same limit as the native game.
const MAX_HISTORY: usize = 4096;
/// Size of the buffer JavaScript writes commands into.
const SCRATCH_CAP: usize = 1 << 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Info = 0,
    Good = 1,
    Bad = 2,
}

/// One active paint stroke: the value it applies and the last cell it touched.
struct Drag {
    value: u8,
    last: usize,
}

struct Engine {
    settings: Settings,
    puzzle: Option<Puzzle>,
    cells: Vec<u8>,
    wrong: Vec<bool>,
    errors: LiveErrors,
    history: VecDeque<(usize, u8)>,
    /// Cell that was changed last, used to break ties in the error analysis.
    last_changed: Option<usize>,
    /// Wall whose group is spotlighted (the last wall that was painted).
    anchor: Option<usize>,
    drag: Option<Drag>,
    status: String,
    kind: Kind,
    solved: bool,
    show_solution: bool,
    seed: u64,
    /// One-line statistics of the last generation.
    stats: String,
    /// Output of `print` / `bench`.
    output: String,
    /// Settings error from the last `set`.
    settings_error: String,
}

impl Engine {
    fn new() -> Engine {
        Engine {
            settings: Settings::default(),
            puzzle: None,
            cells: Vec::new(),
            wrong: Vec::new(),
            errors: LiveErrors {
                cells: Vec::new(),
                clues: Vec::new(),
            },
            history: VecDeque::new(),
            last_changed: None,
            anchor: None,
            drag: None,
            status: "Press New puzzle to generate one.".to_string(),
            kind: Kind::Info,
            solved: false,
            show_solution: false,
            seed: 0,
            stats: String::new(),
            output: String::new(),
            settings_error: String::new(),
        }
    }

    /// Fresh player marks: clue cells start empty, everything else blank.
    fn fresh_marks(puzzle: &Puzzle) -> Vec<u8> {
        let mut cells = vec![UNKNOWN; puzzle.grid.len()];
        for (idx, _) in puzzle.clues.iter() {
            cells[*idx] = WHITE;
        }
        cells
    }

    fn size(&self) -> (usize, usize) {
        match &self.puzzle {
            Some(puzzle) => (puzzle.grid.w, puzzle.grid.h),
            None => (self.settings.size, self.settings.size),
        }
    }

    fn cell_index(&self, x: i64, y: i64) -> Option<usize> {
        let puzzle = self.puzzle.as_ref()?;
        if x < 0 || y < 0 || x as usize >= puzzle.grid.w || y as usize >= puzzle.grid.h {
            return None;
        }
        Some(puzzle.grid.idx(x as usize, y as usize))
    }

    fn new_puzzle(&mut self, seed: u64) {
        self.seed = seed;
        self.solved = false;
        self.show_solution = false;
        self.history.clear();
        self.last_changed = None;
        self.anchor = None;
        self.drag = None;
        self.wrong.clear();
        self.output.clear();
        self.status = "Generating a puzzle with a unique solution...".to_string();
        self.kind = Kind::Info;

        let cfg = GenConfig::from_settings(&self.settings);
        let mut rng = Rng::new(seed);
        match generate_with(&mut rng, &cfg) {
            Some(result) => {
                let mut puzzle = result.puzzle;
                puzzle.seed = seed;
                let n = puzzle.grid.len();
                self.cells = Engine::fresh_marks(&puzzle);
                self.wrong = vec![false; n];
                self.puzzle = Some(puzzle);
                self.stats = format!(
                    "{} clues, {}x{}, {:.0} ms, {} attempt(s), {} node(s)",
                    result.stats.clue_count,
                    self.settings.size,
                    self.settings.size,
                    result.stats.elapsed.as_secs_f64() * 1000.0,
                    result.stats.attempts,
                    result.stats.nodes
                );
                self.refresh_derived();
                self.refresh_status();
            }
            None => {
                self.puzzle = None;
                self.cells.clear();
                self.wrong.clear();
                self.stats.clear();
                self.status = format!(
                    "generation failed (seed {seed}); try a smaller board or a bigger budget"
                );
                self.kind = Kind::Bad;
            }
        }
    }

    fn set_setting(&mut self, key: &str, value: &str) {
        self.settings_error.clear();
        if let Err(err) = self.settings.set(key, value) {
            self.settings_error = err.to_string();
        }
    }

    fn begin_drag(&mut self, x: i64, y: i64, left: bool) {
        if self.show_solution {
            return;
        }
        let Some(idx) = self.cell_index(x, y) else {
            return;
        };
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let want = if left { BLACK } else { WHITE };
        // A clue cell cannot be toggled, so the button decides its value.
        let value = if puzzle.is_clue(idx) {
            want
        } else if self.cells[idx] == want {
            UNKNOWN
        } else {
            want
        };
        self.drag = Some(Drag { value, last: idx });
        self.paint_cell(idx, value);
        self.anchor = if value == BLACK { Some(idx) } else { None };
    }

    fn drag_to(&mut self, x: i64, y: i64) {
        let Some(drag) = self.drag.as_ref() else {
            return;
        };
        let Some(idx) = self.cell_index(x, y) else {
            return;
        };
        if idx == drag.last {
            return;
        }
        let (value, last) = (drag.value, drag.last);
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let path = line_cells(&puzzle.grid, last, idx);
        for cell in path {
            self.paint_cell(cell, value);
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.last = idx;
        }
        self.anchor = if value == BLACK { Some(idx) } else { None };
    }

    /// Apply one stroke cell (clue cells are never touched).
    fn paint_cell(&mut self, cell: usize, value: u8) {
        if let Some(puzzle) = self.puzzle.as_ref() {
            if puzzle.is_clue(cell) {
                return;
            }
        }
        self.set_cell(cell, value);
    }

    /// Change one cell, remembering the previous value for undo.
    fn set_cell(&mut self, cell: usize, value: u8) {
        if self.cells[cell] == value {
            return;
        }
        self.history.push_back((cell, self.cells[cell]));
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
        self.cells[cell] = value;
        self.after_change(Some(cell));
    }

    /// Apply a list of deduced marks as ordinary, undoable moves.
    fn apply_marks(&mut self, marks: Vec<(usize, u8)>) -> usize {
        let mut applied = 0usize;
        for (cell, value) in marks {
            if self.cells[cell] != UNKNOWN {
                continue;
            }
            self.history.push_back((cell, self.cells[cell]));
            while self.history.len() > MAX_HISTORY {
                self.history.pop_front();
            }
            self.cells[cell] = value;
            applied += 1;
        }
        if applied > 0 {
            self.anchor = None;
            self.after_change(None);
        }
        applied
    }

    /// One tap on a touch screen: unmarked -> wall -> empty -> unmarked.
    /// Clue cells are never marked, so a tap on one steps that clue instead.
    fn tap_cell(&mut self, x: i64, y: i64) {
        if self.show_solution {
            return;
        }
        let Some(idx) = self.cell_index(x, y) else {
            return;
        };
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        if puzzle.is_clue(idx) {
            self.clue_step_at(x, y);
            return;
        }
        let next = match self.cells[idx] {
            BLACK => WHITE,
            WHITE => UNKNOWN,
            _ => BLACK,
        };
        self.set_cell(idx, next);
        self.anchor = if next == BLACK { Some(idx) } else { None };
    }

    /// Middle click on a clue: deduce only what that single clue forces now.
    fn clue_step_at(&mut self, x: i64, y: i64) {        if self.show_solution {
            return;
        }
        let Some(cell) = self.cell_index(x, y) else {
            return;
        };
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let Some(clue) = puzzle.clue_at(cell) else {
            self.status = "Middle-click a clue cell to step just that clue.".to_string();
            self.kind = Kind::Info;
            return;
        };
        let step = clue_step(&puzzle.grid, &self.cells, cell, clue);
        let satisfiable = clue_satisfiable(&puzzle.grid, &self.cells, cell, clue);
        let (x0, y0) = puzzle.grid.xy(cell);

        if !satisfiable {
            self.status = format!("Clue ({x0},{y0}) can no longer be satisfied.");
            self.kind = Kind::Bad;
            return;
        }
        if step.is_empty() {
            self.status = format!("Clue ({x0},{y0}) forces nothing new.");
            self.kind = Kind::Info;
            return;
        }
        let applied = self.apply_marks(step);
        self.status = format!("Clue ({x0},{y0}): filled {applied} cell(s).");
        self.kind = Kind::Good;
    }

    /// Fill in everything the clues alone force, given the current marks.
    fn one_step(&mut self) {
        if self.show_solution {
            return;
        }
        let deduced = {
            let Some(puzzle) = self.puzzle.as_ref() else {
                return;
            };
            clue_deductions(&puzzle.grid, &self.cells, &puzzle.clues)
        };
        if deduced.is_empty() {
            self.status = "One step: the clues force nothing new right now.".to_string();
            self.kind = Kind::Info;
            return;
        }
        let applied = self.apply_marks(deduced);
        self.status = format!(
            "One step: filled {applied} cell{} the clues force.",
            if applied == 1 { "" } else { "s" }
        );
        self.kind = Kind::Good;
    }

    fn toggle_solution(&mut self) {
        if self.puzzle.is_some() {
            self.show_solution = !self.show_solution;
            self.status = if self.show_solution {
                "Showing the solution (S to hide)".to_string()
            } else {
                "Solution hidden".to_string()
            };
            self.kind = Kind::Info;
        }
    }

    fn undo(&mut self) {
        if self.show_solution {
            return;
        }
        let Some((cell, previous)) = self.history.pop_back() else {
            self.status = "Nothing to undo.".to_string();
            self.kind = Kind::Info;
            return;
        };
        self.cells[cell] = previous;
        self.anchor = None;
        self.after_change(Some(cell));
    }

    fn reset(&mut self) {
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let fresh = Engine::fresh_marks(puzzle);
        for i in 0..self.cells.len() {
            if self.cells[i] != fresh[i] {
                self.history.push_back((i, self.cells[i]));
                while self.history.len() > MAX_HISTORY {
                    self.history.pop_front();
                }
            }
        }
        self.cells = fresh;
        self.anchor = None;
        self.after_change(None);
    }

    /// Compare the player's marks against the (unique) solution.
    fn check(&mut self) {
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let mut wrong = vec![false; self.cells.len()];
        let mut count = 0usize;
        for i in 0..self.cells.len() {
            if puzzle.is_clue(i) {
                continue;
            }
            let c = self.cells[i];
            if c != UNKNOWN && c != puzzle.solution[i] {
                wrong[i] = true;
                count += 1;
            }
        }
        self.wrong = wrong;
        self.status = if count == 0 {
            "No mistakes so far.".to_string()
        } else {
            format!("{count} wrong cell(s) marked in red")
        };
        self.kind = if count == 0 { Kind::Good } else { Kind::Bad };
    }

    fn after_change(&mut self, focus: Option<usize>) {
        if focus.is_some() {
            self.last_changed = focus;
        }
        self.refresh_derived();
        self.refresh_status();
    }

    fn refresh_derived(&mut self) {
        let Some(puzzle) = self.puzzle.as_ref() else {
            self.errors = LiveErrors {
                cells: Vec::new(),
                clues: Vec::new(),
            };
            return;
        };
        self.errors = live_errors(&puzzle.grid, &self.cells, &puzzle.clues, self.last_changed);
    }

    fn refresh_status(&mut self) {
        let (empty, errors) = match self.puzzle.as_ref() {
            None => return,
            Some(puzzle) => {
                let empty = self.cells.iter().filter(|&&c| c == UNKNOWN).count();
                let errors = if empty == 0 {
                    validate(&puzzle.grid, &self.cells, &puzzle.clues)
                } else {
                    Vec::new()
                };
                (empty, errors)
            }
        };
        if empty > 0 {
            self.solved = false;
            self.status = format!("{empty} cell{} left", if empty == 1 { "" } else { "s" });
            self.kind = Kind::Info;
        } else if errors.is_empty() {
            self.solved = true;
            self.status = "Solved! Press N for a new puzzle.".to_string();
            self.kind = Kind::Good;
        } else {
            self.solved = false;
            self.status = format!(
                "Filled, but {} rule violation(s) - press C to see them",
                errors.len()
            );
            self.kind = Kind::Bad;
        }
    }

    /// Run `count` generations for the Bench button (heavy, but off the page).
    fn bench(&mut self, count: usize) {
        let seed = if self.seed == 0 {
            Timer::start().elapsed_ms() as u64
        } else {
            self.seed
        };
        let started = Timer::start();
        self.output = report::bench(count, &self.settings, seed);
        self.status = format!(
            "Bench: {count} puzzle(s) in {:.2} s",
            started.elapsed_ms() / 1000.0
        );
        self.kind = Kind::Good;
    }

    fn print_puzzles(&mut self, count: usize) {
        let seed = if self.seed == 0 { 1 } else { self.seed };
        self.output = report::print_puzzles(count, &self.settings, seed);
        self.status = format!("Printed {count} puzzle(s) below");
        self.kind = Kind::Good;
    }

    fn command(&mut self, cmd: &str) -> Result<(), String> {
        let mut parts = cmd.split_whitespace();
        let name = parts.next().unwrap_or("");
        match name {
            "" => {}
            "init" => {
                self.settings_error.clear();
                self.output.clear();
            }
            "set" => {
                let key = parts.next().ok_or("set needs a key")?;
                let value = parts.collect::<Vec<_>>().join(" ");
                self.set_setting(key, &value);
            }
            "defaults" => {
                self.settings = Settings::default();
                self.settings_error.clear();
            }
            "generate" => {
                let seed = match parts.next() {
                    Some(text) => text.parse::<u64>().map_err(|_| "bad seed")?,
                    None => self.seed,
                };
                self.new_puzzle(seed);
            }
            "paint" => {
                let phase = parts.next().ok_or("paint needs a phase")?;
                match phase {
                    "begin" => {
                        let x = parse_i64(parts.next())?;
                        let y = parse_i64(parts.next())?;
                        let left = match parts.next() {
                            Some("0") => false,
                            Some("1") | None => true,
                            _ => return Err("paint begin wants 0 or 1".into()),
                        };
                        self.begin_drag(x, y, left);
                    }
                    "move" => {
                        let x = parse_i64(parts.next())?;
                        let y = parse_i64(parts.next())?;
                        self.drag_to(x, y);
                    }
                    "end" => self.drag = None,
                    other => return Err(format!("unknown paint phase '{other}'")),
                }
            }
            "undo" => self.undo(),
            "reset" => self.reset(),
            "check" => self.check(),
            "solution" => self.toggle_solution(),
            "onestep" => self.one_step(),
            "tap" => {
                let x = parse_i64(parts.next())?;
                let y = parse_i64(parts.next())?;
                self.tap_cell(x, y);
            }
            "cluestep" => {
                let x = parse_i64(parts.next())?;
                let y = parse_i64(parts.next())?;
                self.clue_step_at(x, y);
            }
            "print" => {
                let count = parse_count(parts.next(), 1)?;
                self.print_puzzles(count);
            }
            "bench" => {
                let count = parse_count(parts.next(), 1)?;
                self.bench(count);
            }
            other => return Err(format!("unknown command '{other}'")),
        }
        Ok(())
    }

    /// The whole UI state, as JSON. `with_fields` adds the settings metadata
    /// (key, label, hover help) so the page can build its form from the engine
    /// instead of hard-coding it.
    fn state_json(&self, error: &str, with_fields: bool) -> String {
        let mut out = String::with_capacity(4096);
        let (w, h) = self.size();
        out.push_str("{\"ok\":");
        out.push_str(if error.is_empty() { "true" } else { "false" });
        out.push_str(",\"error\":\"");
        escape_into(&mut out, error);
        out.push_str("\",\"status\":\"");
        escape_into(&mut out, &self.status);
        out.push_str("\",\"settingsError\":\"");
        escape_into(&mut out, &self.settings_error);
        out.push_str("\",\"stats\":\"");
        escape_into(&mut out, &self.stats);
        out.push_str(&format!(
            "\",\"w\":{w},\"h\":{h},\"kind\":{},\"seed\":{},\"solved\":{},\"showSolution\":{},\"generating\":{},\"canUndo\":{},\"anchor\":{},\"clueCount\":{}",
            self.kind as u8,
            self.seed,
            self.solved,
            self.show_solution,
            self.puzzle.is_none(),
            !self.history.is_empty(),
            self.anchor.map_or(-1i64, |a| a as i64),
            self.puzzle.as_ref().map_or(0, |p| p.clues.len()),
        ));
        out.push_str(",\"settingsText\":\"");
        escape_into(&mut out, &self.settings.to_text());
        out.push('"');

        match &self.puzzle {
            None => {
                out.push_str(",\"cells\":\"\",\"clues\":\"\",\"errorsCells\":\"\",\"errorsClues\":\"\",\"wrong\":\"\",\"solution\":\"\"");
            }
            Some(puzzle) => {
                out.push_str(",\"cells\":\"");
                for &c in &self.cells {
                    out.push((b'0' + c) as char);
                }
                out.push_str("\",\"clues\":\"");
                for (idx, clue) in &puzzle.clues {
                    out.push_str(&format!("{idx}:{};", clue_text(clue)));
                }
                out.push_str("\",\"errorsCells\":\"");
                push_bools(&mut out, &self.errors.cells);
                out.push_str("\",\"errorsClues\":\"");
                push_bools(&mut out, &self.errors.clues);
                out.push_str("\",\"wrong\":\"");
                push_bools(&mut out, &self.wrong);
                out.push_str("\",\"solution\":\"");
                if self.show_solution {
                    for &c in &puzzle.solution {
                        out.push((b'0' + c) as char);
                    }
                }
                out.push('"');
            }
        }

        if !self.output.is_empty() {
            out.push_str(",\"output\":\"");
            escape_into(&mut out, &self.output);
            out.push('"');
        }
        if with_fields {
            out.push_str(",\"fields\":[");
            for (k, (key, label, help)) in tapa_core::config::FIELDS.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                out.push_str("{\"key\":\"");
                escape_into(&mut out, key);
                out.push_str("\",\"label\":\"");
                escape_into(&mut out, label);
                out.push_str("\",\"help\":\"");
                escape_into(&mut out, help);
                out.push_str("\",\"value\":\"");
                escape_into(&mut out, &self.settings.value_of(key));
                out.push_str("\"}");
            }
            out.push(']');
        }
        out.push('}');
        out
    }
}

fn parse_i64(text: Option<&str>) -> Result<i64, String> {
    text.ok_or_else(|| "missing coordinate".to_string())?
        .parse::<i64>()
        .map_err(|_| "bad coordinate".to_string())
}

fn parse_count(text: Option<&str>, default: usize) -> Result<usize, String> {
    match text {
        None => Ok(default),
        Some(t) => t
            .parse::<usize>()
            .map(|n| n.clamp(1, 200))
            .map_err(|_| "bad count".to_string()),
    }
}

fn push_bools(out: &mut String, flags: &[bool]) {
    for &f in flags {
        out.push(if f { '1' } else { '0' });
    }
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Buffer JavaScript writes the command text into. WebAssembly is single
/// threaded, so a plain static is enough.
static mut SCRATCH: [u8; SCRATCH_CAP] = [0u8; SCRATCH_CAP];

fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        f(slot.get_or_insert_with(Engine::new))
    })
}

/// Where JavaScript writes the command text.
#[no_mangle]
pub extern "C" fn scratch_ptr() -> *mut u8 {
    std::ptr::addr_of_mut!(SCRATCH) as *mut u8
}

/// How many bytes the command buffer can hold.
#[no_mangle]
pub extern "C" fn scratch_cap() -> usize {
    SCRATCH_CAP
}

/// Run one command. Returns the length of the JSON reply, which starts at
/// [`reply_ptr`]. Invalid UTF-8 in the command is replaced, never fatal.
#[no_mangle]
pub extern "C" fn tapa_command(len: usize) -> usize {
    let text = unsafe {
        let buf = &*std::ptr::addr_of!(SCRATCH);
        let end = len.min(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    };
    let with_fields = text.split_whitespace().next() == Some("init");
    let reply = with_engine(|engine| match engine.command(&text) {
        Ok(()) => engine.state_json("", with_fields),
        Err(err) => engine.state_json(&err, with_fields),
    });
    REPLY.with(|r| {
        let mut buf = r.borrow_mut();
        buf.clear();
        buf.extend_from_slice(reply.as_bytes());
        buf.len()
    })
}

/// Start of the JSON reply produced by the last [`tapa_command`].
#[no_mangle]
pub extern "C" fn reply_ptr() -> *const u8 {
    REPLY.with(|r| r.borrow().as_ptr())
}

/// Length of the JSON reply produced by the last [`tapa_command`].
#[no_mangle]
pub extern "C" fn reply_len() -> usize {
    REPLY.with(|r| r.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with_puzzle(size: usize, seed: u64) -> Engine {
        let mut engine = Engine::new();
        engine.set_setting("size", &size.to_string());
        engine.set_setting("node_budget", "1000000");
        engine.set_setting("time_budget_ms", "20000");
        engine.new_puzzle(seed);
        engine
    }

    #[test]
    fn generates_and_starts_blank_apart_from_clues() {
        let engine = engine_with_puzzle(8, 7);
        let puzzle = engine.puzzle.as_ref().expect("puzzle");
        assert_eq!(engine.cells.len(), 64);
        for i in 0..64 {
            if puzzle.is_clue(i) {
                assert_eq!(engine.cells[i], WHITE, "clue cell {i}");
            } else {
                assert_eq!(engine.cells[i], UNKNOWN, "cell {i}");
            }
        }
    }

    #[test]
    fn painting_a_wall_twice_clears_it() {
        let mut engine = engine_with_puzzle(8, 11);
        let free = (0..64)
            .find(|&i| !engine.puzzle.as_ref().unwrap().is_clue(i))
            .unwrap();
        let (x, y) = engine.puzzle.as_ref().unwrap().grid.xy(free);
        engine.begin_drag(x as i64, y as i64, true);
        assert_eq!(engine.cells[free], BLACK);
        assert_eq!(engine.anchor, Some(free));
        engine.begin_drag(x as i64, y as i64, true);
        assert_eq!(engine.cells[free], UNKNOWN);
        engine.undo();
        assert_eq!(engine.cells[free], BLACK);
        engine.undo();
        assert_eq!(engine.cells[free], UNKNOWN);
    }

    #[test]
    fn one_step_agrees_with_the_solution() {
        let mut engine = engine_with_puzzle(10, 3);
        engine.one_step();
        let puzzle = engine.puzzle.as_ref().unwrap();
        for i in 0..engine.cells.len() {
            if engine.cells[i] != UNKNOWN && !puzzle.is_clue(i) {
                assert_eq!(engine.cells[i], puzzle.solution[i], "cell {i}");
            }
        }
    }

    #[test]
    fn check_marks_wrong_cells_only() {
        let mut engine = engine_with_puzzle(8, 5);
        let puzzle = engine.puzzle.as_ref().unwrap();
        let (mut black, mut white) = (None, None);
        for i in 0..engine.cells.len() {
            if puzzle.is_clue(i) {
                continue;
            }
            if puzzle.solution[i] == BLACK && black.is_none() {
                black = Some(i);
            }
            if puzzle.solution[i] == WHITE && white.is_none() {
                white = Some(i);
            }
        }
        let black = black.unwrap();
        let white = white.unwrap();
        engine.set_cell(black, WHITE);
        engine.set_cell(white, BLACK);
        engine.check();
        assert!(engine.wrong[black]);
        assert!(engine.wrong[white]);
        assert_eq!(engine.wrong.iter().filter(|&&w| w).count(), 2);
    }

    #[test]
    fn state_json_carries_the_board() {
        let engine = engine_with_puzzle(6, 9);
        let json = engine.state_json("", false);
        assert!(json.contains("\"w\":6"));
        assert!(json.contains("\"cells\":\""), "{json}");
        assert!(json.contains("\"clues\":\""), "{json}");
        assert!(json.starts_with("{\"ok\":true"));

        let with_fields = engine.state_json("", true);
        assert!(with_fields.contains("\"fields\":["), "{with_fields}");
        assert!(with_fields.contains("\"key\":\"max_attempts\""), "{with_fields}");
        assert!(with_fields.contains("\"help\":\""), "{with_fields}");
    }

    #[test]
    fn a_tap_cycles_wall_empty_unmarked() {
        let mut engine = engine_with_puzzle(8, 21);
        let free = (0..64)
            .find(|&i| !engine.puzzle.as_ref().unwrap().is_clue(i))
            .unwrap();
        let (x, y) = engine.puzzle.as_ref().unwrap().grid.xy(free);

        engine.tap_cell(x as i64, y as i64);
        assert_eq!(engine.cells[free], BLACK, "first tap marks a wall");
        assert_eq!(engine.anchor, Some(free), "the new wall is spotlighted");

        engine.tap_cell(x as i64, y as i64);
        assert_eq!(engine.cells[free], WHITE, "second tap marks empty");
        assert_eq!(engine.anchor, None);

        engine.tap_cell(x as i64, y as i64);
        assert_eq!(engine.cells[free], UNKNOWN, "third tap clears the cell");

        // every tap is a normal, undoable move
        engine.undo();
        assert_eq!(engine.cells[free], WHITE);
        engine.undo();
        assert_eq!(engine.cells[free], BLACK);
        engine.undo();
        assert_eq!(engine.cells[free], UNKNOWN);
    }

    #[test]
    fn a_tap_on_a_clue_steps_it_instead_of_marking_it() {
        let mut engine = engine_with_puzzle(10, 22);
        let clue = engine.puzzle.as_ref().unwrap().clues[0].0;
        let (x, y) = engine.puzzle.as_ref().unwrap().grid.xy(clue);
        let before = engine.cells.clone();

        engine.tap_cell(x as i64, y as i64);

        assert_eq!(engine.cells[clue], before[clue], "the clue cell itself is untouched");
        let filled = (0..engine.cells.len())
            .filter(|&i| engine.cells[i] != before[i])
            .count();
        assert!(filled > 0, "the clue step should fill something: {}", engine.status);
        assert!(engine.status.starts_with("Clue ("), "{}", engine.status);
    }

    #[test]
    fn taps_do_nothing_while_the_solution_is_shown() {
        let mut engine = engine_with_puzzle(8, 23);
        let free = (0..64)
            .find(|&i| !engine.puzzle.as_ref().unwrap().is_clue(i))
            .unwrap();
        let (x, y) = engine.puzzle.as_ref().unwrap().grid.xy(free);
        engine.toggle_solution();
        engine.tap_cell(x as i64, y as i64);
        assert_eq!(engine.cells[free], UNKNOWN);
    }

    #[test]
    fn unknown_commands_are_reported() {
        let mut engine = Engine::new();
        assert!(engine.command("nonsense").is_err());
        assert!(engine.command("cluestep 1").is_err());
        assert!(engine.command("paint sideways 0 0").is_err());
    }

    #[test]
    fn settings_round_trip_through_the_engine() {
        let mut engine = Engine::new();
        engine.command("set size 12").unwrap();
        engine.command("set density 0.30-0.40").unwrap();
        assert_eq!(engine.settings.size, 12);
        assert_eq!(engine.settings.density_lo, 0.30);
        assert!(engine.command("set size 999").is_ok());
        assert!(!engine.settings_error.is_empty());
        assert_eq!(engine.settings.size, 12, "rejected value must not stick");
    }
}
