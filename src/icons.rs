//! Glyph map for the bundled Lucide icon font. Every string is a private-use
//! codepoint, so it can live in normal labels and fall back through the font
//! families in `theme::install_fonts`.

#![allow(dead_code)]

pub const FOLDER_OPEN: &str = "\u{e247}";
pub const FILE_PLUS: &str = "\u{e0c9}";
pub const FILE_PLUS_2: &str = "\u{e0ca}";
pub const IMAGE_PLUS: &str = "\u{e1f7}";
pub const SAVE: &str = "\u{e14d}";
pub const UNDO: &str = "\u{e2a1}";
pub const REDO: &str = "\u{e2a0}";
pub const ROTATE_CCW: &str = "\u{e148}";
pub const ROTATE_CW: &str = "\u{e149}";
pub const CROP: &str = "\u{e0ab}";
pub const COPY_PLUS: &str = "\u{e3fd}";
pub const TRASH: &str = "\u{e18e}";
pub const FILE_OUTPUT: &str = "\u{e0c8}";
pub const SCISSORS: &str = "\u{e14e}";
pub const BOOK_OPEN: &str = "\u{e05f}";
pub const FILE_TEXT: &str = "\u{e0cc}";
pub const HELP: &str = "\u{e082}";
pub const CLOSE: &str = "\u{e1b2}";
pub const CHECK: &str = "\u{e06c}";
pub const GRIP: &str = "\u{e0eb}";
pub const CHEVRON_DOWN: &str = "\u{e06d}";
pub const CHEVRON_RIGHT: &str = "\u{e06f}";
pub const CHEVRON_LEFT: &str = "\u{e06e}";
pub const LAYOUT_GRID: &str = "\u{e0ff}";
pub const ARROW_LEFT: &str = "\u{e048}";
pub const ARROW_RIGHT: &str = "\u{e049}";
pub const SUN: &str = "\u{e178}";
pub const MOON: &str = "\u{e11e}";
pub const ZOOM_IN: &str = "\u{e1b6}";
pub const ZOOM_OUT: &str = "\u{e1b7}";
pub const FILES: &str = "\u{e0cf}";
pub const UPLOAD: &str = "\u{e19e}";
pub const PANEL_LEFT: &str = "\u{e12a}";
pub const PANEL_LEFT_CLOSE: &str = "\u{e21c}";
pub const PANEL_LEFT_OPEN: &str = "\u{e21d}";
pub const CIRCLE_CHECK: &str = "\u{e226}";
pub const ALERT: &str = "\u{e193}";
pub const INFO: &str = "\u{e0f9}";
pub const KEYBOARD: &str = "\u{e284}";
pub const PLUS: &str = "\u{e13d}";
pub const MINUS: &str = "\u{e11c}";
pub const LOCK: &str = "\u{e10b}";
pub const FILE_CHECK: &str = "\u{e0c2}";
pub const STACK: &str = "\u{e4a1}";
pub const MOVE: &str = "\u{e121}";
pub const MOUSE_POINTER: &str = "\u{e11f}";
pub const COMMAND: &str = "\u{e09a}";

/// Icon + label as one string so egui lays it out as a single text run and the
/// icon font fallback keeps it aligned with the label.
pub fn with_icon(icon: &str, label: &str) -> String {
    format!("{icon}  {label}")
}
