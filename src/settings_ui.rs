//! The always-visible control column on the right of the game window.
//!
//! Every action the game has (new puzzle, clear, check, solution, undo) is a
//! button here, and the generation settings are editable in place. The controls
//! are real Win32 children of the main window, so typing, tabbing and focus
//! behave normally.

use std::ptr;

use winapi::shared::minwindef::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use winapi::shared::windef::{HBRUSH, HDC, HWND};
use winapi::um::commctrl::{
    InitCommonControlsEx, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, TBM_GETPOS, TBM_SETPAGESIZE,
    TBM_SETPOS, TBM_SETRANGE, TBM_SETTICFREQ, TBS_HORZ, TBS_NOTICKS, TB_ENDTRACK,
};
use winapi::um::wingdi::{GetStockObject, SetBkColor, SetBkMode, WHITE_BRUSH};
use winapi::um::winuser::*;

use tapa_core::config::{Settings, FIELDS};
use crate::render::{HEADER, MAX_CELL, MIN_CELL, PANEL_W};
use crate::window::App;

pub const ID_APPLY: i32 = 1;
pub const ID_NEW: i32 = 3;
pub const ID_CLEAR: i32 = 4;
pub const ID_CHECK: i32 = 5;
pub const ID_SOLUTION: i32 = 6;
pub const ID_UNDO: i32 = 7;
pub const ID_DEFAULTS: i32 = 8;
pub const ID_ONE_STEP: i32 = 9;
const ID_EDIT_BASE: i32 = 100;

const ROW_H: i32 = 28;
const BTN_H: i32 = 30;
const LABEL_W: i32 = 132;
const EDIT_W: i32 = 150;
const FIELD_TOP: i32 = HEADER + 150;
/// The cell-size slider gets its own row between the size field and the rest.
const SLIDER_H: i32 = 24;
/// Rows used by the settings block: one per field plus the slider row.
const FIELD_ROWS: i32 = FIELDS.len() as i32 + 1;
const MESSAGE_TOP: i32 = FIELD_TOP + FIELD_ROWS * ROW_H + 76;
const HELP_TOP: i32 = MESSAGE_TOP + 46;

/// Top of settings field `index`; the slider row sits right after the first
/// field (the board size), so every later field moves down by one row.
fn field_top(index: usize) -> i32 {
    FIELD_TOP + index as i32 * ROW_H + if index >= 1 { ROW_H } else { 0 }
}

/// Top of the cell-size slider row.
fn slider_top() -> i32 {
    FIELD_TOP + ROW_H
}

const SLIDER_HELP: &str = "How many pixels one board cell is drawn at. Dragging it resizes the window so the board zooms in or out; resizing the window moves the slider.";

pub struct Controls {
    pub edits: Vec<HWND>,
    pub buttons: Vec<HWND>,
    pub labels: Vec<HWND>,
    pub message: HWND,
    /// Line that explains whatever the mouse is hovering.
    pub help: HWND,
    /// Cell-size (zoom) slider.
    pub slider: HWND,
    pub slider_label: HWND,
    pub help_default: String,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn set_text(hwnd: HWND, text: &str) {
    let buffer = wide(text);
    SetWindowTextW(hwnd, buffer.as_ptr());
}

unsafe fn get_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

unsafe fn make_button(
    app: &App,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    hinstance: HINSTANCE,
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP | BS_PUSHBUTTON as u32,
        x,
        y,
        w,
        BTN_H,
        app.hwnd,
        id as isize as winapi::shared::windef::HMENU,
        hinstance,
        ptr::null_mut(),
    );
    SendMessageW(hwnd, WM_SETFONT, app.gfx.ui_font() as WPARAM, 1);
    hwnd
}

unsafe fn make_label(
    app: &App,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    hinstance: HINSTANCE,
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        x,
        y,
        w,
        20,
        app.hwnd,
        ptr::null_mut(),
        hinstance,
        ptr::null_mut(),
    );
    SendMessageW(hwnd, WM_SETFONT, app.gfx.ui_font() as WPARAM, 1);
    hwnd
}

/// The board-size slider: drag it, and the puzzle is rebuilt on release.
/// The cell-size slider: it zooms the board, so dragging it resizes the window.
unsafe fn make_slider(
    app: &App,
    x: i32,
    y: i32,
    w: i32,
    hinstance: HINSTANCE,
) -> HWND {
    // trackbars live in the common controls library
    let mut icc: INITCOMMONCONTROLSEX = std::mem::zeroed();
    icc.dwSize = std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32;
    icc.dwICC = ICC_BAR_CLASSES;
    InitCommonControlsEx(&icc);

    let hwnd = CreateWindowExW(
        0,
        wide("msctls_trackbar32").as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP | TBS_HORZ | TBS_NOTICKS,
        x,
        y,
        w,
        SLIDER_H,
        app.hwnd,
        ptr::null_mut(),
        hinstance,
        ptr::null_mut(),
    );
    SendMessageW(
        hwnd,
        TBM_SETRANGE,
        1,
        winapi::shared::minwindef::MAKELONG(MIN_CELL as u16, MAX_CELL as u16) as LPARAM,
    );
    SendMessageW(hwnd, TBM_SETPAGESIZE, 0, 8);
    SendMessageW(hwnd, TBM_SETTICFREQ, 10, 0);
    hwnd
}

/// Create the control column. Called once, right after the window exists.
pub unsafe fn create(app: &mut App) {
    let hinstance = winapi::um::libloaderapi::GetModuleHandleW(ptr::null());
    let x = app.gfx.panel_x;
    let half = (PANEL_W - 6) / 2;
    let mut buttons = Vec::new();
    let mut labels = Vec::new();

    // --- actions ---------------------------------------------------------
    let mut y = HEADER + 14;
    buttons.push(make_button(app, ID_NEW, "New puzzle (N)", x, y, half, hinstance));
    buttons.push(make_button(
        app,
        ID_CLEAR,
        "Clear (R)",
        x + half + 6,
        y,
        half - 6,
        hinstance,
    ));
    y += BTN_H + 6;
    buttons.push(make_button(app, ID_CHECK, "Check (C)", x, y, half, hinstance));
    buttons.push(make_button(
        app,
        ID_SOLUTION,
        "Solution (S)",
        x + half + 6,
        y,
        half - 6,
        hinstance,
    ));
    y += BTN_H + 6;
    buttons.push(make_button(app, ID_UNDO, "Undo (Z)", x, y, half, hinstance));
    y += BTN_H + 16;

    labels.push(make_label(app, "Generation settings", x, y, PANEL_W, hinstance));

    // --- settings fields -------------------------------------------------
    let mut edits = Vec::with_capacity(FIELDS.len());
    for (index, (_, label, _)) in FIELDS.iter().enumerate() {
        let top = field_top(index);
        let label_hwnd = make_label(app, label, x, top + 3, LABEL_W, hinstance);
        labels.push(label_hwnd);
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP | ES_AUTOHSCROLL,
            x + LABEL_W + 8,
            top,
            EDIT_W,
            24,
            app.hwnd,
            (ID_EDIT_BASE + index as i32) as isize as winapi::shared::windef::HMENU,
            hinstance,
            ptr::null_mut(),
        );
        SendMessageW(edit, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);
        edits.push(edit);
    }
    // the cell-size slider sits in its own row, right under the board size
    let slider_label = make_label(app, "Cell size (px)", x, slider_top() + 3, LABEL_W, hinstance);
    let slider = make_slider(
        app,
        x + LABEL_W + 8,
        slider_top() + 2,
        PANEL_W - LABEL_W - 8,
        hinstance,
    );

    // --- one-step helper and apply row -----------------------------------
    let button_top = FIELD_TOP + FIELD_ROWS * ROW_H + 10;
    let step = make_button(
        app,
        ID_ONE_STEP,
        "One step (D)",
        x,
        button_top,
        half,
        hinstance,
    );
    buttons.push(step);
    buttons.push(make_button(
        app,
        ID_APPLY,
        "Apply & new puzzle",
        x + half + 6,
        button_top,
        half - 6,
        hinstance,
    ));

    let second_top = button_top + BTN_H + 6;
    buttons.push(make_button(
        app,
        ID_DEFAULTS,
        "Defaults",
        x,
        second_top,
        half,
        hinstance,
    ));

    let message = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        x,
        MESSAGE_TOP,
        PANEL_W,
        44,
        app.hwnd,
        ptr::null_mut(),
        hinstance,
        ptr::null_mut(),
    );
    SendMessageW(message, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);

    let help_default = format!("settings saved in {}", app.settings_path.display());
    let help = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide(&help_default).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        x,
        HELP_TOP,
        PANEL_W,
        56,
        app.hwnd,
        ptr::null_mut(),
        hinstance,
        ptr::null_mut(),
    );
    SendMessageW(help, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);

    app.controls = Some(Controls {
        edits,
        buttons,
        labels,
        message,
        help,
        slider,
        slider_label,
        help_default,
    });
    fill_values(app);
    set_text(message, &format!("saved in {}", app.settings_path.display()));
}

/// Show the help of whatever control the mouse is over, or the default text.
pub unsafe fn hover_help(app: &App, screen_x: i32, screen_y: i32) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    let over = |hwnd: HWND| -> bool {
        let mut rect: winapi::shared::windef::RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        screen_x >= rect.left && screen_x < rect.right && screen_y >= rect.top && screen_y < rect.bottom
    };

    for (index, (_, _, help)) in FIELDS.iter().enumerate() {
        let label = controls.labels.get(1 + index);
        let edit = controls.edits.get(index);
        if label.map_or(false, |h| over(*h)) || edit.map_or(false, |h| over(*h)) {
            set_text(controls.help, help);
            return;
        }
    }
    if over(controls.slider) || over(controls.slider_label) {
        set_text(controls.help, SLIDER_HELP);
        return;
    }
    set_text(controls.help, &controls.help_default);
}

/// Push the current settings into the edit boxes and the cell slider.
pub unsafe fn fill_values(app: &App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    for (index, (key, _, _)) in FIELDS.iter().enumerate() {
        if let Some(edit) = controls.edits.get(index) {
            set_text(*edit, &app.settings.value_of(key));
        }
    }
    sync_slider(app);
}

/// Put the zoom slider where the board actually is, e.g. after the window was
/// resized by dragging its frame.
pub unsafe fn sync_slider(app: &App) {
    if let Some(controls) = app.controls.as_ref() {
        SendMessageW(controls.slider, TBM_SETPOS, 1, app.gfx.cell as LPARAM);
    }
}

/// A WM_HSCROLL from the zoom slider. While dragging, only the help line
/// updates; on release the window is resized so one cell is that many pixels.
pub unsafe fn handle_hscroll(app: &mut App, code: i32, lparam: LPARAM) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    if lparam as HWND != controls.slider {
        return;
    }
    let cell = SendMessageW(controls.slider, TBM_GETPOS, 0, 0) as i32;
    set_text(
        controls.help,
        &format!("Cell size: {cell} px. Release to zoom the board to this size."),
    );
    if code as WPARAM == TB_ENDTRACK {
        app.set_cell_size(cell);
    }
}

/// Move every control after the layout changed (board size change).
pub unsafe fn relayout(app: &App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    let x = app.gfx.panel_x;
    let half = (PANEL_W - 6) / 2;
    let set = |hwnd: HWND, cx: i32, cy: i32, cw: i32, ch: i32| {
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            cx,
            cy,
            cw,
            ch,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    };

    let mut y = HEADER + 14;
    for (index, button) in controls.buttons.iter().take(5).enumerate() {
        let (row, col) = (index / 2, index % 2);
        let cy = HEADER + 14 + row as i32 * (BTN_H + 6);
        let (cx, cw) = if col == 1 {
            (x + half + 6, half - 6)
        } else if index == 4 {
            (x, half)
        } else {
            (x, half)
        };
        set(*button, cx, cy, cw, BTN_H);
    }
    y += 3 * (BTN_H + 6) + 10;

    if let Some(caption) = controls.labels.first() {
        set(*caption, x, y, PANEL_W, 20);
    }
    for (index, _) in FIELDS.iter().enumerate() {
        let top = field_top(index);
        if let Some(label) = controls.labels.get(1 + index) {
            set(*label, x, top + 3, LABEL_W, 20);
        }
        if let Some(edit) = controls.edits.get(index) {
            set(*edit, x + LABEL_W + 8, top, EDIT_W, 24);
        }
    }
    set(controls.slider_label, x, slider_top() + 3, LABEL_W, 20);
    set(
        controls.slider,
        x + LABEL_W + 8,
        slider_top() + 2,
        PANEL_W - LABEL_W - 8,
        SLIDER_H,
    );

    let button_top = FIELD_TOP + FIELD_ROWS * ROW_H + 10;
    if let Some(apply) = controls.buttons.get(5) {
        set(*apply, x, button_top, half + 30, BTN_H);
    }
    if let Some(defaults) = controls.buttons.get(6) {
        set(*defaults, x + half + 36, button_top, half - 36, BTN_H);
    }
    set(controls.message, x, MESSAGE_TOP, PANEL_W, 44);
    set(controls.help, x, HELP_TOP, PANEL_W, 56);
}

/// Handle a WM_COMMAND from the control column.
pub unsafe fn handle_command(app: &mut App, id: i32) {
    match id {
        ID_NEW => {
            app.new_puzzle(app.seed.wrapping_add(1));
            mirror_status(app);
        }
        ID_CLEAR => {
            app.reset();
            mirror_status(app);
        }
        ID_CHECK => {
            app.check();
            mirror_status(app);
        }
        ID_SOLUTION => {
            app.toggle_solution();
            mirror_status(app);
        }
        ID_UNDO => {
            app.undo();
            mirror_status(app);
        }
        ID_ONE_STEP => {
            app.one_step();
            mirror_status(app);
        }
        ID_APPLY => apply(app),
        ID_DEFAULTS => defaults(app),
        _ => {}
    }
}

/// Show the game's status line in the panel too, so every button press has
/// visible feedback right next to the button.
unsafe fn mirror_status(app: &App) {
    let status = app.status.clone();
    set_message(app, &status);
}

/// Write something into the panel's message line.
pub unsafe fn set_message(app: &App, text: &str) {
    if let Some(controls) = app.controls.as_ref() {
        set_text(controls.message, text);
    }
}

/// Read the edit boxes, save and apply the settings.
unsafe fn apply(app: &mut App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    let mut settings = app.settings.clone();
    for (index, (key, label, _)) in FIELDS.iter().enumerate() {
        let Some(edit) = controls.edits.get(index) else {
            continue;
        };
        let text = get_text(*edit);
        if let Err(err) = settings.set(key, &text) {
            set_text(controls.message, &format!("{label}: {}", err.0));
            return;
        }
    }

    match settings.save_file(&app.settings_path) {
        Ok(path) => app.settings_path = path,
        Err(err) => {
            set_text(controls.message, &format!("{}", err.0));
            return;
        }
    }

    let size_changed = settings.size != app.settings.size;
    app.settings = settings;
    if let Some(controls) = app.controls.as_ref() {
        set_text(
            controls.message,
            &format!("saved to {}", app.settings_path.display()),
        );
    }
    app.apply_settings(size_changed);
}

unsafe fn defaults(app: &mut App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    let defaults = Settings::default();
    for (index, (key, _, _)) in FIELDS.iter().enumerate() {
        if let Some(edit) = controls.edits.get(index) {
            set_text(*edit, &defaults.value_of(key));
        }
    }
    SendMessageW(controls.slider_label, WM_SETFONT, app.gfx.ui_font() as WPARAM, 1);
    set_text(controls.message, "defaults filled in - press Apply");
}

/// Colour a static/edit control for the panel background.
pub unsafe fn color_control(hdc: HDC) -> LRESULT {
    SetBkMode(hdc, 1); // TRANSPARENT
    SetBkColor(hdc, 0x00ff_ffff);
    GetStockObject(WHITE_BRUSH as i32) as HBRUSH as LRESULT
}

/// Re-apply the control fonts after the board was rebuilt.
pub unsafe fn refresh_fonts(app: &App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    for hwnd in controls
        .buttons
        .iter()
        .chain(controls.labels.iter())
        .chain(std::iter::once(&controls.slider_label))
    {
        SendMessageW(*hwnd, WM_SETFONT, app.gfx.ui_font() as WPARAM, 1);
    }
    for edit in controls.edits.iter() {
        SendMessageW(*edit, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);
    }
    SendMessageW(controls.message, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::PANEL_MIN_H;

    /// The window is never shorter than `PANEL_MIN_H`, so the whole column -
    /// including the last line of help text - has to fit inside it.
    #[test]
    fn the_control_column_fits_the_window() {
        assert!(
            HELP_TOP + 56 + 8 <= PANEL_MIN_H,
            "help line ends at {} but the window is only {PANEL_MIN_H} tall",
            HELP_TOP + 56
        );
    }

    #[test]
    fn the_slider_has_its_own_row_between_the_size_and_density_fields() {
        let size_row = field_top(0);
        let slider = slider_top();
        let density_row = field_top(1);
        assert!(slider >= size_row + 24, "the slider overlaps the size field");
        assert!(
            slider + SLIDER_H <= density_row,
            "the slider overlaps the density field"
        );
    }

    #[test]
    fn the_slider_covers_the_cell_sizes_the_layout_allows() {
        assert_eq!(MIN_CELL, 11);
        assert!(MAX_CELL > MIN_CELL);
    }
}










