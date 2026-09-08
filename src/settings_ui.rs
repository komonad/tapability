//! The always-visible control column on the right of the game window.
//!
//! Every action the game has (new puzzle, clear, check, solution, undo) is a
//! button here, and the generation settings are editable in place. The controls
//! are real Win32 children of the main window, so typing, tabbing and focus
//! behave normally.

use std::ptr;

use winapi::shared::minwindef::{HINSTANCE, LRESULT, WPARAM};
use winapi::shared::windef::{HBRUSH, HDC, HWND};
use winapi::um::wingdi::{GetStockObject, SetBkColor, SetBkMode, WHITE_BRUSH};
use winapi::um::winuser::*;

use crate::config::{Settings, FIELDS};
use crate::render::{HEADER, PANEL_W};
use crate::window::App;

pub const ID_APPLY: i32 = 1;
pub const ID_NEW: i32 = 3;
pub const ID_CLEAR: i32 = 4;
pub const ID_CHECK: i32 = 5;
pub const ID_SOLUTION: i32 = 6;
pub const ID_UNDO: i32 = 7;
pub const ID_DEFAULTS: i32 = 8;
const ID_EDIT_BASE: i32 = 100;

const ROW_H: i32 = 28;
const BTN_H: i32 = 30;
const LABEL_W: i32 = 132;
const EDIT_W: i32 = 150;
const FIELD_TOP: i32 = HEADER + 150;
const MESSAGE_TOP: i32 = FIELD_TOP + FIELDS.len() as i32 * ROW_H + 46;

pub struct Controls {
    pub edits: Vec<HWND>,
    pub buttons: Vec<HWND>,
    pub labels: Vec<HWND>,
    pub message: HWND,
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
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
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
        WS_CHILD | WS_VISIBLE,
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
    for (index, (_, label)) in FIELDS.iter().enumerate() {
        let top = FIELD_TOP + index as i32 * ROW_H;
        labels.push(make_label(app, label, x, top + 3, LABEL_W, hinstance));
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
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

    let button_top = FIELD_TOP + FIELDS.len() as i32 * ROW_H + 10;
    buttons.push(make_button(
        app,
        ID_APPLY,
        "Apply & new puzzle",
        x,
        button_top,
        half + 30,
        hinstance,
    ));
    buttons.push(make_button(
        app,
        ID_DEFAULTS,
        "Defaults",
        x + half + 36,
        button_top,
        half - 36,
        hinstance,
    ));

    let message = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_VISIBLE,
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

    app.controls = Some(Controls {
        edits,
        buttons,
        labels,
        message,
    });
    fill_values(app);
    set_text(message, &format!("saved in {}", app.settings_path.display()));
}

/// Push the current settings into the edit boxes.
pub unsafe fn fill_values(app: &App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    for (index, (key, _)) in FIELDS.iter().enumerate() {
        if let Some(edit) = controls.edits.get(index) {
            set_text(*edit, &app.settings.value_of(key));
        }
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
        let top = FIELD_TOP + index as i32 * ROW_H;
        if let Some(label) = controls.labels.get(1 + index) {
            set(*label, x, top + 3, LABEL_W, 20);
        }
        if let Some(edit) = controls.edits.get(index) {
            set(*edit, x + LABEL_W + 8, top, EDIT_W, 24);
        }
    }

    let button_top = FIELD_TOP + FIELDS.len() as i32 * ROW_H + 10;
    if let Some(apply) = controls.buttons.get(5) {
        set(*apply, x, button_top, half + 30, BTN_H);
    }
    if let Some(defaults) = controls.buttons.get(6) {
        set(*defaults, x + half + 36, button_top, half - 36, BTN_H);
    }
    set(controls.message, x, MESSAGE_TOP, PANEL_W, 44);
}

/// Handle a WM_COMMAND from the control column.
pub unsafe fn handle_command(app: &mut App, id: i32) {
    match id {
        ID_NEW => app.new_puzzle(app.seed.wrapping_add(1)),
        ID_CLEAR => app.reset(),
        ID_CHECK => app.check(),
        ID_SOLUTION => app.toggle_solution(),
        ID_UNDO => app.undo(),
        ID_APPLY => apply(app),
        ID_DEFAULTS => defaults(app),
        _ => {}
    }
}

/// Read the edit boxes, save and apply the settings.
unsafe fn apply(app: &mut App) {
    let Some(controls) = app.controls.as_ref() else {
        return;
    };
    let mut settings = app.settings.clone();
    for (index, (key, label)) in FIELDS.iter().enumerate() {
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
    for (index, (key, _)) in FIELDS.iter().enumerate() {
        if let Some(edit) = controls.edits.get(index) {
            set_text(*edit, &defaults.value_of(key));
        }
    }
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
    {
        SendMessageW(*hwnd, WM_SETFONT, app.gfx.ui_font() as WPARAM, 1);
    }
    for edit in controls.edits.iter() {
        SendMessageW(*edit, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);
    }
    SendMessageW(controls.message, WM_SETFONT, app.gfx.small_font() as WPARAM, 1);
}

