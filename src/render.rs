//! GDI rendering. Everything is drawn into an off-screen bitmap first so the
//! board never flickers.

use std::mem;
use std::ptr;

use winapi::ctypes::c_void;
use winapi::shared::minwindef::UINT;
use winapi::shared::windef::{COLORREF, HBRUSH, HDC, HFONT, HGDIOBJ, HPEN, HWND, RECT};
use winapi::um::wingdi::*;
use winapi::um::winuser::*;

use tapa_core::model::{clue_satisfied, Puzzle, BLACK, WHITE};
use crate::window::{App, StatusKind};

pub const MARGIN: i32 = 24;
/// Width of the always-visible control column on the right.
pub const PANEL_W: i32 = 320;
/// Space between the board and the control column.
pub const PANEL_GAP: i32 = 18;
pub const HEADER: i32 = 62;
/// Room for the two hint lines (they wrap on narrow windows) plus the info line.
pub const FOOTER: i32 = 118;
/// Cell size the window opens with, and the largest a resized window may use.
const INITIAL_MAX_CELL: i32 = 38;
/// Smallest and largest cell the zoom slider allows.
pub const MIN_CELL: i32 = 11;
pub const MAX_CELL: i32 = 96;
/// Height the control column needs, so the window is never shorter than it.
/// `settings_ui` has a test that keeps its layout inside this.
pub const PANEL_MIN_H: i32 = 680;

/// Smallest client width that still fits the control column and a board of
/// `cols` cells at [`MIN_CELL`].
pub fn min_client_w(cols: usize) -> i32 {
    2 * MARGIN + MIN_CELL * cols as i32 + PANEL_GAP + PANEL_W
}

/// Smallest client height: the control column, or the shortest board that still
/// leaves room for the header and the footer.
pub fn min_client_h(rows: usize) -> i32 {
    PANEL_MIN_H.max(HEADER + MIN_CELL * rows as i32 + FOOTER)
}

#[inline]
fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

const C_TEXT: COLORREF = 0x0030_3028; // dark slate
const C_DIM: COLORREF = 0x0088_8070;
const C_GOOD: COLORREF = 0x0030_8A20;
const C_BAD: COLORREF = 0x0020_20C8;
/// Digits of a clue whose numbers the player has already accounted for.
const C_DONE: COLORREF = 0x009c_928f; // grey 143/146/156

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

/// All GDI objects live as long as the window.
pub struct Gfx {
    pub cell: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub client_w: i32,
    pub client_h: i32,
    pub cols: usize,
    pub rows: usize,
    /// Left edge of the control column.
    pub panel_x: i32,

    brush_bg: HBRUSH,
    brush_unknown: HBRUSH,
    brush_empty: HBRUSH,
    brush_black: HBRUSH,
    brush_clue: HBRUSH,
    brush_solution_black: HBRUSH,
    brush_highlight: HBRUSH,
    brush_selection: HBRUSH,
    brush_selected_black: HBRUSH,
    brush_dot: HBRUSH,

    pen_grid: HPEN,
    pen_border: HPEN,
    pen_hover: HPEN,
    pen_wrong: HPEN,
    pen_win: HPEN,
    pen_highlight: HPEN,
    pen_error: HPEN,

    font_title: HFONT,
    font_ui: HFONT,
    font_small: HFONT,
    font_clue: [HFONT; 4],
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    static FONTS: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    // GDI font resources remain registered across window and font recreation.
    FONTS.get_or_init(|| unsafe {
        for bytes in [
            include_bytes!("../web/fonts/Inter-regular.ttf").as_slice(),
            include_bytes!("../web/fonts/Inter-semibold.ttf").as_slice(),
        ] {
            let mut count = 0;
            AddFontMemResourceEx(
                bytes.as_ptr() as *mut c_void,
                bytes.len() as u32,
                ptr::null_mut(),
                &mut count,
            );
        }
    });
    let family = if weight >= 600 {
        "Inter SemiBold\0"
    } else {
        "Inter\0"
    };
    let face: Vec<u16> = family.encode_utf16().collect();
    CreateFontW(
        -height.max(6),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_TT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        face.as_ptr(),
    )
}

/// Client size the window opens with: a `cols x rows` board that fits
/// comfortably inside the usable desktop area (i.e. excluding the taskbar).
pub unsafe fn initial_client_size(cols: usize, rows: usize) -> (i32, i32) {
    let mut work: RECT = mem::zeroed();
    let have_work = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        &mut work as *mut RECT as *mut c_void,
        0,
    ) != 0;
    let (area_w, area_h) = if have_work {
        (work.right - work.left, work.bottom - work.top)
    } else {
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    };

    // leave room for the caption and the window frame
    let chrome = GetSystemMetrics(SM_CYCAPTION) + 2 * GetSystemMetrics(SM_CYFRAME) + 8;
    let avail_w = (area_w as f64 * 0.92) as i32 - 2 * MARGIN - PANEL_GAP - PANEL_W;
    let avail_h = (area_h as f64 * 0.92) as i32 - HEADER - FOOTER - chrome;

    let cell = (avail_h / rows as i32)
        .min(avail_w / cols as i32)
        .clamp(MIN_CELL, INITIAL_MAX_CELL);
    let client_w = 2 * MARGIN + cell * cols as i32 + PANEL_GAP + PANEL_W;
    let client_h = HEADER + cell * rows as i32 + FOOTER;
    (client_w.max(min_client_w(cols)), client_h.max(min_client_h(rows)))
}

/// Cell size that fits a `cols x rows` board into a client area: the largest
/// size that still leaves the control column its width on the right.
pub fn fit_cell(cols: usize, rows: usize, client_w: i32, client_h: i32) -> i32 {
    let panel_x = client_w.max(min_client_w(cols)) - MARGIN - PANEL_W;
    let avail_w = (panel_x - PANEL_GAP - MARGIN).max(MIN_CELL * cols as i32);
    let avail_h = (client_h.max(min_client_h(rows)) - HEADER - FOOTER).max(MIN_CELL * rows as i32);
    (avail_w / cols as i32)
        .min(avail_h / rows as i32)
        .clamp(MIN_CELL, MAX_CELL)
}

/// Top-left corner that centres a `w x h` window inside the work area.
pub unsafe fn centered_origin(w: i32, h: i32) -> (i32, i32) {
    let mut work: RECT = mem::zeroed();
    if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut c_void, 0) == 0 {
        return (0, 0);
    }
    let x = work.left + ((work.right - work.left - w) / 2).max(0);
    let y = work.top + ((work.bottom - work.top - h) / 2).max(0);
    (x, y)
}

impl Gfx {
    pub unsafe fn new(cols: usize, rows: usize) -> Gfx {
        let (client_w, client_h) = initial_client_size(cols, rows);

        let mut gfx = Gfx {
            cell: 0,
            grid_x: MARGIN,
            panel_x: 0,
            grid_y: HEADER,
            client_w,
            client_h,
            cols,
            rows,
            brush_bg: CreateSolidBrush(rgb(246, 247, 250)),
            // an undecided cell and a cell marked empty share their background;
            // the empty mark only adds the small dot
            brush_unknown: CreateSolidBrush(rgb(255, 255, 255)),
            brush_empty: CreateSolidBrush(rgb(255, 255, 255)),
            brush_black: CreateSolidBrush(rgb(42, 45, 60)),
            brush_clue: CreateSolidBrush(rgb(253, 248, 226)),
            brush_solution_black: CreateSolidBrush(rgb(150, 158, 178)),
            brush_highlight: CreateSolidBrush(rgb(64, 104, 176)),
            brush_selection: CreateSolidBrush(rgb(255, 245, 224)),
            brush_selected_black: CreateSolidBrush(rgb(76, 69, 62)),
            brush_dot: CreateSolidBrush(rgb(120, 128, 146)),
            pen_grid: CreatePen(PS_SOLID as i32, 1, rgb(168, 174, 188)),
            pen_border: CreatePen(PS_SOLID as i32, 2, rgb(70, 76, 96)),
            pen_hover: CreatePen(PS_SOLID as i32, 2, rgb(58, 130, 226)),
            pen_wrong: CreatePen(PS_SOLID as i32, 2, rgb(214, 48, 48)),
            pen_win: CreatePen(PS_SOLID as i32, 3, rgb(46, 158, 66)),
            pen_highlight: CreatePen(PS_SOLID as i32, 2, rgb(255, 196, 72)),
            pen_error: CreatePen(PS_SOLID as i32, 2, rgb(226, 52, 52)),
            font_title: make_font(22, 700),
            font_ui: make_font(15, 400),
            font_small: make_font(13, 400),
            font_clue: [ptr::null_mut(); 4],
        };
        gfx.fit(client_w, client_h);
        gfx.rebuild_clue_fonts();
        gfx
    }

    /// Recompute the board geometry for a client size. The control column is
    /// pinned to the right edge, the board takes whatever is left. Returns true
    /// when the cell size changed, i.e. when the clue fonts are stale.
    pub unsafe fn fit(&mut self, client_w: i32, client_h: i32) -> bool {
        let client_w = client_w.max(min_client_w(self.cols));
        let client_h = client_h.max(min_client_h(self.rows));
        self.client_w = client_w;
        self.client_h = client_h;
        self.panel_x = client_w - MARGIN - PANEL_W;

        let cell = fit_cell(self.cols, self.rows, client_w, client_h);
        if cell == self.cell {
            return false;
        }
        self.cell = cell;
        true
    }

    /// Change the board dimensions; call [`Gfx::fit`] afterwards.
    pub fn set_grid(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
    }

    /// Clue digits scale with the cell size, so they are rebuilt on resize.
    pub unsafe fn rebuild_clue_fonts(&mut self) {
        for font in self.font_clue.iter() {
            if !font.is_null() {
                DeleteObject(*font as HGDIOBJ);
            }
        }
        let f = |scale: f64| ((self.cell as f64 * scale).round() as i32).max(7);
        self.font_clue = [
            make_font(f(0.60), 400),
            make_font(f(0.42), 400),
            make_font(f(0.33), 400),
            make_font(f(0.27), 400),
        ];
    }

    /// Right edge of the board area, where text stops so it can never run into
    /// the control column.
    pub fn text_right(&self) -> i32 {
        self.panel_x - PANEL_GAP
    }

    /// Fonts for the settings panel controls.
    pub fn ui_font(&self) -> HFONT {
        self.font_ui
    }

    pub fn small_font(&self) -> HFONT {
        self.font_small
    }

    pub unsafe fn destroy(&mut self) {        for b in [
            self.brush_bg,
            self.brush_unknown,
            self.brush_empty,
            self.brush_black,
            self.brush_clue,
            self.brush_solution_black,
            self.brush_highlight,
            self.brush_selection,
            self.brush_selected_black,
            self.brush_dot,
        ] {
            DeleteObject(b as HGDIOBJ);
        }
        for p in [
            self.pen_grid,
            self.pen_border,
            self.pen_hover,
            self.pen_wrong,
            self.pen_win,
            self.pen_highlight,
            self.pen_error,
        ] {
            DeleteObject(p as HGDIOBJ);
        }
        for f in [
            self.font_title,
            self.font_ui,
            self.font_small,
            self.font_clue[0],
            self.font_clue[1],
            self.font_clue[2],
            self.font_clue[3],
        ] {
            if !f.is_null() {
                DeleteObject(f as HGDIOBJ);
            }
        }
    }
}

unsafe fn text(hdc: HDC, font: HFONT, color: COLORREF, s: &str, mut r: RECT, flags: UINT) {
    let old = SelectObject(hdc, font as HGDIOBJ);
    SetTextColor(hdc, color);
    SetBkMode(hdc, 1); // TRANSPARENT
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    buf.push(0);
    DrawTextW(hdc, buf.as_mut_ptr(), -1, &mut r, flags);
    SelectObject(hdc, old);
}

unsafe fn stroke(hdc: HDC, r: &RECT, pen: HPEN) {
    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));
    Rectangle(hdc, r.left, r.top, r.right, r.bottom);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
}

/// Draw the whole board into `hdc`.
pub unsafe fn paint(app: &App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    let mem_dc = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem_dc, bmp as HGDIOBJ);

    draw(app, mem_dc, w, h);

    BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
    SelectObject(mem_dc, old_bmp);
    DeleteObject(bmp as HGDIOBJ);
    DeleteDC(mem_dc);
    EndPaint(hwnd, &mut ps);
}

unsafe fn draw(app: &App, hdc: HDC, w: i32, h: i32) {
    let g = &app.gfx;
    FillRect(hdc, &rect(0, 0, w, h), g.brush_bg);
    // the control column is one white card, painted before any text so the
    // header stays readable on top of it
    FillRect(
        hdc,
        &rect(g.panel_x - PANEL_GAP / 2, 0, w, h),
        g.brush_empty,
    );

    // ---- header -----------------------------------------------------------
    // Text never crosses into the control column: it stops at the board's right
    // edge, which is also where the grid ends.
    let text_right = g.text_right();
    text(
        hdc,
        g.font_title,
        C_TEXT,
        "Tapa",
        rect(MARGIN, 10, text_right, 38),
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    let (status_color, status_text) = match app.status_kind {
        StatusKind::Good => (C_GOOD, app.status.clone()),
        StatusKind::Bad => (C_BAD, app.status.clone()),
        StatusKind::Info => (C_DIM, app.status.clone()),
    };
    text(
        hdc,
        g.font_ui,
        status_color,
        &status_text,
        rect(MARGIN, 12, text_right, 40),
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );

    // ---- board ------------------------------------------------------------
    if let Some(puzzle) = &app.puzzle {
        let n = puzzle.grid.len();
        if app.cells.len() == n {
            let mut selected=vec![false;n];
            if !app.show_solution { for &i in &app.deduction_selection { if i<n {selected[i]=true;} } }
            // nothing may bleed out of the board area, whatever the window size
            let saved = SaveDC(hdc);
            IntersectClipRect(hdc, 0, HEADER - 2, g.panel_x - 2, h);
            for y in 0..g.rows {
                for x in 0..g.cols {
                    let idx = puzzle.grid.idx(x, y);
                    let px = g.grid_x + x as i32 * g.cell;
                    let py = g.grid_y + y as i32 * g.cell;
                    let r = rect(px, py, px + g.cell, py + g.cell);
                    let clue = puzzle.clue_at(idx);
                    let brush = if clue.is_some() {
                        g.brush_clue
                    } else if app.show_solution {
                        if puzzle.solution[idx] == BLACK {
                            g.brush_solution_black
                        } else {
                            g.brush_empty
                        }
                    } else if selected[idx] {
                        if app.cells[idx]==BLACK {g.brush_selected_black} else {g.brush_selection}
                    } else if app.highlight.get(idx).copied().unwrap_or(false) {
                        g.brush_highlight
                    } else {
                        match app.cells[idx] {
                            BLACK => g.brush_black,
                            WHITE => g.brush_empty,
                            _ => g.brush_unknown,
                        }
                    };
                    FillRect(hdc, &r, brush);

                    // A small square marks a cell the player explicitly
                    // declared empty, so it cannot be confused with an
                    // undecided cell.
                    if !app.show_solution
                        && clue.is_none()
                        && app.cells[idx] == WHITE
                        && !app.highlight.get(idx).copied().unwrap_or(false)
                    {
                        let side = (g.cell / 12).max(2);
                        let dx = px + (g.cell - side + 1) / 2;
                        let dy = py + (g.cell - side + 1) / 2;
                        FillRect(hdc, &rect(dx, dy, dx + side, dy + side), g.brush_dot);
                    }
                }
            }

            // clue numbers
            for y in 0..g.rows {
                for x in 0..g.cols {
                    let idx = puzzle.grid.idx(x, y);
                    let Some(clue) = puzzle.clue_at(idx) else {
                        continue;
                    };
                    let px = g.grid_x + x as i32 * g.cell;
                    let py = g.grid_y + y as i32 * g.cell;
                    let r = rect(px, py, px + g.cell, py + g.cell);
                    let color = if app.errors.clues.get(idx).copied().unwrap_or(false) {
                        C_BAD
                    } else if !app.show_solution
                        && clue_satisfied(&puzzle.grid, &app.cells, idx, clue)
                    {
                        // the numbers are all accounted for: grey them out
                        C_DONE
                    } else {
                        C_TEXT
                    };
                    draw_clue(hdc, g, clue, r, color);
                }
            }

            // grid lines, but never between two cells that are already filled
            // in: a wall group (or the spotlight) then reads as one solid shape
            let solid = |idx: usize| -> bool {
                if puzzle.is_clue(idx) {
                    return false;
                }
                if app.show_solution {
                    return puzzle.solution[idx] == BLACK;
                }
                if app.highlight.get(idx).copied().unwrap_or(false) {
                    return true;
                }
                app.cells[idx] == BLACK
            };
            let old = SelectObject(hdc, g.pen_grid as HGDIOBJ);
            for x in 0..=g.cols {
                let px = g.grid_x + x as i32 * g.cell;
                for y in 0..g.rows {
                    let left = if x > 0 {
                        Some(puzzle.grid.idx(x - 1, y))
                    } else {
                        None
                    };
                    let right = if x < g.cols {
                        Some(puzzle.grid.idx(x, y))
                    } else {
                        None
                    };
                    if let (Some(a), Some(b)) = (left, right) {
                        if solid(a) && solid(b) {
                            continue;
                        }
                    }
                    MoveToEx(hdc, px, g.grid_y + y as i32 * g.cell, ptr::null_mut());
                    LineTo(hdc, px, g.grid_y + (y as i32 + 1) * g.cell);
                }
            }
            for y in 0..=g.rows {
                let py = g.grid_y + y as i32 * g.cell;
                for x in 0..g.cols {
                    let above = if y > 0 {
                        Some(puzzle.grid.idx(x, y - 1))
                    } else {
                        None
                    };
                    let below = if y < g.rows {
                        Some(puzzle.grid.idx(x, y))
                    } else {
                        None
                    };
                    if let (Some(a), Some(b)) = (above, below) {
                        if solid(a) && solid(b) {
                            continue;
                        }
                    }
                    MoveToEx(hdc, g.grid_x + x as i32 * g.cell, py, ptr::null_mut());
                    LineTo(hdc, g.grid_x + (x as i32 + 1) * g.cell, py);
                }
            }
            SelectObject(hdc, old);

            // spotlighted wall group (outline only, so the group reads as one)
            for idx in 0..n {
                if app.highlight.get(idx).copied().unwrap_or(false) {
                    outline_cell(hdc, g, puzzle, &app.highlight, g.pen_highlight, idx);
                }
            }

            // live rule violations
            for idx in 0..n {
                if app.errors.cells.get(idx).copied().unwrap_or(false) {
                    outline_cell(hdc, g, puzzle, &app.errors.cells, g.pen_error, idx);
                }
            }

            for &idx in &app.deduction_selection {
                if app.show_solution {break;}
                let (x,y)=puzzle.grid.xy(idx);
                let px=g.grid_x+x as i32*g.cell;
                let py=g.grid_y+y as i32*g.cell;
                stroke(hdc,&rect(px+1,py+1,px+g.cell-1,py+g.cell-1),g.pen_highlight);
            }

            // hover highlight
            if let Some(hover) = app.hover {
                if !puzzle.is_clue(hover) && !app.show_solution {
                    let (x, y) = puzzle.grid.xy(hover);
                    let px = g.grid_x + x as i32 * g.cell;
                    let py = g.grid_y + y as i32 * g.cell;
                    stroke(hdc, &rect(px + 1, py + 1, px + g.cell - 1, py + g.cell - 1), g.pen_hover);
                }
            }

            // wrong cells from the last check
            let pen_old = SelectObject(hdc, g.pen_wrong as HGDIOBJ);
            for idx in 0..n {
                if !app.wrong.get(idx).copied().unwrap_or(false) {
                    continue;
                }
                let (x, y) = puzzle.grid.xy(idx);
                let px = g.grid_x + x as i32 * g.cell;
                let py = g.grid_y + y as i32 * g.cell;
                let r = rect(px + 2, py + 2, px + g.cell - 2, py + g.cell - 2);
                let brush_old = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));
                Rectangle(hdc, r.left, r.top, r.right, r.bottom);
                SelectObject(hdc, brush_old);
                let inset = g.cell / 5;
                MoveToEx(hdc, px + inset, py + inset, ptr::null_mut());
                LineTo(hdc, px + g.cell - inset, py + g.cell - inset);
                MoveToEx(hdc, px + g.cell - inset, py + inset, ptr::null_mut());
                LineTo(hdc, px + inset, py + g.cell - inset);
            }
            SelectObject(hdc, pen_old);

            // outer frame
            let border = rect(
                g.grid_x - 2,
                g.grid_y - 2,
                g.grid_x + g.cols as i32 * g.cell + 2,
                g.grid_y + g.rows as i32 * g.cell + 2,
            );
            stroke(hdc, &border, if app.solved { g.pen_win } else { g.pen_border });
            RestoreDC(hdc, saved);
        }
    } else {
        text(
            hdc,
            g.font_ui,
            C_DIM,
            "Generating a puzzle with a unique solution...",
            rect(MARGIN, HEADER + 40, text_right, HEADER + 80),
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
    }

    // ---- footer -----------------------------------------------------------
    // The two hints wrap inside the board column, so a narrow window keeps them
    // out of the control column instead of running underneath it.
    let top = g.grid_y + g.rows as i32 * g.cell + 12;
    let line_h = 21;
    text(
        hdc,
        g.font_small,
        C_DIM,
        "Left click: wall   Right click: empty   click the same mark again: clear   drag: paint a stroke",
        rect(MARGIN, top, text_right, top + 2 * line_h),
        DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL,
    );
    text(
        hdc,
        g.font_small,
        C_DIM,
        "Shift + hover: spotlight a wall group   middle click or Alt+click a clue: step just it   Z: undo   Esc: quit",
        rect(MARGIN, top + 2 * line_h, text_right, top + 4 * line_h),
        DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL,
    );
    if let Some(puzzle) = &app.puzzle {
        let broken = if app.errors.any() {
            "   red = rule broken"
        } else {
            ""
        };
        let info = format!(
            "seed {}    {} clues{}",
            puzzle.seed,
            puzzle.clues.len(),
            broken
        );
        text(
            hdc,
            g.font_small,
            C_DIM,
            &info,
            rect(MARGIN, top + 4 * line_h, text_right, top + 5 * line_h),
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
    }
}

/// Outline the border of cell `idx`, drawing only the edges that face a cell
/// which is not part of the same group. This makes a group of cells read as a
/// single shape instead of a grid of boxes.
unsafe fn outline_cell(hdc: HDC, g: &Gfx, puzzle: &Puzzle, group: &[bool], pen: HPEN, idx: usize) {
    let (x, y) = puzzle.grid.xy(idx);
    let in_group = |xx: i32, yy: i32| -> bool {
        if xx < 0 || yy < 0 || xx as usize >= puzzle.grid.w || yy as usize >= puzzle.grid.h {
            return false;
        }
        group
            .get(puzzle.grid.idx(xx as usize, yy as usize))
            .copied()
            .unwrap_or(false)
    };
    let px = g.grid_x + x as i32 * g.cell;
    let py = g.grid_y + y as i32 * g.cell;
    let (x0, y0, x1, y1) = (px, py, px + g.cell, py + g.cell);

    let old = SelectObject(hdc, pen as HGDIOBJ);
    if !in_group(x as i32, y as i32 - 1) {
        MoveToEx(hdc, x0, y0, ptr::null_mut());
        LineTo(hdc, x1, y0);
    }
    if !in_group(x as i32, y as i32 + 1) {
        MoveToEx(hdc, x0, y1, ptr::null_mut());
        LineTo(hdc, x1, y1);
    }
    if !in_group(x as i32 - 1, y as i32) {
        MoveToEx(hdc, x0, y0, ptr::null_mut());
        LineTo(hdc, x0, y1);
    }
    if !in_group(x as i32 + 1, y as i32) {
        MoveToEx(hdc, x1, y0, ptr::null_mut());
        LineTo(hdc, x1, y1);
    }
    SelectObject(hdc, old);
}

/// Clue numbers, laid out in a grid of at most two columns.
unsafe fn draw_clue(hdc: HDC, g: &Gfx, clue: &[u8], r: RECT, color: COLORREF) {
    let count = clue.len();
    let font = match count {
        0 | 1 => g.font_clue[0],
        2 => g.font_clue[1],
        3 => g.font_clue[2],
        _ => g.font_clue[3],
    };
    for (slot, value) in clue_slots(count, r.right - r.left, r.bottom - r.top)
        .into_iter()
        .zip(clue.iter())
    {
        let cell = rect(
            r.left + slot.left,
            r.top + slot.top,
            r.left + slot.right,
            r.top + slot.bottom,
        );
        let s = ((b'0' + value) as char).to_string();
        let old_font = SelectObject(hdc, font as HGDIOBJ);
        let mut transform: MAT2 = mem::zeroed();
        transform.eM11.value = 1;
        transform.eM22.value = 1;
        let mut bounds: GLYPHMETRICS = mem::zeroed();
        let measured = GetGlyphOutlineW(
            hdc,
            (b'0' + value) as u32,
            GGO_METRICS,
            &mut bounds,
            0,
            ptr::null_mut(),
            &transform,
        );
        if measured != GDI_ERROR && bounds.gmBlackBoxY > 0 {
            let old_align = SetTextAlign(hdc, TA_LEFT | TA_BASELINE);
            let old_mode = SetBkMode(hdc, TRANSPARENT as i32);
            let old_color = SetTextColor(hdc, color);
            let x =
                (cell.left + cell.right - bounds.gmBlackBoxX as i32) / 2 - bounds.gmptGlyphOrigin.x;
            let y =
                (cell.top + cell.bottom - bounds.gmBlackBoxY as i32) / 2 + bounds.gmptGlyphOrigin.y;
            let glyph = [(b'0' + value) as u16];
            TextOutW(hdc, x, y, glyph.as_ptr(), 1);
            SetTextColor(hdc, old_color);
            SetBkMode(hdc, old_mode);
            SetTextAlign(hdc, old_align);
        } else {
            text(
                hdc,
                font,
                color,
                &s,
                cell,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
        SelectObject(hdc, old_font);
    }
}

/// Where the digits of a clue sit inside its `w` x `h` cell: at most two per
/// row, in reading order, with every row centred. Three numbers therefore read
/// `a b` with a centred `c` below, instead of `c` stuck under the left digit.
/// The numbers themselves have no fixed order - a clue is read clockwise from
/// any starting point - so the layout only has to look balanced.
fn clue_slots(count: usize, w: i32, h: i32) -> Vec<RECT> {
    let cols = if count <= 2 { count.max(1) } else { 2 };
    let rows = (count + cols - 1) / cols;
    let cw = w / cols as i32;
    let ch = if count == 3 {
        h * 2 / 5
    } else {
        h / rows.max(1) as i32
    };
    let y0 = if count == 3 {
        (h * 16 + 50) / 100
    } else {
        (h - ch * rows as i32) / 2
    };
    let mut slots = Vec::with_capacity(count);
    for row in 0..rows {
        let first = row * cols;
        let in_row = count.min(first + cols) - first;
        let x0 = (w - cw * in_row as i32) / 2;
        for k in 0..in_row {
            slots.push(rect(
                x0 + k as i32 * cw,
                y0 + row as i32 * ch,
                x0 + (k as i32 + 1) * cw,
                y0 + (row as i32 + 1) * ch,
            ));
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_shows_38_pixel_cells() {
        // the client size the window opens with for a 20x20 board
        let (w, h) = (2 * MARGIN + 38 * 20 + PANEL_GAP + PANEL_W, HEADER + 38 * 20 + FOOTER);
        assert_eq!(fit_cell(20, 20, w, h), 38);
    }

    #[test]
    fn a_bigger_window_gets_bigger_cells() {
        let small = fit_cell(20, 20, 1146, 940);
        let big = fit_cell(20, 20, 2000, 1400);
        assert!(big > small, "{big} should be bigger than {small}");
    }

    #[test]
    fn cells_never_shrink_below_the_minimum_or_grow_past_the_maximum() {
        assert_eq!(fit_cell(60, 60, 400, 400), MIN_CELL);
        assert_eq!(fit_cell(3, 3, 4000, 3000), MAX_CELL);
    }

    #[test]
    fn the_board_always_fits_beside_the_control_column() {
        for size in [3usize, 8, 20, 40, 60] {
            for (w, h) in [(419, 680), (800, 700), (1146, 940), (2000, 1400), (900, 1600)] {
                let cell = fit_cell(size, size, w, h);
                let panel_x = w.max(min_client_w(size)) - MARGIN - PANEL_W;
                assert!(
                    cell * size as i32 <= panel_x - PANEL_GAP - MARGIN,
                    "{size}x{size} in {w}x{h}: cells of {cell} do not fit"
                );
                assert!(
                    HEADER + cell * size as i32 + FOOTER <= h.max(min_client_h(size)),
                    "{size}x{size} in {w}x{h}: the board does not fit vertically"
                );
            }
        }
    }

    #[test]
    fn the_minimum_window_fits_the_column_and_a_three_cell_board() {
        assert_eq!(min_client_w(3), 2 * MARGIN + 3 * MIN_CELL + PANEL_GAP + PANEL_W);
        assert_eq!(min_client_h(3), PANEL_MIN_H);
        assert_eq!(min_client_h(60), HEADER + 60 * MIN_CELL + FOOTER);
        assert!(min_client_h(60) > PANEL_MIN_H);
    }

    #[test]
    fn a_three_number_clue_centres_its_lower_number() {
        let (w, h) = (40, 40);
        let slots = clue_slots(3, w, h);
        assert_eq!(slots.len(), 3);
        // the top row holds two numbers at the quarter and three-quarter mark
        assert_eq!(slots[0].left, 0);
        assert_eq!(slots[0].top, 6);
        assert_eq!(slots[0].bottom, 22);
        assert_eq!(slots[1].left, 20);
        assert_eq!(slots[1].top, 6);
        // the single number below sits in the middle of the cell
        assert_eq!(slots[2].left, 10);
        assert_eq!(slots[2].right, 30);
        assert_eq!(slots[2].top, 22);
        assert_eq!(slots[2].bottom, 38);
        // an odd cell width stays inside the cell and keeps the row centred
        let odd = clue_slots(3, 41, 41);
        assert_eq!(odd[2].left, 10);
        assert_eq!(odd[2].right, 30);
        assert!(odd[2].right <= 41);
    }

    #[test]
    fn clue_digits_never_leave_their_cell() {
        for count in 1..=4usize {
            for cell in [11i32, 17, 38, 96] {
                let slots = clue_slots(count, cell, cell);
                assert_eq!(slots.len(), count);
                for slot in slots {
                    assert!(slot.left >= 0 && slot.top >= 0);
                    assert!(slot.right <= cell && slot.bottom <= cell);
                    assert!(slot.right > slot.left && slot.bottom > slot.top);
                }
            }
        }
    }

    #[test]
    fn one_and_two_number_clues_keep_their_own_rows() {
        assert_eq!(clue_slots(1, 40, 40)[0].left, 0);
        assert_eq!(clue_slots(1, 40, 40)[0].right, 40);
        let two = clue_slots(2, 40, 40);
        assert_eq!((two[0].left, two[1].left), (0, 20));
        assert_eq!(two[0].top, 0);
        assert_eq!(two[0].bottom, 40);
        let four = clue_slots(4, 40, 40);
        assert_eq!((four[0].left, four[1].left, four[2].left), (0, 20, 0));
        assert_eq!((four[2].top, four[3].top), (20, 20));
    }
}






