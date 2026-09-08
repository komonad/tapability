//! Win32 window, application state and input handling.

use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::{Arc, Mutex};
use std::thread;

use winapi::shared::minwindef::*;
use winapi::shared::windef::*;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::*;

use crate::config::Settings;
use crate::model::{
    live_errors, validate, wall_component, LiveErrors, Puzzle, BLACK, UNKNOWN, WHITE,
};
use crate::render::{self, Gfx};
use crate::settings_ui;

const WM_PUZZLE_READY: UINT = WM_APP + 1;

/// How many marks can be undone. Holding Z walks back through them.
const MAX_HISTORY: usize = 4096;

// winapi 0.3.9 does not define virtual key codes for letter keys.
const VK_ESCAPE_KEY: i32 = 0x1B;
const VK_SHIFT_KEY: i32 = 0x10;
const VK_C_KEY: i32 = b'C' as i32;
const VK_N_KEY: i32 = b'N' as i32;
const VK_R_KEY: i32 = b'R' as i32;
const VK_S_KEY: i32 = b'S' as i32;
const VK_Z_KEY: i32 = b'Z' as i32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Good,
    Bad,
}

type Slot = Arc<Mutex<Option<Result<Puzzle, String>>>>;

/// One generation run. Each run gets its own slot so that a stale run can never
/// overwrite (or steal) the result of a newer one.
struct Pending {
    seed: u64,
    slot: Slot,
}

/// One paint stroke: the value it applies and the cell it last touched.
struct Drag {
    value: u8,
    last: usize,
}

pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub size: usize,
    pub puzzle: Option<Puzzle>,
    /// Player marks: UNKNOWN / BLACK / WHITE, one entry per cell.
    pub cells: Vec<u8>,
    pub wrong: Vec<bool>,
    pub hover: Option<usize>,
    pub status: String,
    pub status_kind: StatusKind,
    pub solved: bool,
    pub generating: bool,
    pub show_solution: bool,
    pub seed: u64,
    /// Rule violations in the current (possibly unfinished) board.
    pub errors: LiveErrors,
    /// Cells of the wall group that is currently spotlighted.
    pub highlight: Vec<bool>,
    /// Undo stack of (cell, previous value), most recent last.
    history: VecDeque<(usize, u8)>,
    /// Wall cell whose group is spotlighted, if any.
    highlight_anchor: Option<usize>,
    /// Whether the spotlight comes from Shift-hovering rather than a click.
    highlight_from_hover: bool,
    /// Cell that was changed last, used to break ties in the error analysis.
    last_changed: Option<usize>,
    /// Active mouse drag, if a button is held down.
    drag: Option<Drag>,
    pub settings: Settings,
    pub settings_path: std::path::PathBuf,
    pub controls: Option<settings_ui::Controls>,
    pending: Option<Pending>,
}

impl App {
    fn new(hwnd: HWND, gfx: Gfx, settings: Settings, settings_path: std::path::PathBuf, seed: u64) -> App {
        let size = settings.size;
        App {
            hwnd,
            gfx,
            size,
            puzzle: None,
            cells: Vec::new(),
            wrong: Vec::new(),
            hover: None,
            status: "Generating puzzle...".to_string(),
            status_kind: StatusKind::Info,
            solved: false,
            generating: false,
            show_solution: false,
            seed,
            errors: LiveErrors {
                cells: Vec::new(),
                clues: Vec::new(),
            },
            highlight: Vec::new(),
            history: VecDeque::new(),
            highlight_anchor: None,
            highlight_from_hover: false,
            last_changed: None,
            drag: None,
            settings,
            settings_path,
            controls: None,
            pending: None,
        }
    }

    /// Fresh player marks: clue cells are given as empty, everything else blank.
    fn fresh_marks(puzzle: &Puzzle) -> Vec<u8> {
        let mut cells = vec![UNKNOWN; puzzle.grid.len()];
        for (idx, _) in puzzle.clues.iter() {
            cells[*idx] = WHITE;
        }
        cells
    }

    fn cell_at(&self, x: i32, y: i32) -> Option<usize> {
        let puzzle = self.puzzle.as_ref()?;
        let gx = x - self.gfx.grid_x;
        let gy = y - self.gfx.grid_y;
        if gx < 0 || gy < 0 {
            return None;
        }
        let cx = (gx / self.gfx.cell) as usize;
        let cy = (gy / self.gfx.cell) as usize;
        if cx >= puzzle.grid.w || cy >= puzzle.grid.h {
            return None;
        }
        Some(puzzle.grid.idx(cx, cy))
    }

    /// Start a paint stroke. The first cell decides whether the stroke paints
    /// or erases: pressing on a cell that already carries the mark clears it,
    /// and dragging then keeps clearing.
    fn begin_drag(&mut self, x: i32, y: i32, left: bool) {
        if self.generating || self.show_solution {
            return;
        }
        let Some(idx) = self.cell_at(x, y) else {
            return;
        };
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let want = if left { BLACK } else { WHITE };
        let value = if puzzle.is_clue(idx) {
            want // a clue cell cannot be toggled, so just use the button
        } else if self.cells[idx] == want {
            UNKNOWN
        } else {
            want
        };
        self.drag = Some(Drag { value, last: idx });
        self.paint_cell(idx, value);
        self.highlight_from_hover = false;
        self.highlight_anchor = if value == BLACK { Some(idx) } else { None };
        self.refresh_highlight();
    }

    /// Continue the stroke to the cell under the cursor, filling the cells in
    /// between so that a fast drag does not leave gaps.
    fn drag_to(&mut self, x: i32, y: i32) {
        let Some(drag) = self.drag.as_ref() else {
            return;
        };
        let Some(idx) = self.cell_at(x, y) else {
            return;
        };
        if idx == drag.last {
            return;
        }
        let value = drag.value;
        let last = drag.last;
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
        self.highlight_anchor = if value == BLACK { Some(idx) } else { None };
        self.refresh_highlight();
    }

    fn end_drag(&mut self) {
        self.drag = None;
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

    /// Undo the most recent mark. Holding Z repeats this through key repeat.
    /// Show or hide the solution (the S button / key).
    pub fn toggle_solution(&mut self) {
        if self.puzzle.is_some() {
            self.show_solution = !self.show_solution;
            self.status = if self.show_solution {
                "Showing the solution (S to hide)".to_string()
            } else {
                "Solution hidden".to_string()
            };
            self.status_kind = StatusKind::Info;
        }
    }

    pub fn undo(&mut self) {
        if self.show_solution {
            return;
        }
        let Some((cell, previous)) = self.history.pop_back() else {
            self.status = "Nothing to undo.".to_string();
            self.status_kind = StatusKind::Info;
            return;
        };
        self.cells[cell] = previous;
        self.highlight_from_hover = false;
        self.highlight_anchor = None;
        self.after_change(Some(cell));
        self.refresh_highlight();
    }

    /// Recompute everything that depends on the marks.
    fn after_change(&mut self, focus: Option<usize>) {
        if focus.is_some() {
            self.last_changed = focus;
        }
        for w in self.wrong.iter_mut() {
            *w = false;
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

    /// Recompute the spotlight from `highlight_anchor`.
    fn refresh_highlight(&mut self) {
        let Some(puzzle) = self.puzzle.as_ref() else {
            self.highlight = Vec::new();
            return;
        };
        match self.highlight_anchor {
            Some(c) if self.cells.get(c).copied() == Some(BLACK) => {
                self.highlight = wall_component(&puzzle.grid, &self.cells, c);
            }
            _ => {
                self.highlight = vec![false; puzzle.grid.len()];
            }
        }
    }

    /// Shift-hovering spotlights the wall group under the cursor.
    fn update_hover_highlight(&mut self, shift_down: bool) {
        if !shift_down {
            if self.highlight_from_hover {
                self.highlight_from_hover = false;
                self.highlight_anchor = None;
                self.refresh_highlight();
            }
            return;
        }
        self.highlight_from_hover = true;
        self.highlight_anchor = self
            .hover
            .filter(|&c| self.cells.get(c).copied() == Some(BLACK));
        self.refresh_highlight();
    }

    pub fn reset(&mut self) {
        let Some(puzzle) = self.puzzle.as_ref() else {
            return;
        };
        let fresh = App::fresh_marks(puzzle);
        for i in 0..self.cells.len() {
            if self.cells[i] != fresh[i] {
                self.history.push_back((i, self.cells[i]));
                while self.history.len() > MAX_HISTORY {
                    self.history.pop_front();
                }
            }
        }
        self.cells = fresh;
        self.highlight_anchor = None;
        self.highlight_from_hover = false;
        self.after_change(None);
        self.refresh_highlight();
    }

    /// Compare the player's marks against the (unique) solution.
    pub fn check(&mut self) {
        let (mut wrong, count, total) = match self.puzzle.as_ref() {
            None => return,
            Some(puzzle) => {
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
                (wrong, count, self.cells.len())
            }
        };
        if wrong.len() != total {
            wrong.resize(total, false);
        }
        self.wrong = wrong;
        self.status = if count == 0 {
            "No mistakes so far.".to_string()
        } else {
            format!("{count} wrong cell(s) marked in red")
        };
        self.status_kind = if count == 0 { StatusKind::Good } else { StatusKind::Bad };
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
            self.status = format!(
                "{empty} cell{} left",
                if empty == 1 { "" } else { "s" }
            );
            self.status_kind = StatusKind::Info;
        } else if errors.is_empty() {
            self.solved = true;
            self.status = "Solved! Press N for a new puzzle.".to_string();
            self.status_kind = StatusKind::Good;
        } else {
            self.solved = false;
            self.status = format!(
                "Filled, but {} rule violation(s) - press C to see them",
                errors.len()
            );
            self.status_kind = StatusKind::Bad;
        }
    }

    pub fn new_puzzle(&mut self, seed: u64) {
        self.seed = seed;
        self.generating = true;
        self.solved = false;
        self.show_solution = false;
        self.puzzle = None;
        self.cells.clear();
        self.wrong.clear();
        self.highlight.clear();
        self.history.clear();
        self.highlight_anchor = None;
        self.highlight_from_hover = false;
        self.last_changed = None;
        self.errors = LiveErrors {
            cells: Vec::new(),
            clues: Vec::new(),
        };
        self.hover = None;
        self.status = "Generating a puzzle with a unique solution...".to_string();
        self.status_kind = StatusKind::Info;

        let slot: Slot = Arc::new(Mutex::new(None));
        self.pending = Some(Pending {
            seed,
            slot: slot.clone(),
        });
        let hwnd = self.hwnd as isize;
        let settings = self.settings.clone();
        thread::spawn(move || {
            let cfg = crate::generator::GenConfig::from_settings(&settings);
            let mut rng = crate::rng::Rng::new(seed);
            let result = crate::generator::generate_with(&mut rng, &cfg)
                .map(|r| r.puzzle)
                .ok_or_else(|| format!("generation failed (seed {seed})"));
            *slot.lock().unwrap() = Some(result);
            unsafe {
                PostMessageW(hwnd as HWND, WM_PUZZLE_READY, 0, 0);
            }
        });
    }

    fn on_puzzle_ready(&mut self) {
        // Only the slot of the generation we are currently waiting for counts;
        // results of superseded runs are simply dropped.
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let seed = pending.seed;
        let Some(result) = pending.slot.lock().unwrap().take() else {
            return;
        };
        self.pending = None;
        self.generating = false;
        match result {
            Ok(mut puzzle) => {
                puzzle.seed = seed;
                let n = puzzle.grid.len();
                self.cells = App::fresh_marks(&puzzle);
                self.wrong = vec![false; n];
                self.highlight = vec![false; n];
                self.puzzle = Some(puzzle);
                self.refresh_derived();
                self.refresh_status();
            }
            Err(err) => {
                self.status = err;
                self.status_kind = StatusKind::Bad;
            }
        }
    }

    /// Rebuild the board layout for the current settings and start over.
    /// Called after the settings panel applied a new configuration.
    pub fn apply_settings(&mut self, size_changed: bool) {
        unsafe {
            if size_changed {
                let size = self.settings.size;
                self.gfx.destroy();
                self.gfx = Gfx::new(size, size);
                self.size = size;

                let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
                let mut rc = RECT {
                    left: 0,
                    top: 0,
                    right: self.gfx.client_w,
                    bottom: self.gfx.client_h,
                };
                AdjustWindowRectEx(&mut rc, style, 0, 0);
                let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
                let (x, y) = render::centered_origin(w, h);
                SetWindowPos(
                    self.hwnd,
                    ptr::null_mut(),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                settings_ui::refresh_fonts(self);
                settings_ui::relayout(self);
            }
            let title: Vec<u16> = format!("Tapa {}x{} - {}\0", self.size, self.size, self.settings.summary())
                .encode_utf16()
                .collect();
            SetWindowTextW(self.hwnd, title.as_ptr());
            InvalidateRect(self.hwnd, ptr::null(), 0);
        }
        self.new_puzzle(self.seed.wrapping_add(1));
    }

    fn on_key(&mut self, vk: i32) {
        match vk {
            VK_ESCAPE_KEY => unsafe {
                DestroyWindow(self.hwnd);
            },
            VK_N_KEY => self.new_puzzle(self.seed.wrapping_add(1)),
            VK_R_KEY => self.reset(),
            VK_C_KEY => self.check(),
            VK_Z_KEY => self.undo(),
            VK_SHIFT_KEY => {
                let down = unsafe { GetAsyncKeyState(VK_SHIFT_KEY) < 0 };
                self.update_hover_highlight(down);
            }
            VK_S_KEY => {
                if self.puzzle.is_some() {
                    self.show_solution = !self.show_solution;
                    self.status = if self.show_solution {
                        "Showing the solution (S to hide)".to_string()
                    } else {
                        "Solution hidden".to_string()
                    };
                    self.status_kind = StatusKind::Info;
                }
            }
            _ => {}
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe { self.gfx.destroy() };
    }
}

fn mouse_pos(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Cells on the orthogonal line between two cells, inclusive of both ends.
/// Used so a quick drag paints every cell it crosses.
fn line_cells(grid: &crate::model::Grid, from: usize, to: usize) -> Vec<usize> {
    let (x0, y0) = grid.xy(from);
    let (x1, y1) = grid.xy(to);
    let (mut x, mut y) = (x0 as i32, y0 as i32);
    let (tx, ty) = (x1 as i32, y1 as i32);
    let dx = (tx - x).abs();
    let dy = (ty - y).abs();
    let sx = if x < tx { 1 } else { -1 };
    let sy = if y < ty { 1 } else { -1 };
    let mut err = dx - dy;
    let mut out = Vec::with_capacity((dx + dy) as usize + 1);
    loop {
        out.push(grid.idx(x as usize, y as usize));
        if x == tx && y == ty {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
    out
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCDESTROY {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !ptr.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let app = &mut *ptr;

    match msg {
        WM_COMMAND => {
            settings_ui::handle_command(app, (wparam & 0xFFFF) as i32);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT => {
            settings_ui::color_control(wparam as HDC)
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            render::paint(app, hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = mouse_pos(lparam);
            if app.drag.is_some() {
                app.drag_to(x, y);
                InvalidateRect(hwnd, ptr::null(), 0);
                return 0;
            }
            let cell = app.cell_at(x, y);
            let shift = GetAsyncKeyState(VK_SHIFT_KEY) < 0;
            let moved = cell != app.hover;
            if moved {
                app.hover = cell;
            }
            if shift || app.highlight_from_hover {
                app.update_hover_highlight(shift);
                InvalidateRect(hwnd, ptr::null(), 0);
            } else if moved {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_KEYUP => {
            if wparam as i32 == VK_SHIFT_KEY {
                app.update_hover_highlight(false);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = mouse_pos(lparam);
            app.begin_drag(x, y, true);
            SetCapture(hwnd);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_RBUTTONDOWN => {
            let (x, y) = mouse_pos(lparam);
            app.begin_drag(x, y, false);
            SetCapture(hwnd);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            app.end_drag();
            ReleaseCapture();
            0
        }
        WM_CAPTURECHANGED => {
            app.end_drag();
            0
        }
        WM_KEYDOWN => {
            app.on_key(wparam as i32);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_PUZZLE_READY => {
            app.on_puzzle_ready();
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run(seed: u64, settings: &Settings, settings_path: std::path::PathBuf) -> Result<(), String> {
    unsafe {
        SetProcessDPIAware();

        let size = settings.size;
        let gfx = Gfx::new(size, size);
        let hinst = GetModuleHandleW(ptr::null());
        let class_name: Vec<u16> = "TapaWndClass\0".encode_utf16().collect();

        let mut wc: WNDCLASSW = mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = ptr::null_mut();
        wc.lpszClassName = class_name.as_ptr();
        if RegisterClassW(&wc) == 0 {
            return Err(format!("RegisterClassW failed ({})", GetLastError()));
        }

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: gfx.client_w,
            bottom: gfx.client_h,
        };
        AdjustWindowRectEx(&mut rc, style, 0, 0);
        let (win_w, win_h) = (rc.right - rc.left, rc.bottom - rc.top);
        // centre the window in the work area so it can never land off-screen
        let (win_x, win_y) = render::centered_origin(win_w, win_h);

        let title: Vec<u16> = format!("Tapa {size}x{size} - {}\0", settings.summary()).encode_utf16().collect();
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            win_x,
            win_y,
            win_w,
            win_h,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(format!("CreateWindowExW failed ({})", GetLastError()));
        }

        let app = Box::new(App::new(hwnd, gfx, settings.clone(), settings_path, seed));
        let app_ptr = Box::into_raw(app);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);

        settings_ui::create(&mut *app_ptr);
        (*app_ptr).new_puzzle(seed);

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg: MSG = mem::zeroed();
        loop {
            let got = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if got == 0 {
                break;
            }
            if got == -1 {
                return Err(format!("GetMessageW failed ({})", GetLastError()));
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}









