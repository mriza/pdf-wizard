#![allow(deprecated)]
use crate::arrange;
use crate::history::History;
use crate::icons;
use crate::model::{CropMargins, Metadata, OverlaySpec, PageItem, SourceId};
use crate::pdf_engine::{self, SourceDocument, A4_H, A4_W};
use crate::renderer::Renderer;
use crate::theme::{
    self, body, icon_font, label_font, medium, mono, semibold, serif_font, Tokens, RADIUS_CARD,
    RADIUS_CONTROL, RADIUS_MODAL, RADIUS_PAPER,
};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, LayerId, Label, Layout,
    Margin, Modifiers, Order, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, UiBuilder,
    Vec2,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

const THUMB_PX: u32 = 360;
const GRID_GAP: f32 = 18.0;
const DEFAULT_ZOOM: f32 = 172.0;
const MIN_ZOOM: f32 = 104.0;
const MAX_ZOOM: f32 = 340.0;
const HEADER_H: f32 = 58.0;
const STATUS_H: f32 = 30.0;
const CANVAS_BAR_H: f32 = 52.0;
const SIDEBAR_W: f32 = 272.0;
const CARD_HEADER_H: f32 = 38.0;
const CARD_FOOTER_H: f32 = 34.0;
const CARD_BTN_H: f32 = 36.0;
const TOUCH_MIN: f32 = 36.0;
const TRAY_W: f32 = 520.0;
const TRAY_H: f32 = 56.0;


fn page_word(n: usize) -> &'static str {
    if n == 1 {
        "page"
    } else {
        "pages"
    }
}

fn file_word(n: usize) -> &'static str {
    if n == 1 {
        "file"
    } else {
        "files"
    }
}

fn short_source(label: &str) -> &str {
    label.split(" - p.").next().unwrap_or(label)
}

fn truncate_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 5 {
        return text.to_string();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - tail..].iter());
    out
}

fn clean_stem(stem: &str) -> String {
    let mut out = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if (ch.is_whitespace() || ch == '.') && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn card_tags(item: &PageItem) -> String {
    let mut tags: Vec<String> = Vec::new();
    if item.rotation != 0 {
        tags.push(format!("R{}", item.rotation));
    }
    if item.crop != CropMargins::default() {
        tags.push("Crop".into());
    }
    if (item.scale - 1.0).abs() > f32::EPSILON {
        tags.push(format!("{:.0}%", item.scale * 100.0));
    }
    if !item.overlays.is_empty() {
        tags.push("Layered".into());
    }
    if tags.len() > 2 {
        format!("{} +{}", tags[..2].join(" · "), tags.len() - 2)
    } else {
        tags.join(" · ")
    }
}

fn card_header_text(index: usize, item: &PageItem, card_width: f32) -> String {
    let folio = format!("#{}", index + 1);
    if card_width < 130.0 {
        return folio;
    }
    let tags = card_tags(item);
    if tags.is_empty() {
        folio
    } else {
        format!("{folio} · {tags}")
    }
}

/// Number of thumbnail columns that fit in `avail_width` (at least 1).
pub fn grid_columns(avail_width: f32, thumb: f32, spacing: f32) -> usize {
    (((avail_width + spacing) / (thumb + spacing)).floor() as usize).max(1)
}

/// Inclusive, ascending range of indices between `anchor` and `clicked`.
pub fn apply_range_select(anchor: usize, clicked: usize) -> Vec<usize> {
    (anchor.min(clicked)..=anchor.max(clicked)).collect()
}

fn drop_is_noop(selection: &[usize], target: usize) -> bool {
    let Some(&first) = selection.first() else {
        return true;
    };
    let last = *selection.last().unwrap_or(&first);
    target >= first && target <= last + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

#[derive(Clone)]
enum Thumb {
    Loading,
    Failed,
    Ready {
        tex: egui::TextureHandle,
        w: f32,
        h: f32,
    },
}

#[derive(Clone)]
enum Action {
    Open,
    OpenReplace,
    ConfirmReplace,
    ImportImages,
    DropFiles(Vec<PathBuf>),
    Save,
    ExtractSingle,
    ExtractSeparate,
    SplitAll,
    Booklet,
    Metadata,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    PasteAfter,
    PasteOverlay,
    PasteUnderlay,
    Duplicate,
    Delete,
    InsertBlank,
    SelectAll,
    SelectNone,
    Invert,
    SelectOdd,
    SelectEven,
    SelectSame,
    RotateLeft,
    RotateRight,
    Rotate180,
    CropDialog,
    ApplyCrop,
    Reverse,
    SwapOddEven,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTheme,
    StepLeft,
    StepRight,
    ToggleSidebar,
    ShowShortcuts,
    ShowAbout,
}

enum BgResult {
    Loaded {
        sources: HashMap<SourceId, SourceDocument>,
        items: Vec<PageItem>,
        errors: Vec<String>,
        requested: usize,
        next_source: SourceId,
        next_uid: u64,
        replace: bool,
    },
    Message {
        text: String,
        ok: bool,
    },
}

pub struct PdfWizardApp {
    sources: HashMap<SourceId, SourceDocument>,
    next_source: SourceId,
    next_uid: u64,
    pages: Vec<PageItem>,
    history: History,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    clipboard: Vec<PageItem>,
    renderer: Renderer,
    thumbs: HashMap<(SourceId, u32), Thumb>,
    zoom: f32,
    dark: bool,
    show_sidebar: bool,
    show_meta: bool,
    show_crop: bool,
    show_shortcuts: bool,
    show_about: bool,
    show_replace: bool,
    pending_replace: Option<Vec<PathBuf>>,
    meta: Metadata,
    meta_edit: Metadata,
    crop_edit: [f32; 4],
    scale_edit: f32,
    status: String,
    status_kind: StatusKind,
    last_dir: Option<PathBuf>,
    window_title: String,
    dragging: bool,
    drop_target: Option<usize>,
    tray_count: usize,
    canvas_rect: Option<Rect>,
    scroll_to: Option<usize>,
    busy: Option<String>,
    bg_rx: Option<Receiver<BgResult>>,
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn is_image(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| ["png", "jpg", "jpeg", "webp"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn load_paths_into(
    paths: &[PathBuf],
    next_source: &mut SourceId,
    next_uid: &mut u64,
) -> (HashMap<SourceId, SourceDocument>, Vec<PageItem>, Vec<String>) {
    let mut new_sources = HashMap::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map_or("file".to_string(), |n| n.to_string_lossy().to_string());
        let id = *next_source;
        let result = std::fs::read(path).map_err(anyhow::Error::from).and_then(|bytes| {
            if is_pdf(path) {
                pdf_engine::load_pdf(id, &name, bytes)
            } else {
                pdf_engine::image_to_source(id, &name, &bytes)
            }
        });
        match result {
            Ok(src) => {
                *next_source += 1;
                for n in 1..=src.page_count as u32 {
                    let (w, h) = pdf_engine::page_size(&src, n);
                    *next_uid += 1;
                    items.push(PageItem::from_source(
                        *next_uid,
                        id,
                        n,
                        w,
                        h,
                        format!("{name} - p.{n}"),
                    ));
                }
                new_sources.insert(id, src);
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    (new_sources, items, errors)
}

// ------------------------------------------------------------------ controls

#[derive(Clone, Copy, PartialEq)]
enum BtnKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
    DangerFilled,
}

struct Btn<'a> {
    text: &'a str,
    kind: BtnKind,
    width: Option<f32>,
    height: f32,
    font: Option<FontId>,
    enabled: bool,
    selected: bool,
    tooltip: Option<&'a str>,
    a11y: Option<&'a str>,
    icon_only: bool,
}

impl<'a> Btn<'a> {
    fn new(text: &'a str, kind: BtnKind) -> Self {
        Self {
            text,
            kind,
            width: None,
            height: 38.0,
            font: None,
            enabled: true,
            selected: false,
            tooltip: None,
            a11y: None,
            icon_only: false,
        }
    }

    fn primary(text: &'a str) -> Self {
        Self::new(text, BtnKind::Primary)
    }

    fn secondary(text: &'a str) -> Self {
        Self::new(text, BtnKind::Secondary)
    }

    fn ghost(text: &'a str) -> Self {
        Self::new(text, BtnKind::Ghost)
    }

    fn danger(text: &'a str) -> Self {
        Self::new(text, BtnKind::Danger)
    }

    fn danger_filled(text: &'a str) -> Self {
        Self::new(text, BtnKind::DangerFilled)
    }

    fn icon(icon: &'a str) -> Self {
        Self {
            text: icon,
            kind: BtnKind::Ghost,
            width: Some(36.0),
            height: 36.0,
            font: Some(icon_font(16.0)),
            enabled: true,
            selected: false,
            tooltip: None,
            a11y: None,
            icon_only: true,
        }
    }

    fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    fn height(mut self, height: f32) -> Self {
        self.height = height.max(TOUCH_MIN);
        self
    }

    fn font(mut self, font: FontId) -> Self {
        self.font = Some(font);
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn tip(mut self, tooltip: &'a str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }


    fn show(self, ui: &mut egui::Ui, t: Tokens) -> Response {
        let font = self.font.clone().unwrap_or_else(|| medium(13.0));
        let text_w = ui
            .painter()
            .layout_no_wrap(self.text.to_owned(), font.clone(), t.ink)
            .size()
            .x;
        let width = self
            .width
            .unwrap_or_else(|| text_w + 26.0)
            .max(if self.icon_only { TOUCH_MIN } else { 44.0 });
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width, self.height), sense);
        let hover_t = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hover"), resp.hovered() && self.enabled, 0.12);
        let pressed = self.enabled && resp.is_pointer_button_down_on();

        let (bg, stroke, fg) = button_colors(&self, t, hover_t, pressed);
        let radius = CornerRadius::same(RADIUS_CONTROL);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, bg);
        if stroke.width > 0.0 {
            painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }

        let galley = painter.layout_no_wrap(self.text.to_owned(), font, fg);
        let text_pos = rect.center() - galley.size() * 0.5;
        painter
            .with_clip_rect(rect.shrink2(Vec2::new(6.0, 0.0)))
            .galley(text_pos, galley, fg);
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(1.5),
                CornerRadius::same(RADIUS_CONTROL + 1),
                Stroke::new(1.5, t.accent),
                StrokeKind::Middle,
            );
        }

        let a11y = self.a11y.unwrap_or(if self.icon_only {
            self.tooltip.unwrap_or("Icon")
        } else {
            self.text
        });
        let enabled = self.enabled;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, a11y));
        if let Some(tip) = self.tooltip {
            resp = resp.on_hover_text(tip);
        }
        resp
    }
}

fn button_colors(btn: &Btn<'_>, t: Tokens, hover_t: f32, pressed: bool) -> (Color32, Stroke, Color32) {
    if !btn.enabled {
        let fg = t.faint;
        return match btn.kind {
            BtnKind::Ghost | BtnKind::Danger => (Color32::TRANSPARENT, Stroke::NONE, fg),
            _ => (t.surface, Stroke::new(1.0, t.line), fg),
        };
    }
    if btn.selected {
        let bg = t.accent_soft;
        return (bg, Stroke::new(1.0, t.accent), t.accent);
    }
    match btn.kind {
        BtnKind::Primary => {
            let bg = if pressed {
                t.accent_active
            } else {
                t.accent.lerp_to_gamma(t.accent_hover, hover_t)
            };
            (bg, Stroke::new(1.0, bg), t.on_accent)
        }
        BtnKind::Secondary => {
            let bg = if pressed {
                t.surface_active
            } else {
                t.surface.lerp_to_gamma(t.surface_hover, hover_t)
            };
            let edge = t.line.lerp_to_gamma(t.line_strong, hover_t);
            (bg, Stroke::new(1.0, edge), t.ink)
        }
        BtnKind::Ghost => {
            let bg = if pressed || hover_t > 0.01 {
                Tokens::alpha(t.accent_soft, (255.0 * hover_t.max(if pressed { 1.0 } else { 0.0 })) as u8)
            } else {
                Color32::TRANSPARENT
            };
            (bg, Stroke::NONE, t.accent)
        }
        BtnKind::Danger => {
            let bg = if pressed || hover_t > 0.01 {
                Tokens::alpha(t.danger_soft, (255.0 * hover_t.max(if pressed { 1.0 } else { 0.0 })) as u8)
            } else {
                Color32::TRANSPARENT
            };
            (bg, Stroke::NONE, t.danger)
        }
        BtnKind::DangerFilled => {
            let bg = t.danger.lerp_to_gamma(t.ink, if pressed { 0.16 } else { 0.08 * hover_t });
            let fg = if t.dark {
                Color32::from_rgb(28, 22, 20)
            } else {
                Color32::WHITE
            };
            (bg, Stroke::new(1.0, bg), fg)
        }
    }
}

/// True when a custom control was clicked or activated with Enter/Space while focused.
fn activated(resp: &Response) -> bool {
    if resp.clicked() {
        return true;
    }
    if resp.has_focus()
        && resp.ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Space)
        })
    {
        return true;
    }
    false
}

fn section_label(ui: &mut egui::Ui, t: Tokens, text: &str) {
    ui.add(Label::new(
        RichText::new(text)
            .font(label_font())
            .color(t.muted)
            .extra_letter_spacing(0.8),
    ));
}

fn hairline(ui: &mut egui::Ui, t: Tokens) {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, t.line),
    );
}

fn vertical_rule(ui: &mut egui::Ui, t: Tokens, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, height), Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.center().x, rect.center().y - height * 0.4),
            egui::pos2(rect.center().x, rect.center().y + height * 0.4),
        ],
        Stroke::new(1.0, t.line),
    );
}

fn key_chip(ui: &mut egui::Ui, t: Tokens, label: &str) {
    let font = mono(10.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, t.muted);
    let size = Vec2::new(galley.size().x + 12.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), t.surface_hover);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, t.line),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        rect.center() - galley.size() * 0.5,
        galley,
        t.muted,
    );
}

fn logo_mark(ui: &mut egui::Ui, t: Tokens, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let fold = (rect.width() * 0.28).max(4.0);
    let page = vec![
        rect.left_top(),
        egui::pos2(rect.right() - fold, rect.top()),
        egui::pos2(rect.right(), rect.top() + fold),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    painter.add(egui::Shape::convex_polygon(
        page,
        t.surface,
        Stroke::new(1.0, t.line_strong),
    ));
    let fold_pts = vec![
        egui::pos2(rect.right() - fold, rect.top()),
        egui::pos2(rect.right(), rect.top() + fold),
        egui::pos2(rect.right() - fold, rect.top() + fold),
    ];
    painter.add(egui::Shape::convex_polygon(
        fold_pts,
        t.brass,
        Stroke::new(0.5, t.line_strong),
    ));
    let rule = Stroke::new(1.0, Tokens::alpha(t.accent, 150));
    for (y, w) in [(0.42, 0.58), (0.55, 0.66), (0.68, 0.44)] {
        let yy = rect.top() + rect.height() * y;
        painter.line_segment(
            [
                egui::pos2(rect.left() + rect.width() * 0.18, yy),
                egui::pos2(
                    rect.left() + rect.width() * (0.18 + w),
                    yy,
                ),
            ],
            rule,
        );
    }
}

fn rotated_points(center: Pos2, size: Vec2, angle: f32) -> Vec<Pos2> {
    let h = size * 0.5;
    let corners = [
        Vec2::new(-h.x, -h.y),
        Vec2::new(h.x, -h.y),
        Vec2::new(h.x, h.y),
        Vec2::new(-h.x, h.y),
    ];
    corners
        .iter()
        .map(|v| {
            let (s, c) = angle.sin_cos();
            center + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
        })
        .collect()
}

fn stacked_sheets(ui: &mut egui::Ui, t: Tokens, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let center = rect.center() + Vec2::new(4.0, 4.0);
    let sheet = Vec2::new(156.0, 198.0).min(size * 0.72);

    for (offset, angle, fill) in [
        (Vec2::new(34.0, -16.0), -0.075, t.surface_hover),
        (Vec2::new(16.0, -2.0), 0.045, t.surface),
    ] {
        let c = center + offset;
        painter.add(
            t.paper_shadow()
                .as_shape(Rect::from_center_size(c + Vec2::new(2.0, 6.0), sheet), 3),
        );
        painter.add(egui::Shape::convex_polygon(
            rotated_points(c, sheet, angle),
            fill,
            Stroke::new(1.0, t.line),
        ));
    }

    let front = Rect::from_center_size(center - Vec2::new(10.0, 8.0), sheet);
    painter.add(
        t.paper_shadow()
            .as_shape(front.translate(Vec2::new(0.0, 6.0)), 2),
    );
    painter.rect_filled(front, CornerRadius::same(RADIUS_PAPER), t.surface);
    painter.rect_stroke(
        front,
        CornerRadius::same(RADIUS_PAPER),
        Stroke::new(1.0, t.line_strong),
        StrokeKind::Inside,
    );

    let clipped = painter.with_clip_rect(front.intersect(rect));
    clipped.line_segment(
        [
            egui::pos2(front.left() + 8.0, front.top() + 24.0),
            egui::pos2(front.left() + 8.0, front.bottom() - 24.0),
        ],
        Stroke::new(1.5, Tokens::alpha(t.accent, 110)),
    );

    let text_color = t.line_strong;
    for (y, w) in [(0.27, 0.62), (0.35, 0.72), (0.43, 0.55)] {
        clipped.line_segment(
            [
                egui::pos2(front.left() + 22.0, front.top() + front.height() * y),
                egui::pos2(
                    front.left() + 22.0 + (front.width() - 42.0) * w,
                    front.top() + front.height() * y,
                ),
            ],
            Stroke::new(2.0, text_color),
        );
    }

    // A slim brass bookmark, the only ornament.
    let bookmark = Rect::from_min_size(
        egui::pos2(front.right() - 31.0, front.top() + 20.0),
        Vec2::new(7.0, 50.0),
    );
    clipped.rect_filled(bookmark, CornerRadius::same(1), t.brass);
    clipped.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(bookmark.left(), bookmark.bottom()),
            egui::pos2(bookmark.center().x, bookmark.bottom() - 6.0),
            egui::pos2(bookmark.right(), bookmark.bottom()),
        ],
        t.surface,
        Stroke::NONE,
    ));

    clipped.text(
        egui::pos2(front.center().x, front.bottom() - 17.0),
        Align2::CENTER_CENTER,
        "#01",
        mono(10.5),
        t.muted,
    );

    // Trim marks around the top sheet.
    let trim = Stroke::new(1.0, t.line_strong);
    for (corner, dx, dy) in [
        (front.left_top(), -1.0, 0.0),
        (front.right_top(), 1.0, 0.0),
        (front.left_bottom(), -1.0, 0.0),
        (front.right_bottom(), 1.0, 0.0),
    ] {
        let x = if dx < 0.0 { corner.x - 10.0 } else { corner.x + 10.0 };
        let y = if dy < 0.0 { corner.y - 10.0 } else { corner.y + 10.0 };
        painter.line_segment(
            [egui::pos2(x, corner.y), egui::pos2(x + dx * 5.0, corner.y)],
            trim,
        );
        painter.line_segment(
            [egui::pos2(corner.x, y), egui::pos2(corner.x, y + 5.0)],
            trim,
        );
    }
}

fn dotted_bench(painter: &egui::Painter, rect: Rect, t: Tokens) {
    let color = if t.dark {
        Tokens::alpha(t.line_strong, 70)
    } else {
        Tokens::alpha(t.line_strong, 82)
    };
    let step = 22.0;
    let mut y = rect.top() + 12.0;
    while y < rect.bottom() {
        let mut x = rect.left() + 12.0;
        while x < rect.right() {
            painter.circle_filled(egui::pos2(x, y), 0.8, color);
            x += step;
        }
        y += step;
    }
}

fn modal_frame(t: Tokens) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(CornerRadius::same(RADIUS_MODAL))
        .inner_margin(Margin::symmetric(24, 22))
        .shadow(t.modal_shadow())
}

fn modal_header(ui: &mut egui::Ui, t: Tokens, title: &str, subtitle: &str, close: &mut bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .font(serif_font(25.0))
                    .color(t.ink),
            );
            if !subtitle.is_empty() {
                ui.add_space(3.0);
                ui.add(
                    Label::new(
                        RichText::new(subtitle)
                            .font(body(12.0))
                            .color(t.muted)
                            .line_height(Some(17.0)),
                    )
                    .wrap(),
                );
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if activated(
                &Btn::icon(icons::CLOSE)
                    .tip("Close")
                    .show(ui, t),
            ) {
                *close = true;
            }
        });
    });
}

fn menu_item(
    ui: &mut egui::Ui,
    a: &mut Vec<Action>,
    label: &str,
    shortcut: &str,
    enabled: bool,
    act: Action,
) {
    let mut button = egui::Button::new(RichText::new(label).font(body(13.0)))
        .min_size(Vec2::new(214.0, 30.0));
    if !shortcut.is_empty() {
        button = button.shortcut_text(RichText::new(shortcut).font(mono(10.5)));
    }
    if ui.add_enabled(enabled, button).clicked() {
        a.push(act);
        ui.close();
    }
}

fn menu_check_item(
    ui: &mut egui::Ui,
    a: &mut Vec<Action>,
    label: &str,
    shortcut: &str,
    checked: bool,
    act: Action,
) {
    let text = if checked {
        format!("{}  {label}", icons::CHECK)
    } else {
        format!("   {label}")
    };
    let mut button = egui::Button::new(RichText::new(text).font(body(13.0)))
        .min_size(Vec2::new(214.0, 30.0));
    if !shortcut.is_empty() {
        button = button.shortcut_text(RichText::new(shortcut).font(mono(10.5)));
    }
    if ui.add(button).clicked() {
        a.push(act);
        ui.close();
    }
}

impl PdfWizardApp {
    fn empty() -> Self {
        PdfWizardApp {
            sources: HashMap::new(),
            next_source: 0,
            next_uid: 0,
            pages: Vec::new(),
            history: History::new(Vec::new()),
            selected: BTreeSet::new(),
            anchor: None,
            clipboard: Vec::new(),
            renderer: Renderer::new(),
            thumbs: HashMap::new(),
            zoom: DEFAULT_ZOOM,
            dark: false,
            show_sidebar: true,
            show_meta: false,
            show_crop: false,
            show_shortcuts: false,
            show_about: false,
            show_replace: false,
            pending_replace: None,
            meta: Metadata::default(),
            meta_edit: Metadata::default(),
            crop_edit: [0.0; 4],
            scale_edit: 100.0,
            status: "Ready.".into(),
            status_kind: StatusKind::Info,
            last_dir: None,
            window_title: String::new(),
            dragging: false,
            drop_target: None,
            tray_count: 0,
            canvas_rect: None,
            scroll_to: None,
            busy: None,
            bg_rx: None,
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::empty();
        theme::apply(&cc.egui_ctx, app.dark);
        let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
        if !args.is_empty() {
            app.remember_file(&args[0]);
            let mut next_source = app.next_source;
            let mut next_uid = app.next_uid;
            let (sources, items, errors) = load_paths_into(&args, &mut next_source, &mut next_uid);
            app.next_source = next_source;
            app.next_uid = next_uid;
            app.apply_loaded(sources, items, errors, args.len(), true);
        }
        app
    }

    fn uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    fn set_status(&mut self, msg: impl Into<String>, kind: StatusKind) {
        self.status = msg.into();
        self.status_kind = kind;
    }

    fn commit(&mut self, pages: Vec<PageItem>) {
        self.pages = pages;
        self.history.push(self.pages.clone());
        self.selected.retain(|&i| i < self.pages.len());
        self.anchor = self.anchor.filter(|&a| a < self.pages.len());
    }

    fn sel_vec(&self) -> Vec<usize> {
        self.selected.iter().copied().collect()
    }

    fn selected_items(&self) -> Vec<PageItem> {
        self.selected
            .iter()
            .filter_map(|&i| self.pages.get(i).cloned())
            .collect()
    }

    fn used_sources(&self) -> BTreeSet<SourceId> {
        let mut used = BTreeSet::new();
        for page in &self.pages {
            if let Some(id) = page.source_id {
                used.insert(id);
            }
            for ov in &page.overlays {
                used.insert(ov.source_id);
            }
        }
        used
    }

    fn first_source(&self) -> Option<&SourceDocument> {
        let sid = self.pages.iter().find_map(|p| p.source_id)?;
        self.sources.get(&sid)
    }

    fn document_name(&self) -> String {
        if self.pages.is_empty() {
            return "No document".into();
        }
        if let Some(src) = self.first_source() {
            let used = self.used_sources().len();
            if used > 1 {
                return format!("{} +{}", src.file_name, used - 1);
            }
            src.file_name.clone()
        } else {
            "Blank pages".into()
        }
    }

    fn document_meta(&self) -> String {
        let pages = self.pages.len();
        if pages == 0 {
            return "No pages yet".into();
        }
        let files = self.used_sources().len();
        if files == 0 {
            format!("{} {} · blank", pages, page_word(pages))
        } else {
            format!("{} {} · {} {}", pages, page_word(pages), files, file_word(files))
        }
    }

    fn suggested_name(&self, suffix: &str) -> String {
        let stem = self
            .first_source()
            .and_then(|s| Path::new(&s.file_name).file_stem().map(|x| x.to_string_lossy().to_string()))
            .or_else(|| {
                self.sources
                    .values()
                    .next()
                    .and_then(|s| Path::new(&s.file_name).file_stem().map(|x| x.to_string_lossy().to_string()))
            })
            .map(|s| clean_stem(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "document".into());
        format!("{stem}-{suffix}.pdf")
    }

    fn remember_file(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                self.last_dir = Some(parent.to_path_buf());
            }
        }
    }

    fn remember_folder(&mut self, dir: &Path) {
        self.last_dir = Some(dir.to_path_buf());
    }

    fn pick_files(&mut self, title: &str, filter: &str, exts: &[&str]) -> Option<Vec<PathBuf>> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter, exts);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.pick_files();
        if let Some(first) = picked.as_ref().and_then(|p| p.first()) {
            self.remember_file(first);
        }
        picked
    }

    fn pick_folder(&mut self, title: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.pick_folder();
        if let Some(dir) = &picked {
            self.remember_folder(dir);
        }
        picked
    }

    fn save_location(&mut self, default_name: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save PDF")
            .add_filter("PDF", &["pdf"])
            .set_file_name(default_name);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.save_file();
        if let Some(path) = &picked {
            self.remember_file(path);
        }
        picked
    }

    fn select_only(&mut self, i: usize) {
        self.selected.clear();
        if i < self.pages.len() {
            self.selected.insert(i);
            self.anchor = Some(i);
        } else {
            self.anchor = None;
        }
    }

    fn toggle_select(&mut self, i: usize) {
        if i >= self.pages.len() {
            return;
        }
        if !self.selected.remove(&i) {
            self.selected.insert(i);
        }
        self.anchor = Some(i);
    }

    fn handle_card_click(&mut self, i: usize, shift: bool, command: bool) {
        if i >= self.pages.len() {
            return;
        }
        match (shift, command, self.anchor) {
            (true, _, Some(anchor)) => {
                let max_idx = self.pages.len().saturating_sub(1);
                self.selected = apply_range_select(anchor.min(max_idx), i)
                    .into_iter()
                    .filter(|&idx| idx < self.pages.len())
                    .collect();
            }
            (_, true, _) => self.toggle_select(i),
            _ => self.select_only(i),
        }
    }

    fn ensure_thumb(&mut self, sid: SourceId, page: u32) -> Thumb {
        if let Some(t) = self.thumbs.get(&(sid, page)) {
            return t.clone();
        }
        if let Some(src) = self.sources.get(&sid) {
            self.renderer
                .request(sid, src.raw_bytes.clone(), page, THUMB_PX);
            self.thumbs.insert((sid, page), Thumb::Loading);
        } else {
            self.thumbs.insert((sid, page), Thumb::Failed);
        }
        Thumb::Loading
    }

    fn poll_thumbs(&mut self, ctx: &egui::Context) {
        for r in self.renderer.poll() {
            let thumb = match r.image {
                Some(img) => {
                    let ci =
                        egui::ColorImage::from_rgba_premultiplied([img.width, img.height], &img.rgba);
                    let tex = ctx.load_texture(
                        format!("thumb-{}-{}", r.source_id, r.page_num),
                        ci,
                        egui::TextureOptions::LINEAR,
                    );
                    Thumb::Ready {
                        tex,
                        w: img.width as f32,
                        h: img.height as f32,
                    }
                }
                None => Thumb::Failed,
            };
            self.thumbs.insert((r.source_id, r.page_num), thumb);
        }
        if self
            .thumbs
            .values()
            .any(|t| matches!(t, Thumb::Loading))
        {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn spawn_bg(&mut self, label: &str, job: impl FnOnce() -> BgResult + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.busy = Some(label.to_string());
        self.bg_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    fn start_load_bg(&mut self, paths: Vec<PathBuf>, replace: bool) {
        let mut next_source = self.next_source;
        let mut next_uid = self.next_uid;
        let requested = paths.len();
        let label = if paths.len() == 1 {
            let name = paths[0]
                .file_name()
                .map_or("file".to_string(), |n| n.to_string_lossy().to_string());
            format!("Reading {name}...")
        } else {
            format!("Reading {} files...", paths.len())
        };
        self.spawn_bg(&label, move || {
            let (sources, items, errors) =
                load_paths_into(&paths, &mut next_source, &mut next_uid);
            BgResult::Loaded {
                sources,
                items,
                errors,
                requested,
                next_source,
                next_uid,
                replace,
            }
        });
    }

    fn poll_bg(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.bg_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(res) => {
                self.busy = None;
                self.bg_rx = None;
                match res {
                    BgResult::Loaded {
                        sources,
                        items,
                        errors,
                        requested,
                        next_source,
                        next_uid,
                        replace,
                    } => {
                        self.next_source = self.next_source.max(next_source);
                        self.next_uid = self.next_uid.max(next_uid);
                        self.apply_loaded(sources, items, errors, requested, replace);
                    }
                    BgResult::Message { text, ok } => {
                        let kind = if ok {
                            StatusKind::Success
                        } else {
                            StatusKind::Error
                        };
                        self.set_status(text, kind);
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Err(TryRecvError::Disconnected) => {
                self.busy = None;
                self.bg_rx = None;
            }
        }
    }

    fn apply_loaded(
        &mut self,
        new_sources: HashMap<SourceId, SourceDocument>,
        loaded: Vec<PageItem>,
        errors: Vec<String>,
        requested: usize,
        replace: bool,
    ) {
        if loaded.is_empty() {
            let detail = if errors.is_empty() {
                String::new()
            } else {
                format!(" {}", errors.join(" "))
            };
            let what = if requested == 1 {
                "Could not open the file.".to_string()
            } else {
                format!("Could not open any of the {requested} selected files.")
            };
            self.set_status(format!("{what}{detail}"), StatusKind::Error);
            return;
        }

        let loaded_files = new_sources.len();
        let first_name = new_sources
            .values()
            .min_by_key(|s| s.id)
            .map(|s| s.file_name.clone());
        let n = loaded.len();
        let files = if loaded_files == 1 {
            first_name.unwrap_or_else(|| "one file".into())
        } else {
            format!("{loaded_files} files")
        };
        self.sources.extend(new_sources);

        if replace {
            self.pages = loaded;
            self.history = History::new(self.pages.clone());
            self.selected.clear();
            self.anchor = None;
            self.clipboard.clear();
            let keep: HashSet<SourceId> = self.pages.iter().filter_map(|p| p.source_id).collect();
            self.sources.retain(|id, _| keep.contains(id));
            self.thumbs.retain(|(id, _), _| keep.contains(id));
            self.renderer.retain_sources(keep);
            self.scroll_to = Some(0);
            let msg = format!("Opened {n} {} from {files}.", page_word(n));
            if errors.is_empty() {
                self.set_status(msg, StatusKind::Success);
            } else {
                self.set_status(
                    format!(
                        "{msg} {} {} could not be opened: {}",
                        errors.len(),
                        file_word(errors.len()),
                        errors.join(" ")
                    ),
                    StatusKind::Error,
                );
            }
        } else {
            let at = self.pages.len();
            self.scroll_to = Some(at);
            let mut pages = self.pages.clone();
            pages.extend(loaded);
            self.commit(pages);
            let msg = format!("Added {n} {} from {files}.", page_word(n));
            if errors.is_empty() {
                self.set_status(msg, StatusKind::Success);
            } else {
                self.set_status(
                    format!(
                        "{msg} {} {} could not be opened: {}",
                        errors.len(),
                        file_word(errors.len()),
                        errors.join(" ")
                    ),
                    StatusKind::Error,
                );
            }
        }
    }

    fn restore(&mut self, pages: Vec<PageItem>, msg: &str) {
        self.pages = pages;
        self.selected.clear();
        self.anchor = None;
        self.set_status(msg, StatusKind::Info);
    }

    fn delete_selected(&mut self) {
        let n = self.selected.len();
        let pages: Vec<PageItem> = self
            .pages
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.selected.contains(i))
            .map(|(_, p)| p.clone())
            .collect();
        self.selected.clear();
        self.anchor = None;
        self.commit(pages);
        self.set_status(
            format!("Removed {n} {}.", page_word(n)),
            StatusKind::Success,
        );
    }

    fn rotate_selected(&mut self, delta: i32) {
        if self.selected.is_empty() {
            self.set_status("Select pages to rotate.", StatusKind::Info);
            return;
        }
        let n = self.selected.len();
        let mut pages = self.pages.clone();
        for &i in &self.selected {
            if let Some(p) = pages.get_mut(i) {
                p.rotate(delta);
            }
        }
        self.commit(pages);
        let turn = match delta.rem_euclid(360) {
            90 => "clockwise",
            180 => "180°",
            270 => "counterclockwise",
            _ => "",
        };
        self.set_status(
            format!("Rotated {n} {} {turn}.", page_word(n)),
            StatusKind::Success,
        );
    }

    fn apply_crop(&mut self) {
        if self.selected.is_empty() {
            self.set_status("Select pages to crop or scale.", StatusKind::Info);
            return;
        }
        let [l, r, t, b] = self.crop_edit.map(|v| (v / 100.0).clamp(0.0, 0.49));
        let crop = CropMargins {
            left: l,
            right: r,
            top: t,
            bottom: b,
        };
        let n = self.selected.len();
        let mut pages = self.pages.clone();
        for &i in &self.selected {
            if let Some(p) = pages.get_mut(i) {
                p.crop = crop;
                p.scale = (self.scale_edit / 100.0).clamp(0.1, 4.0);
            }
        }
        self.commit(pages);
        self.set_status(
            format!("Crop and scale applied to {n} {}.", page_word(n)),
            StatusKind::Success,
        );
    }

    fn move_selected_to(&mut self, target: usize) {
        let sel = self.sel_vec();
        let mut pages = self.pages.clone();
        arrange::move_pages(&mut pages, &sel, target);
        let unchanged = pages
            .iter()
            .map(|p| p.uid)
            .eq(self.pages.iter().map(|p| p.uid));
        if unchanged {
            return;
        }
        let at = target - sel.iter().filter(|&&i| i < target).count();
        self.selected = (at..at + sel.len()).collect();
        self.anchor = self.selected.iter().next().copied();
        self.pages = pages;
        self.history.push(self.pages.clone());
        self.set_status(
            format!("Moved {} {}.", sel.len(), page_word(sel.len())),
            StatusKind::Success,
        );
    }

    fn step_move_selected(&mut self, delta: isize) {
        if self.selected.is_empty() {
            self.set_status("Select pages to move.", StatusKind::Info);
            return;
        }
        let sel = self.sel_vec();
        let mut pages = self.pages.clone();
        let new_sel = arrange::step_move_pages(&mut pages, &sel, delta);
        let unchanged = pages
            .iter()
            .map(|p| p.uid)
            .eq(self.pages.iter().map(|p| p.uid));
        if unchanged {
            return;
        }
        self.selected = new_sel.into_iter().collect();
        self.anchor = self.selected.iter().next().copied();
        self.pages = pages;
        self.history.push(self.pages.clone());
        self.set_status(
            if delta < 0 {
                "Moved selection left."
            } else {
                "Moved selection right."
            },
            StatusKind::Success,
        );
    }

    fn save_pdf_dialog(&mut self, pages: Vec<PageItem>, default_name: String, booklet: bool) {
        if self.busy.is_some() {
            self.set_status("A task is still running. Try again when it finishes.", StatusKind::Info);
            return;
        }
        let Some(path) = self.save_location(&default_name) else {
            return;
        };
        let sources = self.sources.clone();
        let meta = self.meta.clone();
        let label = if booklet {
            "Writing booklet...".to_string()
        } else {
            "Writing PDF...".to_string()
        };
        self.spawn_bg(&label, move || {
            let result = if booklet {
                pdf_engine::export_booklet(&pages, &sources, &meta)
            } else {
                pdf_engine::export_pages(&pages, &sources, &meta)
            };
            match result.and_then(|b| std::fs::write(&path, b).map_err(Into::into)) {
                Ok(()) => BgResult::Message {
                    text: format!("Saved {}", path.display()),
                    ok: true,
                },
                Err(e) => BgResult::Message {
                    text: format!("Could not save the PDF: {e}"),
                    ok: false,
                },
            }
        });
    }

    fn extract_to_folder(&mut self, pages: Vec<PageItem>) {
        if self.busy.is_some() {
            self.set_status("A task is still running. Try again when it finishes.", StatusKind::Info);
            return;
        }
        let Some(dir) = self.pick_folder("Choose a folder for the PDF files") else {
            return;
        };
        let sources = self.sources.clone();
        let meta = self.meta.clone();
        self.spawn_bg("Extracting pages...", move || {
            match pdf_engine::extract_separate(&pages, &sources, &dir, &meta) {
                Ok(files) => BgResult::Message {
                    text: format!("Wrote {} {} to {}", files.len(), file_word(files.len()), dir.display()),
                    ok: true,
                },
                Err(e) => BgResult::Message {
                    text: format!("Could not extract the pages: {e}"),
                    ok: false,
                },
            }
        });
    }

    fn modal_open(&self) -> bool {
        self.show_meta
            || self.show_crop
            || self.show_shortcuts
            || self.show_about
            || self.show_replace
    }

    // ---------------------------------------------------------------- input

    fn handle_input(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.modal_open() {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            actions.push(Action::DropFiles(dropped));
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        ctx.input_mut(|i| {
            let cmd = Modifiers::COMMAND;
            if i.consume_key(cmd | Modifiers::SHIFT, Key::Z) || i.consume_key(cmd, Key::Y) {
                actions.push(Action::Redo);
            } else if i.consume_key(cmd, Key::Z) {
                actions.push(Action::Undo);
            }
            if i.consume_key(cmd | Modifiers::SHIFT, Key::O) {
                actions.push(Action::OpenReplace);
            }
            if i.consume_key(cmd | Modifiers::SHIFT, Key::L) {
                actions.push(Action::ToggleTheme);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowLeft) {
                actions.push(Action::StepLeft);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowRight) {
                actions.push(Action::StepRight);
            }
            let table = [
                (cmd, Key::A, Action::SelectAll),
                (cmd, Key::C, Action::Copy),
                (cmd, Key::X, Action::Cut),
                (cmd, Key::V, Action::PasteAfter),
                (cmd, Key::D, Action::Duplicate),
                (cmd, Key::O, Action::Open),
                (cmd, Key::S, Action::Save),
                (cmd, Key::B, Action::ToggleSidebar),
                (cmd, Key::Q, Action::Quit),
                (Modifiers::NONE, Key::Delete, Action::Delete),
                (Modifiers::NONE, Key::Escape, Action::SelectNone),
                (Modifiers::NONE, Key::F1, Action::ShowShortcuts),
                (cmd, Key::Equals, Action::ZoomIn),
                (cmd, Key::Plus, Action::ZoomIn),
                (cmd, Key::Minus, Action::ZoomOut),
                (cmd, Key::Num0, Action::ZoomReset),
            ];
            for (m, k, a) in table {
                if i.consume_key(m, k) {
                    actions.push(a);
                }
            }
            for e in &i.events {
                match e {
                    egui::Event::Copy if !actions.iter().any(|a| matches!(a, Action::Copy)) => {
                        actions.push(Action::Copy);
                    }
                    egui::Event::Cut if !actions.iter().any(|a| matches!(a, Action::Cut)) => {
                        actions.push(Action::Cut);
                    }
                    egui::Event::Paste(_)
                        if !self.clipboard.is_empty()
                            && !actions.iter().any(|a| matches!(a, Action::PasteAfter)) =>
                    {
                        actions.push(Action::PasteAfter);
                    }
                    _ => {}
                }
            }
        });
    }

    // --------------------------------------------------------------- actions

    fn run(&mut self, action: Action, ctx: &egui::Context) {
        let file_action = matches!(
            &action,
            Action::Open
                | Action::OpenReplace
                | Action::ImportImages
                | Action::DropFiles(_)
                | Action::Save
                | Action::ExtractSingle
                | Action::ExtractSeparate
                | Action::SplitAll
                | Action::Booklet
        );
        if file_action && self.busy.is_some() {
            self.set_status("A task is still running. Try again when it finishes.", StatusKind::Info);
            return;
        }
        let has_sel = !self.selected.is_empty();
        match action {
            Action::Open => {
                if let Some(paths) = self.pick_files(
                    "Add pages",
                    "PDF and images",
                    &["pdf", "png", "jpg", "jpeg", "webp"],
                ) {
                    self.start_load_bg(paths, false);
                }
            }
            Action::OpenReplace => {
                let Some(paths) = self.pick_files(
                    "Open and replace",
                    "PDF and images",
                    &["pdf", "png", "jpg", "jpeg", "webp"],
                ) else {
                    return;
                };
                if self.pages.is_empty() {
                    self.start_load_bg(paths, true);
                } else {
                    self.pending_replace = Some(paths);
                    self.show_replace = true;
                    self.show_meta = false;
                    self.show_crop = false;
                    self.show_shortcuts = false;
                    self.show_about = false;
                }
            }
            Action::ConfirmReplace => {
                if let Some(paths) = self.pending_replace.take() {
                    self.show_replace = false;
                    self.start_load_bg(paths, true);
                }
            }
            Action::ImportImages => {
                if let Some(paths) =
                    self.pick_files("Import images", "Images", &["png", "jpg", "jpeg", "webp"])
                {
                    self.start_load_bg(paths, false);
                }
            }
            Action::DropFiles(paths) => {
                let ok: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| is_pdf(p) || is_image(p))
                    .collect();
                if ok.is_empty() {
                    self.set_status(
                        "Those files are not PDFs or images.",
                        StatusKind::Error,
                    );
                } else {
                    self.start_load_bg(ok, false);
                }
            }
            Action::Save => {
                if self.pages.is_empty() {
                    self.set_status("There are no pages to save.", StatusKind::Info);
                } else {
                    let pages = self.pages.clone();
                    let name = self.suggested_name("arranged");
                    self.save_pdf_dialog(pages, name, false);
                }
            }
            Action::ExtractSingle => {
                if !has_sel {
                    self.set_status("Select pages to extract.", StatusKind::Info);
                } else {
                    let pages = self.selected_items();
                    let name = self.suggested_name("selected");
                    self.save_pdf_dialog(pages, name, false);
                }
            }
            Action::ExtractSeparate => {
                if !has_sel {
                    self.set_status("Select pages to extract.", StatusKind::Info);
                } else {
                    let pages = self.selected_items();
                    self.extract_to_folder(pages);
                }
            }
            Action::SplitAll => {
                if self.pages.is_empty() {
                    self.set_status("There are no pages to split.", StatusKind::Info);
                } else {
                    let pages = self.pages.clone();
                    self.extract_to_folder(pages);
                }
            }
            Action::Booklet => {
                let pages = if has_sel {
                    self.selected_items()
                } else {
                    self.pages.clone()
                };
                if pages.is_empty() {
                    self.set_status("There are no pages to export.", StatusKind::Info);
                } else {
                    let name = self.suggested_name("booklet");
                    self.save_pdf_dialog(pages, name, true);
                }
            }
            Action::Metadata => {
                if self.pages.is_empty() {
                    self.set_status(
                        "Add pages before editing document details.",
                        StatusKind::Info,
                    );
                } else {
                    self.meta_edit = self.meta.clone();
                    self.show_meta = true;
                    self.show_crop = false;
                    self.show_shortcuts = false;
                    self.show_about = false;
                    self.show_replace = false;
                }
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => {
                if let Some(pages) = self.history.undo() {
                    self.restore(pages, "Undid the last change.");
                } else {
                    self.set_status("Nothing to undo.", StatusKind::Info);
                }
            }
            Action::Redo => {
                if let Some(pages) = self.history.redo() {
                    self.restore(pages, "Redid the last change.");
                } else {
                    self.set_status("Nothing to redo.", StatusKind::Info);
                }
            }
            Action::Copy => {
                if has_sel {
                    self.clipboard = self.selected_items();
                    ctx.copy_text("pdf-wizard-pages".into());
                    let n = self.clipboard.len();
                    self.set_status(
                        format!("Copied {n} {}.", page_word(n)),
                        StatusKind::Success,
                    );
                } else {
                    self.set_status("Select pages to copy.", StatusKind::Info);
                }
            }
            Action::Cut => {
                if has_sel {
                    self.clipboard = self.selected_items();
                    ctx.copy_text("pdf-wizard-pages".into());
                    let n = self.clipboard.len();
                    self.delete_selected();
                    self.set_status(
                        format!("Cut {n} {}.", page_word(n)),
                        StatusKind::Success,
                    );
                } else {
                    self.set_status("Select pages to cut.", StatusKind::Info);
                }
            }
            Action::PasteAfter => {
                if self.clipboard.is_empty() {
                    self.set_status("There are no copied pages.", StatusKind::Info);
                } else {
                    let at = self
                        .selected
                        .iter()
                        .next_back()
                        .map_or(self.pages.len(), |&i| i + 1)
                        .min(self.pages.len());
                    let clones: Vec<PageItem> = self
                        .clipboard
                        .clone()
                        .into_iter()
                        .map(|p| self.renumber(p))
                        .collect();
                    let mut pages = self.pages.clone();
                    let n = clones.len();
                    for (k, p) in clones.into_iter().enumerate() {
                        pages.insert(at + k, p);
                    }
                    self.commit(pages);
                    self.selected = (at..at + n).collect();
                    self.anchor = Some(at);
                    self.scroll_to = Some(at);
                    self.set_status(
                        format!("Pasted {n} {}.", page_word(n)),
                        StatusKind::Success,
                    );
                }
            }
            Action::PasteOverlay | Action::PasteUnderlay => {
                let under = matches!(action, Action::PasteUnderlay);
                let src = self
                    .clipboard
                    .first()
                    .and_then(|p| p.source_id.map(|s| (s, p.source_page_num)));
                match (src, has_sel) {
                    (Some((sid, num)), true) => {
                        let n = self.selected.len();
                        let mut pages = self.pages.clone();
                        for &i in &self.selected {
                            if let Some(p) = pages.get_mut(i) {
                                p.overlays.push(OverlaySpec {
                                    source_id: sid,
                                    page_number: num,
                                    is_underlay: under,
                                });
                            }
                        }
                        self.commit(pages);
                        let layer = if under { "Underlay" } else { "Overlay" };
                        self.set_status(
                            format!("{layer} applied to {n} {}.", page_word(n)),
                            StatusKind::Success,
                        );
                    }
                    _ => self.set_status(
                        "Copy a PDF page, then select the pages to place it on.",
                        StatusKind::Info,
                    ),
                }
            }
            Action::Duplicate => {
                if has_sel {
                    let mut pages = Vec::new();
                    let mut new_sel = BTreeSet::new();
                    for (i, p) in self.pages.clone().into_iter().enumerate() {
                        let dup = self
                            .selected
                            .contains(&i)
                            .then(|| self.renumber(p.clone()));
                        pages.push(p);
                        if let Some(d) = dup {
                            new_sel.insert(pages.len());
                            pages.push(d);
                        }
                    }
                    let n = new_sel.len();
                    self.commit(pages);
                    self.selected = new_sel;
                    self.anchor = self.selected.iter().next().copied();
                    self.set_status(
                        format!("Duplicated {n} {}.", page_word(n)),
                        StatusKind::Success,
                    );
                } else {
                    self.set_status("Select pages to duplicate.", StatusKind::Info);
                }
            }
            Action::Delete => {
                if has_sel {
                    self.delete_selected();
                } else {
                    self.set_status("Select pages to delete.", StatusKind::Info);
                }
            }
            Action::InsertBlank => {
                let at = self
                    .selected
                    .iter()
                    .next_back()
                    .map_or(self.pages.len(), |&i| i + 1);
                let (w, h) = self
                    .pages
                    .get(at.saturating_sub(1))
                    .map_or((A4_W, A4_H), |p| (p.width_pt, p.height_pt));
                let blank = pdf_engine::blank_page(self.uid(), w, h);
                let mut pages = self.pages.clone();
                pages.insert(at, blank);
                self.commit(pages);
                self.select_only(at);
                self.scroll_to = Some(at);
                self.set_status("Inserted a blank page.", StatusKind::Success);
            }
            Action::SelectAll => {
                self.selected = (0..self.pages.len()).collect();
                self.anchor = self.selected.iter().next().copied();
            }
            Action::SelectNone => {
                self.selected.clear();
                self.anchor = None;
            }
            Action::Invert => {
                self.selected = arrange::invert_selection(self.pages.len(), &self.sel_vec())
                    .into_iter()
                    .collect();
                self.anchor = self.selected.iter().next().copied();
            }
            Action::SelectOdd => {
                self.selected = arrange::select_odd(&self.pages).into_iter().collect();
                self.anchor = self.selected.iter().next().copied();
            }
            Action::SelectEven => {
                self.selected = arrange::select_even(&self.pages).into_iter().collect();
                self.anchor = self.selected.iter().next().copied();
            }
            Action::SelectSame => {
                let ids: Vec<Option<SourceId>> =
                    self.selected_items().iter().map(|p| p.source_id).collect();
                self.selected = self
                    .pages
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| ids.contains(&p.source_id))
                    .map(|(i, _)| i)
                    .collect();
                self.anchor = self.selected.iter().next().copied();
            }
            Action::RotateLeft => self.rotate_selected(-90),
            Action::RotateRight => self.rotate_selected(90),
            Action::Rotate180 => self.rotate_selected(180),
            Action::CropDialog => {
                if let Some(p) = self.selected.iter().next().and_then(|&i| self.pages.get(i)) {
                    self.crop_edit = [
                        p.crop.left * 100.0,
                        p.crop.right * 100.0,
                        p.crop.top * 100.0,
                        p.crop.bottom * 100.0,
                    ];
                    self.scale_edit = p.scale * 100.0;
                    self.show_crop = true;
                    self.show_meta = false;
                    self.show_shortcuts = false;
                    self.show_about = false;
                    self.show_replace = false;
                } else {
                    self.set_status("Select pages to crop or scale.", StatusKind::Info);
                }
            }
            Action::ApplyCrop => {
                self.show_crop = false;
                self.apply_crop();
            }
            Action::Reverse => {
                let n = self.selected.len();
                let mut pages = self.pages.clone();
                if n >= 2 {
                    let idx: Vec<usize> = self.sel_vec().into_iter().filter(|&i| i < pages.len()).collect();
                    let mut chosen: Vec<PageItem> = idx.iter().map(|&i| pages[i].clone()).collect();
                    chosen.reverse();
                    for (k, &i) in idx.iter().enumerate() {
                        pages[i] = chosen[k].clone();
                    }
                    self.commit(pages);
                    self.set_status("Reversed the selected pages.", StatusKind::Success);
                } else {
                    arrange::reverse(&mut pages);
                    self.selected.clear();
                    self.anchor = None;
                    self.commit(pages);
                    self.set_status("Reversed the page order.", StatusKind::Success);
                }
            }
            Action::SwapOddEven => {
                let mut pages = self.pages.clone();
                arrange::swap_odd_even(&mut pages);
                self.commit(pages);
                self.selected.clear();
                self.anchor = None;
                self.set_status("Swapped odd and even pages.", StatusKind::Success);
            }
            Action::ZoomIn => self.zoom = (self.zoom * 1.15).min(MAX_ZOOM),
            Action::ZoomOut => self.zoom = (self.zoom / 1.15).max(MIN_ZOOM),
            Action::ZoomReset => self.zoom = DEFAULT_ZOOM,
            Action::ToggleTheme => {
                self.dark = !self.dark;
                theme::apply(ctx, self.dark);
            }
            Action::StepLeft => self.step_move_selected(-1),
            Action::StepRight => self.step_move_selected(1),
            Action::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Action::ShowShortcuts => {
                self.show_shortcuts = true;
                self.show_about = false;
                self.show_meta = false;
                self.show_crop = false;
                self.show_replace = false;
            }
            Action::ShowAbout => {
                self.show_about = true;
                self.show_shortcuts = false;
                self.show_meta = false;
                self.show_crop = false;
                self.show_replace = false;
            }
        }
    }

    fn renumber(&mut self, mut p: PageItem) -> PageItem {
        p.uid = self.uid();
        p
    }

    #[cfg(test)]
    fn add_loaded(&mut self, paths: &[PathBuf], replace: bool) {
        let mut next_source = self.next_source;
        let mut next_uid = self.next_uid;
        let (sources, items, errors) = load_paths_into(paths, &mut next_source, &mut next_uid);
        self.next_source = next_source;
        self.next_uid = next_uid;
        self.apply_loaded(sources, items, errors, paths.len(), replace);
    }

    // -------------------------------------------------------------------- UI

    fn header(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        ui.horizontal_centered(|ui| {
            ui.add_space(16.0);
            let sidebar_tip = if self.show_sidebar {
                "Hide sidebar (Ctrl+B)"
            } else {
                "Show sidebar (Ctrl+B)"
            };
            if activated(
                &Btn::icon(icons::PANEL_LEFT)
                    .selected(self.show_sidebar)
                    .tip(sidebar_tip)
                    .show(ui, t),
            ) {
                a.push(Action::ToggleSidebar);
            }
            ui.add_space(7.0);
            logo_mark(ui, t, Vec2::new(21.0, 25.0));
            ui.add_space(7.0);
            ui.label(
                RichText::new("PDF Wizard")
                    .font(serif_font(20.0))
                    .color(t.ink),
            );
            ui.add_space(13.0);
            ui.allocate_ui_with_layout(
                Vec2::new(270.0, HEADER_H),
                Layout::left_to_right(Align::Center),
                |ui| {
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.menu_button(
                            RichText::new("File").font(medium(13.0)),
                            |ui| self.file_menu(ui, a),
                        );
                        ui.menu_button(
                            RichText::new("Edit").font(medium(13.0)),
                            |ui| self.edit_menu(ui, a),
                        );
                        ui.menu_button(
                            RichText::new("Pages").font(medium(13.0)),
                            |ui| self.pages_menu(ui, a),
                        );
                        ui.menu_button(
                            RichText::new("View").font(medium(13.0)),
                            |ui| self.view_menu(ui, a),
                        );
                        ui.menu_button(
                            RichText::new("Help").font(medium(13.0)),
                            |ui| self.help_menu(ui, a),
                        );
                    });
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(16.0);
                let save_label = icons::with_icon(icons::SAVE, "Save PDF");
                let tip = if self.pages.is_empty() {
                    "Add pages before saving"
                } else {
                    "Save PDF (Ctrl+S)"
                };
                if activated(
                    &Btn::primary(&save_label)
                        .width(116.0)
                        .height(38.0)
                        .enabled(!self.pages.is_empty())
                        .tip(tip)
                        .show(ui, t),
                ) {
                    a.push(Action::Save);
                }
                ui.add_space(5.0);
                if activated(
                    &Btn::icon(icons::HELP)
                        .tip("Keyboard shortcuts (F1)")
                        .show(ui, t),
                ) {
                    a.push(Action::ShowShortcuts);
                }
                let theme_tip = if self.dark {
                    "Switch to the paper theme (Ctrl+Shift+L)"
                } else {
                    "Switch to the night theme (Ctrl+Shift+L)"
                };
                if activated(
                    &Btn::icon(if self.dark { icons::SUN } else { icons::MOON })
                        .tip(theme_tip)
                        .show(ui, t),
                ) {
                    a.push(Action::ToggleTheme);
                }
                ui.add_space(1.0);
                vertical_rule(ui, t, 26.0);
                let redo_tip = if self.history.can_redo() {
                    "Redo (Ctrl+Shift+Z)"
                } else {
                    "Nothing to redo"
                };
                if activated(
                    &Btn::icon(icons::REDO)
                        .enabled(self.history.can_redo())
                        .tip(redo_tip)
                        .show(ui, t),
                ) {
                    a.push(Action::Redo);
                }
                let undo_tip = if self.history.can_undo() {
                    "Undo (Ctrl+Z)"
                } else {
                    "Nothing to undo"
                };
                if activated(
                    &Btn::icon(icons::UNDO)
                        .enabled(self.history.can_undo())
                        .tip(undo_tip)
                        .show(ui, t),
                ) {
                    a.push(Action::Undo);
                }
            });
        });
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() - 0.5),
                egui::pos2(rect.right(), rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, t.line),
        );
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let has_pages = !self.pages.is_empty();
        let has_sel = !self.selected.is_empty();
        ui.set_min_width(248.0);
        menu_item(ui, a, "Add pages…", "Ctrl O", true, Action::Open);
        menu_item(
            ui,
            a,
            "Open and replace…",
            "Ctrl Shift O",
            true,
            Action::OpenReplace,
        );
        menu_item(ui, a, "Import images…", "", true, Action::ImportImages);
        ui.separator();
        menu_item(ui, a, "Save PDF…", "Ctrl S", has_pages, Action::Save);
        menu_item(
            ui,
            a,
            "Extract selected…",
            "",
            has_sel,
            Action::ExtractSingle,
        );
        menu_item(
            ui,
            a,
            "Split all to folder…",
            "",
            has_pages,
            Action::SplitAll,
        );
        menu_item(
            ui,
            a,
            "Generate booklet…",
            "",
            has_pages,
            Action::Booklet,
        );
        ui.separator();
        menu_item(
            ui,
            a,
            "Document details…",
            "",
            has_pages,
            Action::Metadata,
        );
        ui.separator();
        menu_item(ui, a, "Quit", "Ctrl Q", true, Action::Quit);
    }

    fn edit_menu(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let has_sel = !self.selected.is_empty();
        let has_clip = !self.clipboard.is_empty();
        ui.set_min_width(248.0);
        menu_item(ui, a, "Undo", "Ctrl Z", self.history.can_undo(), Action::Undo);
        menu_item(
            ui,
            a,
            "Redo",
            "Ctrl Shift Z",
            self.history.can_redo(),
            Action::Redo,
        );
        ui.separator();
        menu_item(ui, a, "Cut", "Ctrl X", has_sel, Action::Cut);
        menu_item(ui, a, "Copy", "Ctrl C", has_sel, Action::Copy);
        menu_item(ui, a, "Paste after", "Ctrl V", has_clip, Action::PasteAfter);
        menu_item(
            ui,
            a,
            "Paste as overlay",
            "",
            has_clip && has_sel,
            Action::PasteOverlay,
        );
        menu_item(
            ui,
            a,
            "Paste as underlay",
            "",
            has_clip && has_sel,
            Action::PasteUnderlay,
        );
        menu_item(ui, a, "Duplicate", "Ctrl D", has_sel, Action::Duplicate);
        menu_item(ui, a, "Delete", "Del", has_sel, Action::Delete);
        ui.separator();
        menu_item(ui, a, "Insert blank page", "", true, Action::InsertBlank);
    }

    fn pages_menu(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let has_pages = !self.pages.is_empty();
        let has_sel = !self.selected.is_empty();
        ui.set_min_width(248.0);
        menu_item(ui, a, "Select all", "Ctrl A", has_pages, Action::SelectAll);
        menu_item(ui, a, "Deselect all", "Esc", has_sel, Action::SelectNone);
        menu_item(ui, a, "Invert selection", "", has_pages, Action::Invert);
        ui.separator();
        menu_item(ui, a, "Select odd pages", "", has_pages, Action::SelectOdd);
        menu_item(ui, a, "Select even pages", "", has_pages, Action::SelectEven);
        menu_item(
            ui,
            a,
            "Select same source",
            "",
            has_sel,
            Action::SelectSame,
        );
        ui.separator();
        menu_item(
            ui,
            a,
            "Move left",
            "Alt Left",
            has_sel,
            Action::StepLeft,
        );
        menu_item(
            ui,
            a,
            "Move right",
            "Alt Right",
            has_sel,
            Action::StepRight,
        );
        ui.separator();
        menu_item(ui, a, "Rotate left 90°", "", has_sel, Action::RotateLeft);
        menu_item(ui, a, "Rotate right 90°", "", has_sel, Action::RotateRight);
        menu_item(ui, a, "Rotate 180°", "", has_sel, Action::Rotate180);
        menu_item(
            ui,
            a,
            "Crop and scale…",
            "",
            has_sel,
            Action::CropDialog,
        );
        ui.separator();
        menu_item(ui, a, "Reverse order", "", has_pages, Action::Reverse);
        menu_item(
            ui,
            a,
            "Swap odd and even",
            "",
            has_pages,
            Action::SwapOddEven,
        );
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let has_pages = !self.pages.is_empty();
        ui.set_min_width(248.0);
        menu_check_item(
            ui,
            a,
            "Sidebar",
            "Ctrl B",
            self.show_sidebar,
            Action::ToggleSidebar,
        );
        menu_check_item(
            ui,
            a,
            "Dark theme",
            "Ctrl Shift L",
            self.dark,
            Action::ToggleTheme,
        );
        ui.separator();
        menu_item(ui, a, "Zoom in", "Ctrl +", has_pages, Action::ZoomIn);
        menu_item(ui, a, "Zoom out", "Ctrl -", has_pages, Action::ZoomOut);
        menu_item(ui, a, "Actual size", "Ctrl 0", has_pages, Action::ZoomReset);
    }

    fn help_menu(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        ui.set_min_width(248.0);
        menu_item(
            ui,
            a,
            "Keyboard shortcuts",
            "F1",
            true,
            Action::ShowShortcuts,
        );
        menu_item(ui, a, "About PDF Wizard", "", true, Action::ShowAbout);
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.document_block(ui, t);
                ui.add_space(18.0);
                section_label(ui, t, "ADD PAGES");
                ui.add_space(8.0);
                let add_label = icons::with_icon(icons::FOLDER_OPEN, "Add pages");
                if activated(
                    &Btn::primary(&add_label)
                        .width(ui.available_width())
                        .height(42.0)
                        .tip("Choose PDFs or images to append")
                        .show(ui, t),
                ) {
                    a.push(Action::Open);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let half = ((ui.available_width() - 8.0) * 0.5).max(80.0);
                    let image_label = icons::with_icon(icons::IMAGE_PLUS, "Images");
                    if activated(
                        &Btn::secondary(&image_label)
                            .width(half)
                            .height(38.0)
                            .font(medium(12.0))
                            .tip("Import images as pages")
                            .show(ui, t),
                    ) {
                        a.push(Action::ImportImages);
                    }
                    let blank_label = icons::with_icon(icons::FILE_PLUS, "Blank");
                    if activated(
                        &Btn::secondary(&blank_label)
                            .width(half)
                            .height(38.0)
                            .font(medium(12.0))
                            .tip("Insert a blank A4 page")
                            .show(ui, t),
                    ) {
                        a.push(Action::InsertBlank);
                    }
                });
                ui.add_space(3.0);
                if activated(
                    &Btn::ghost("Open and replace…")
                        .width(ui.available_width())
                        .height(30.0)
                        .font(body(11.5))
                        .tip("Replace every page on the bench")
                        .show(ui, t),
                ) {
                    a.push(Action::OpenReplace);
                }

                if !self.pages.is_empty() {
                    ui.add_space(17.0);
                    hairline(ui, t);
                    ui.add_space(16.0);
                    self.selection_section(ui, t, a);
                    ui.add_space(17.0);
                    hairline(ui, t);
                    ui.add_space(16.0);
                    self.export_section(ui, t, a);
                } else {
                    ui.add_space(18.0);
                    ui.add(
                        Label::new(
                            RichText::new(
                                "Open a PDF or images. Every page becomes a card on the bench.",
                            )
                            .font(body(11.5))
                            .color(t.muted)
                            .line_height(Some(17.0)),
                        )
                        .wrap(),
                    );

                    ui.add_space(20.0);
                    hairline(ui, t);
                    ui.add_space(16.0);
                    section_label(ui, t, "EXPORT");
                    ui.add_space(8.0);
                    let save_label = icons::with_icon(icons::SAVE, "Save PDF");
                    let _ = Btn::primary(&save_label)
                        .width(ui.available_width())
                        .height(42.0)
                        .enabled(false)
                        .tip("Add pages before saving")
                        .show(ui, t);
                }
                ui.add_space(20.0);
            });
    }

    fn document_block(&mut self, ui: &mut egui::Ui, t: Tokens) {
        Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.line))
            .corner_radius(CornerRadius::same(RADIUS_CONTROL))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_label(ui, t, "DOCUMENT");
                ui.add_space(5.0);
                let name = self.document_name();
                let shown = truncate_middle(&name, 26);
                let name_color = if self.pages.is_empty() { t.muted } else { t.ink };
                ui.add(
                    Label::new(
                        RichText::new(&shown)
                            .font(serif_font(19.0))
                            .color(name_color),
                    )
                    .truncate(),
                )
                .on_hover_text(&name);
                ui.add_space(2.0);
                ui.label(
                    RichText::new(self.document_meta())
                        .font(body(11.5))
                        .color(t.muted),
                );
            });
    }

    fn selection_section(&mut self, ui: &mut egui::Ui, t: Tokens, a: &mut Vec<Action>) {
        let has_sel = !self.selected.is_empty();
        ui.horizontal(|ui| {
            section_label(ui, t, "SELECTION");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if activated(
                    &Btn::ghost("Clear")
                        .width(52.0)
                        .height(26.0)
                        .font(body(11.5))
                        .enabled(has_sel)
                        .tip("Clear selection (Esc)")
                        .show(ui, t),
                ) {
                    a.push(Action::SelectNone);
                }
            });
        });
        ui.add_space(6.0);
        if !has_sel {
            ui.add(
                Label::new(
                    RichText::new("Click a page to select it. Shift-click for a range, Ctrl-click to add or remove.")
                        .font(body(11.5))
                        .color(t.muted)
                        .line_height(Some(17.0)),
                )
                .wrap(),
            );
            return;
        }

        let n = self.selected.len();
        ui.label(
            RichText::new(format!("{n} {} selected", page_word(n)))
                .font(medium(12.5))
                .color(t.accent),
        );
        ui.add_space(7.0);

        let half = ((ui.available_width() - 8.0) * 0.5).max(80.0);
        let compact = medium(12.0);
        let rotate_left = icons::with_icon(icons::ROTATE_CCW, "Rotate left");
        let rotate_right = icons::with_icon(icons::ROTATE_CW, "Rotate right");
        let crop = icons::with_icon(icons::CROP, "Crop");
        let duplicate = icons::with_icon(icons::COPY_PLUS, "Duplicate");
        let move_left = icons::with_icon(icons::ARROW_LEFT, "Move left");
        let move_right = icons::with_icon(icons::ARROW_RIGHT, "Move right");
        let extract = icons::with_icon(icons::FILE_OUTPUT, "Extract");
        let delete = icons::with_icon(icons::TRASH, "Delete");

        ui.horizontal(|ui| {
            if activated(
                &Btn::secondary(&rotate_left)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Rotate counterclockwise")
                    .show(ui, t),
            ) {
                a.push(Action::RotateLeft);
            }
            if activated(
                &Btn::secondary(&rotate_right)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Rotate clockwise")
                    .show(ui, t),
            ) {
                a.push(Action::RotateRight);
            }
        });
        ui.horizontal(|ui| {
            if activated(
                &Btn::secondary(&crop)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Crop and scale selected pages")
                    .show(ui, t),
            ) {
                a.push(Action::CropDialog);
            }
            if activated(
                &Btn::secondary(&duplicate)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Duplicate selected pages (Ctrl+D)")
                    .show(ui, t),
            ) {
                a.push(Action::Duplicate);
            }
        });
        ui.horizontal(|ui| {
            if activated(
                &Btn::secondary(&move_left)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Move selection left (Alt+Left)")
                    .show(ui, t),
            ) {
                a.push(Action::StepLeft);
            }
            if activated(
                &Btn::secondary(&move_right)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Move selection right (Alt+Right)")
                    .show(ui, t),
            ) {
                a.push(Action::StepRight);
            }
        });
        ui.horizontal(|ui| {
            if activated(
                &Btn::secondary(&extract)
                    .width(half)
                    .height(38.0)
                    .font(compact.clone())
                    .tip("Save selected pages as one PDF")
                    .show(ui, t),
            ) {
                a.push(Action::ExtractSingle);
            }
            if activated(
                &Btn::danger(&delete)
                    .width(half)
                    .height(38.0)
                    .font(compact)
                    .tip("Remove selected pages (Delete)")
                    .show(ui, t),
            ) {
                a.push(Action::Delete);
            }
        });
    }

    fn export_section(&mut self, ui: &mut egui::Ui, t: Tokens, a: &mut Vec<Action>) {
        let has_sel = !self.selected.is_empty();
        section_label(ui, t, "EXPORT");
        ui.add_space(8.0);
        let save_label = icons::with_icon(icons::SAVE, "Save PDF");
        if activated(
            &Btn::primary(&save_label)
                .width(ui.available_width())
                .height(42.0)
                .tip("Save the whole document (Ctrl+S)")
                .show(ui, t),
        ) {
            a.push(Action::Save);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let half = ((ui.available_width() - 8.0) * 0.5).max(80.0);
            let extract = icons::with_icon(icons::FILE_OUTPUT, "Extract");
            if activated(
                &Btn::secondary(&extract)
                    .width(half)
                    .height(38.0)
                    .font(medium(12.0))
                    .enabled(has_sel)
                    .tip(if has_sel {
                        "Save selected pages as one PDF"
                    } else {
                        "Select pages to extract"
                    })
                    .show(ui, t),
            ) {
                a.push(Action::ExtractSingle);
            }
            let split = icons::with_icon(icons::SCISSORS, "Split all");
            if activated(
                &Btn::secondary(&split)
                    .width(half)
                    .height(38.0)
                    .font(medium(12.0))
                    .tip("Write every page as its own PDF")
                    .show(ui, t),
            ) {
                a.push(Action::SplitAll);
            }
        });
        ui.add_space(8.0);
        let booklet = icons::with_icon(icons::BOOK_OPEN, "Booklet");
        if activated(
            &Btn::secondary(&booklet)
                .width(ui.available_width())
                .height(38.0)
                .tip("Impose pages 2-up in saddle-stitch order")
                .show(ui, t),
        ) {
            a.push(Action::Booklet);
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        let rect = ui.max_rect();
        self.canvas_rect = Some(rect);
        let bar_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), CANVAS_BAR_H));
        let bench = Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + CANVAS_BAR_H),
            rect.max,
        );
        dotted_bench(ui.painter(), bench, t);

        let mut bar_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(bar_rect.shrink2(Vec2::new(24.0, 0.0)))
                .layout(Layout::top_down(Align::Min)),
        );
        self.canvas_bar(&mut bar_ui, a);

        ui.painter().line_segment(
            [
                egui::pos2(bar_rect.left(), bar_rect.bottom() - 0.5),
                egui::pos2(bar_rect.right(), bar_rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, t.line),
        );

        if self.pages.is_empty() {
            let mut content = ui.new_child(
                UiBuilder::new()
                    .max_rect(bench)
                    .layout(Layout::top_down(Align::Min)),
            );
            self.empty_state(&mut content, a);
        } else {
            let mut content = ui.new_child(
                UiBuilder::new()
                    .max_rect(bench)
                    .layout(Layout::top_down(Align::Min)),
            );
            self.grid(&mut content, a);
        }
    }

    fn canvas_bar(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Pages")
                    .font(serif_font(21.0))
                    .color(t.ink),
            );
            if self.pages.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("No pages yet")
                        .font(body(11.5))
                        .color(t.muted),
                );
            } else {
                ui.add_space(7.0);
                Frame::new()
                    .fill(t.surface_hover)
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(6, 1))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(self.pages.len().to_string())
                                .font(mono(10.5))
                                .color(t.muted),
                        );
                    });
                if !self.selected.is_empty() {
                    ui.add_space(7.0);
                    ui.label(RichText::new("·").font(body(12.0)).color(t.line_strong));
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new(format!("{} selected", self.selected.len()))
                            .font(medium(11.5))
                            .color(t.accent),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if activated(
                        &Btn::icon(icons::ZOOM_IN)
                            .tip("Zoom in (Ctrl+=)")
                            .show(ui, t),
                    ) {
                        a.push(Action::ZoomIn);
                    }
                    let percent = (self.zoom / DEFAULT_ZOOM * 100.0).round() as i32;
                    if activated(
                        &Btn::ghost(&format!("{percent}%"))
                            .width(58.0)
                            .height(34.0)
                            .font(mono(10.5))
                            .tip("Reset zoom (Ctrl+0)")
                            .show(ui, t),
                    ) {
                        a.push(Action::ZoomReset);
                    }
                    if activated(
                        &Btn::icon(icons::ZOOM_OUT)
                            .tip("Zoom out (Ctrl+-)")
                            .show(ui, t),
                    ) {
                        a.push(Action::ZoomOut);
                    }
                });
            }
        });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        let avail = ui.max_rect();
        let wide = avail.width() >= 760.0;
        let content_w = if wide { 750.0 } else { avail.width().min(430.0) };
        let content_h = 336.0;
        let top = (avail.top() + ((avail.height() - content_h) * 0.42).max(24.0))
            .min((avail.bottom() - content_h - 20.0).max(avail.top() + 20.0));
        let left = avail.center().x - content_w * 0.5;
        let text_w = if wide { 382.0 } else { content_w };
        let text_rect = Rect::from_min_size(egui::pos2(left, top), Vec2::new(text_w, content_h));
        let mut text_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        {
            let ui = &mut text_ui;
            ui.set_width(text_w);
            ui.add_space(8.0);
            let (rule, _) = ui.allocate_exact_size(Vec2::new(24.0, 2.0), Sense::hover());
            ui.painter()
                .rect_filled(rule, CornerRadius::same(1), t.accent);
            ui.add_space(10.0);
            ui.label(
                RichText::new("START A DOCUMENT")
                    .font(label_font())
                    .color(t.accent)
                    .extra_letter_spacing(0.9),
            );
            ui.add_space(9.0);
            ui.add(
                Label::new(
                    RichText::new("Give every page its place.")
                        .font(serif_font(38.0))
                        .color(t.ink)
                        .line_height(Some(43.0)),
                )
                .wrap(),
            );
            ui.add_space(11.0);
            ui.add(
                Label::new(
                    RichText::new(
                        "Open PDFs and images, reorder or turn the pages, then save a clean PDF. Nothing is written until you export.",
                    )
                    .font(body(14.0))
                    .color(t.muted)
                    .line_height(Some(21.0)),
                )
                .wrap(),
            );
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                let choose = icons::with_icon(icons::FOLDER_OPEN, "Choose files");
                if activated(
                    &Btn::primary(&choose)
                        .width(156.0)
                        .height(44.0)
                        .font(medium(13.5))
                        .tip("Add PDFs or images")
                        .show(ui, t),
                ) {
                    a.push(Action::Open);
                }
                let blank = icons::with_icon(icons::FILE_PLUS, "Blank page");
                if activated(
                    &Btn::secondary(&blank)
                        .width(136.0)
                        .height(44.0)
                        .font(medium(13.0))
                        .tip("Start with an empty A4 page")
                        .show(ui, t),
                ) {
                    a.push(Action::InsertBlank);
                }
            });
            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::UPLOAD)
                        .font(icon_font(13.0))
                        .color(t.muted),
                );
                ui.add_space(1.0);
                ui.label(
                    RichText::new("or drop files anywhere")
                        .font(body(12.0))
                        .color(t.muted),
                );
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new("PDF · PNG · JPEG · WEBP")
                    .font(mono(10.5))
                    .color(t.muted),
            );
        }

        if wide {
            let illo_rect = Rect::from_min_size(
                egui::pos2(left + text_w + 44.0, top - 4.0),
                Vec2::new(324.0, content_h),
            );
            let mut illo_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(illo_rect)
                    .layout(Layout::top_down(Align::Center)),
            );
            stacked_sheets(&mut illo_ui, t, Vec2::new(318.0, content_h));
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        let card = Vec2::new(self.zoom, (self.zoom * 1.30 + 62.0).round());
        let bottom_pad = if self.selected.is_empty() { 28.0 } else { 92.0 };
        self.drop_target = None;
        let mut cards: Vec<(usize, Rect)> = Vec::new();
        let pending_scroll: Option<usize> = self.scroll_to.take();
        let mut scrolled = false;

        egui::ScrollArea::vertical()
            .id_salt("page_grid")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(GRID_GAP);
                ui.add_space(20.0);
                let avail = (ui.available_width() - 48.0).max(1.0);
                let cols = grid_columns(avail, card.x, GRID_GAP);
                let n = self.pages.len();
                for start in (0..n).step_by(cols) {
                    ui.horizontal_top(|ui| {
                        ui.add_space(24.0);
                        for i in start..(start + cols).min(n) {
                            let rect = self.page_card(ui, i, card, a);
                            cards.push((i, rect));
                            if pending_scroll == Some(i) {
                                ui.scroll_to_rect(rect, Some(Align::Center));
                                scrolled = true;
                            }
                        }
                    });
                }
                ui.add_space(bottom_pad);

                if self.dragging {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    let pointer = ui.input(|i| i.pointer.hover_pos());
                    if let Some(pos) = pointer {
                        let mut best: Option<(usize, f32, bool)> = None;
                        for &(i, rect) in &cards {
                            let hit = rect.expand2(Vec2::new(GRID_GAP * 0.7, GRID_GAP * 0.55));
                            if hit.contains(pos) {
                                let d = (pos.x - rect.center().x).abs();
                                if best.is_none_or(|(_, bd, _)| d < bd) {
                                    best = Some((i, d, pos.x < rect.center().x));
                                }
                            }
                        }
                        if let Some((i, _, before)) = best {
                            self.drop_target = Some(if before { i } else { i + 1 });
                        }
                        let sel = self.sel_vec();
                        if let Some(target) = self.drop_target {
                            if !drop_is_noop(&sel, target) {
                                let slot = cards
                                    .iter()
                                    .find(|(i, _)| *i == target.min(n.saturating_sub(1)))
                                    .map(|(_, r)| *r);
                                if let Some(slot) = slot {
                                    let x = if target >= n {
                                        slot.right() + GRID_GAP * 0.5
                                    } else {
                                        slot.left() - GRID_GAP * 0.5
                                    };
                                    ui.painter().rect_filled(
                                        Rect::from_center_size(
                                            egui::pos2(x, slot.center().y),
                                            Vec2::new(2.0, slot.height()),
                                        ),
                                        CornerRadius::same(1),
                                        t.accent,
                                    );
                                }
                            }
                        }
                        let clip = ui.clip_rect();
                        let scroll = if pos.y > clip.bottom() - 38.0 {
                            Some(-14.0)
                        } else if pos.y < clip.top() + 38.0 {
                            Some(14.0)
                        } else {
                            None
                        };
                        if let Some(dy) = scroll {
                            ui.scroll_with_delta(Vec2::new(0.0, dy));
                            ui.ctx().request_repaint();
                        }
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        let target = self.drop_target.take();
                        self.dragging = false;
                        if let Some(target) = target {
                            self.move_selected_to(target);
                        }
                    }
                }
            });

        if scrolled {
            self.scroll_to = None;
        } else if pending_scroll.is_some() {
            self.scroll_to = pending_scroll;
        }
    }

    fn page_card(&mut self, ui: &mut egui::Ui, i: usize, size: Vec2, a: &mut Vec<Action>) -> Rect {
        let t = Tokens::for_mode(self.dark);
        let item = self.pages[i].clone();
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let selected = self.selected.contains(&i);
        let hover_t = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hover"), resp.hovered() && !self.dragging, 0.12);
        let sel_t = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("selected"), selected, 0.14);
        let lift = 2.0 * hover_t;
        let draw_rect = rect.translate(Vec2::new(0.0, -lift));
        let check_hit = Rect::from_min_size(
            draw_rect.min + Vec2::new(6.0, (CARD_HEADER_H - CARD_BTN_H) * 0.5),
            Vec2::new(CARD_BTN_H, CARD_BTN_H),
        );
        let grip_rect = Rect::from_min_size(
            egui::pos2(draw_rect.right() - 34.0, check_hit.top()),
            Vec2::new(28.0, CARD_BTN_H),
        );
        let title_rect = Rect::from_min_max(
            egui::pos2(check_hit.right() + 2.0, draw_rect.top()),
            egui::pos2(grip_rect.left() - 2.0, draw_rect.top() + CARD_HEADER_H),
        );

        let thumb = item
            .source_id
            .map(|sid| self.ensure_thumb(sid, item.source_page_num));
        let pointer_in_grip = ui
            .input(|inp| inp.pointer.hover_pos())
            .is_some_and(|p| grip_rect.contains(p));

        if ui.is_rect_visible(draw_rect) {
            let base = t.surface.lerp_to_gamma(t.surface_hover, hover_t);
            let bg = base.lerp_to_gamma(t.accent_tint, sel_t * 0.92);
            let border = t
                .line
                .lerp_to_gamma(t.line_strong, hover_t)
                .lerp_to_gamma(t.accent, sel_t);
            let border_w = 1.0 + 0.5 * sel_t;
            let painter = ui.painter_at(draw_rect.expand(lift + 1.0));
            if hover_t > 0.01 || sel_t > 0.01 {
                let mut shadow = t.card_shadow();
                let strength = hover_t.max(sel_t);
                shadow.color = Tokens::alpha(t.shadow, (shadow.color.a() as f32 * (0.3 + 0.7 * strength)) as u8);
                painter.add(shadow.as_shape(draw_rect, RADIUS_CARD));
            }
            painter.rect_filled(draw_rect, CornerRadius::same(RADIUS_CARD), bg);
            painter.rect_stroke(
                draw_rect,
                CornerRadius::same(RADIUS_CARD),
                Stroke::new(border_w, border),
                StrokeKind::Inside,
            );

            // Folio and transform tags.
            let folio_color = if selected { t.accent } else { t.muted };
            painter.with_clip_rect(title_rect).text(
                title_rect.center(),
                Align2::CENTER_CENTER,
                card_header_text(i, &item, size.x),
                mono(10.5),
                folio_color,
            );

            // Checkbox.
            let check_size = 18.0;
            let check = Rect::from_center_size(check_hit.center(), Vec2::splat(check_size));
            if selected {
                painter.rect_filled(check, CornerRadius::same(4), t.accent);
                painter.text(
                    check.center(),
                    Align2::CENTER_CENTER,
                    icons::CHECK,
                    icon_font(12.0),
                    t.on_accent,
                );
            } else {
                let edge = if hover_t > 0.4 {
                    t.line_strong.lerp_to_gamma(t.accent, hover_t * 0.6)
                } else {
                    t.line_strong
                };
                painter.rect_filled(check, CornerRadius::same(4), t.surface);
                painter.rect_stroke(
                    check,
                    CornerRadius::same(4),
                    Stroke::new(1.0, edge),
                    StrokeKind::Inside,
                );
            }

            // Grip.
            let grip_color = if pointer_in_grip || self.dragging {
                t.accent
            } else if selected {
                t.muted
            } else {
                t.faint
            };
            painter.text(
                grip_rect.center(),
                Align2::CENTER_CENTER,
                icons::GRIP,
                icon_font(14.0),
                grip_color,
            );

            // Paper.
            let area = Rect::from_min_max(
                egui::pos2(draw_rect.left() + 8.0, draw_rect.top() + CARD_HEADER_H),
                egui::pos2(draw_rect.right() - 8.0, draw_rect.bottom() - CARD_FOOTER_H),
            );
            let (bw, bh) = match &thumb {
                Some(Thumb::Ready { w, h, .. }) => (*w, *h),
                _ => (item.width_pt, item.height_pt),
            };
            let c = item.crop;
            let cw = (bw * (1.0 - c.left - c.right)).max(1.0);
            let ch = (bh * (1.0 - c.top - c.bottom)).max(1.0);
            let swap = item.rotation == 90 || item.rotation == 270;
            let (dw, dh) = if swap { (ch, cw) } else { (cw, ch) };
            let s = (area.width() / dw).min(area.height() / dh);
            let shown = Vec2::new(dw * s, dh * s);
            let paper = Rect::from_center_size(area.center(), shown);
            painter.add(
                t.paper_shadow()
                    .as_shape(paper.translate(Vec2::new(0.0, 2.0)), RADIUS_PAPER),
            );
            painter.rect_filled(paper, CornerRadius::same(RADIUS_PAPER), t.paper);

            match &thumb {
                Some(Thumb::Ready { tex, w, h }) => {
                    let unrot = if swap {
                        Vec2::new(shown.y, shown.x)
                    } else {
                        shown
                    };
                    let img_rect = Rect::from_center_size(paper.center(), unrot);
                    let uv = Rect::from_min_max(
                        egui::pos2(c.left, c.top),
                        egui::pos2(1.0 - c.right, 1.0 - c.bottom),
                    );
                    let angle = (item.rotation as f32).to_radians();
                    ui.scope_builder(
                        UiBuilder::new()
                            .max_rect(paper)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.set_clip_rect(paper.intersect(ui.clip_rect()));
                            egui::Image::new(egui::load::SizedTexture::new(
                                tex.id(),
                                Vec2::new(*w, *h),
                            ))
                            .uv(uv)
                            .rotate(angle, Vec2::splat(0.5))
                            .paint_at(ui, img_rect);
                        },
                    );
                }
                Some(Thumb::Loading) | None => {
                    let rule = Stroke::new(2.0, t.line);
                    for f in [0.34, 0.44, 0.54] {
                        let y = paper.top() + paper.height() * f;
                        painter.line_segment(
                            [
                                egui::pos2(paper.left() + paper.width() * 0.18, y),
                                egui::pos2(paper.left() + paper.width() * 0.78, y),
                            ],
                            rule,
                        );
                    }
                    let spinner_rect = Rect::from_center_size(
                        egui::pos2(paper.center().x, paper.center().y + paper.height() * 0.24),
                        Vec2::splat(18.0),
                    );
                    ui.scope_builder(
                        UiBuilder::new()
                            .max_rect(spinner_rect)
                            .layout(Layout::top_down(Align::Center)),
                        |ui| {
                            ui.add(egui::Spinner::new().size(16.0).color(t.muted));
                        },
                    );
                }
                Some(Thumb::Failed) => {
                    painter.text(
                        egui::pos2(paper.center().x, paper.center().y - 8.0),
                        Align2::CENTER_CENTER,
                        icons::ALERT,
                        icon_font(16.0),
                        t.faint,
                    );
                    painter.text(
                        egui::pos2(paper.center().x, paper.center().y + 12.0),
                        Align2::CENTER_CENTER,
                        "No preview",
                        body(10.5),
                        t.muted,
                    );
                }
            }

            // Caption.
            let caption = short_source(&item.label);
            let max_chars = ((size.x / 5.4) as usize).max(8);
            let caption = truncate_middle(caption, max_chars);
            painter.with_clip_rect(Rect::from_min_max(
                egui::pos2(draw_rect.left() + 8.0, draw_rect.bottom() - CARD_FOOTER_H),
                egui::pos2(draw_rect.right() - 8.0, draw_rect.bottom()),
            ))
            .text(
                egui::pos2(draw_rect.center().x, draw_rect.bottom() - CARD_FOOTER_H * 0.48),
                Align2::CENTER_CENTER,
                caption,
                body(10.5),
                t.muted,
            );

            if resp.has_focus() {
                painter.rect_stroke(
                    draw_rect.expand(1.5),
                    CornerRadius::same(RADIUS_CARD + 1),
                    Stroke::new(1.5, t.accent),
                    StrokeKind::Middle,
                );
            }
        }

        if pointer_in_grip {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let mods = ui.input(|inp| inp.modifiers);
        let keyboard_select = resp.has_focus()
            && ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::Space)
            });
        if keyboard_select {
            self.handle_card_click(i, mods.shift, mods.command);
        } else if activated(&resp) {
            let pos = resp
                .interact_pointer_pos()
                .unwrap_or_else(|| rect.center());
            if check_hit.contains(pos) {
                self.toggle_select(i);
            } else {
                self.handle_card_click(i, mods.shift, mods.command);
            }
        }
        if resp.secondary_clicked() && !selected {
            self.select_only(i);
        }
        let drag_allowed = !resp
            .interact_pointer_pos()
            .is_some_and(|p| check_hit.contains(p));
        if resp.drag_started() && drag_allowed {
            if !self.selected.contains(&i) {
                self.select_only(i);
            }
            self.dragging = true;
        }

        let full_label = format!(
            "{} · Click to select, drag to reorder",
            if item.label.is_empty() {
                format!("Page {}", i + 1)
            } else {
                item.label.clone()
            }
        );
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::Button,
                true,
                selected,
                format!("Page {}, {}", i + 1, item.label),
            )
        });
        let resp = resp.on_hover_text(full_label);
        resp.context_menu(|ui| {
            let has_sel = !self.selected.is_empty();
            let has_clip = !self.clipboard.is_empty();
            menu_item(ui, a, "Rotate left 90°", "", has_sel, Action::RotateLeft);
            menu_item(ui, a, "Rotate right 90°", "", has_sel, Action::RotateRight);
            menu_item(ui, a, "Rotate 180°", "", has_sel, Action::Rotate180);
            menu_item(ui, a, "Crop and scale…", "", has_sel, Action::CropDialog);
            ui.separator();
            menu_item(ui, a, "Copy", "Ctrl C", has_sel, Action::Copy);
            menu_item(ui, a, "Cut", "Ctrl X", has_sel, Action::Cut);
            menu_item(ui, a, "Paste after", "Ctrl V", has_clip, Action::PasteAfter);
            menu_item(
                ui,
                a,
                "Paste as overlay",
                "",
                has_clip && has_sel,
                Action::PasteOverlay,
            );
            menu_item(
                ui,
                a,
                "Paste as underlay",
                "",
                has_clip && has_sel,
                Action::PasteUnderlay,
            );
            menu_item(ui, a, "Duplicate", "Ctrl D", has_sel, Action::Duplicate);
            menu_item(ui, a, "Delete", "Del", has_sel, Action::Delete);
            ui.separator();
            menu_item(ui, a, "Extract selected…", "", has_sel, Action::ExtractSingle);
            menu_item(
                ui,
                a,
                "Extract separate files…",
                "",
                has_sel,
                Action::ExtractSeparate,
            );
        });
        rect
    }

    fn selection_tray(&mut self, ctx: &egui::Context, a: &mut Vec<Action>) {
        let t = Tokens::for_mode(self.dark);
        if !self.selected.is_empty() {
            self.tray_count = self.selected.len();
        }
        let show = !self.selected.is_empty() && !self.modal_open();
        let progress = ctx.animate_bool_with_time(Id::new("selection-tray"), show, 0.18);
        if progress <= 0.001 || self.tray_count == 0 {
            return;
        }
        let Some(canvas) = self.canvas_rect else {
            return;
        };
        let x = (canvas.center().x - TRAY_W * 0.5)
            .clamp(canvas.left() + 16.0, (canvas.right() - TRAY_W - 16.0).max(canvas.left() + 16.0));
        let y = canvas.bottom() - TRAY_H - 16.0 + 12.0 * (1.0 - progress);
        let area_ui = egui::Area::new(Id::new("selection_tray_area"))
            .order(Order::Foreground)
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                ui.set_opacity(progress);
                Frame::new()
                    .fill(t.surface)
                    .stroke(Stroke::new(1.0, t.line_strong))
                    .corner_radius(CornerRadius::same(RADIUS_MODAL))
                    .inner_margin(Margin::symmetric(12, 10))
                    .shadow(t.card_shadow())
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            ui.label(
                                RichText::new(icons::CIRCLE_CHECK)
                                    .font(icon_font(15.0))
                                    .color(t.accent),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!("{} selected", self.tray_count))
                                    .font(semibold(12.5))
                                    .color(t.accent),
                            );
                            ui.add_space(2.0);
                            vertical_rule(ui, t, TRAY_H - 18.0);
                            tray_icon(ui, t, icons::ARROW_LEFT, "Move left (Alt+Left)", a, Action::StepLeft);
                            tray_icon(ui, t, icons::ARROW_RIGHT, "Move right (Alt+Right)", a, Action::StepRight);
                            vertical_rule(ui, t, TRAY_H - 18.0);
                            tray_icon(ui, t, icons::ROTATE_CCW, "Rotate left", a, Action::RotateLeft);
                            tray_icon(ui, t, icons::ROTATE_CW, "Rotate right", a, Action::RotateRight);
                            tray_icon(ui, t, icons::CROP, "Crop and scale", a, Action::CropDialog);
                            tray_icon(ui, t, icons::COPY_PLUS, "Duplicate (Ctrl+D)", a, Action::Duplicate);
                            vertical_rule(ui, t, TRAY_H - 18.0);
                            tray_icon(ui, t, icons::FILE_OUTPUT, "Export selection", a, Action::ExtractSingle);
                            if activated(
                                &Btn::icon(icons::TRASH)
                                    .tip("Delete selected pages (Del)")
                                    .show(ui, t),
                            ) {
                                a.push(Action::Delete);
                            }
                            ui.add_space(3.0);
                            vertical_rule(ui, t, TRAY_H - 18.0);
                            if activated(
                                &Btn::icon(icons::CLOSE)
                                    .selected(false)
                                    .tip("Clear selection (Esc)")
                                    .show(ui, t),
                            ) {
                                a.push(Action::SelectNone);
                            }
                        });
                    });
            });
        let _ = area_ui.response; // keep the area alive this frame
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::for_mode(self.dark);
        ui.horizontal_centered(|ui| {
            ui.add_space(16.0);
            if let Some(msg) = &self.busy {
                ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(msg)
                        .font(medium(11.5))
                        .color(t.accent),
                );
            } else {
                let (tone, text_color) = match self.status_kind {
                    StatusKind::Success => (t.success, t.success),
                    StatusKind::Error => (t.danger, t.danger),
                    StatusKind::Info => (t.muted, t.muted),
                };
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.0, tone);
                ui.add_space(4.0);
                let shown = truncate_middle(&self.status, 92);
                let resp = ui.add(
                    Label::new(RichText::new(&shown).font(body(11.5)).color(text_color))
                        .truncate(),
                );
                if shown != self.status {
                    resp.on_hover_text(&self.status);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(self.document_meta())
                        .font(body(11.5))
                        .color(t.muted),
                );
            });
        });
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.top() + 0.5),
                egui::pos2(rect.right(), rect.top() + 0.5),
            ],
            Stroke::new(1.0, t.line),
        );
    }

    // ----------------------------------------------------------------- modals

    fn modals(&mut self, ctx: &egui::Context, a: &mut Vec<Action>) {
        self.metadata_modal(ctx);
        self.crop_modal(ctx, a);
        self.shortcuts_modal(ctx);
        self.about_modal(ctx);
        self.replace_modal(ctx, a);
    }

    fn metadata_modal(&mut self, ctx: &egui::Context) {
        if !self.show_meta {
            return;
        }
        let t = Tokens::for_mode(self.dark);
        let mut close = false;
        let mut save = false;
        let resp = egui::Modal::new(Id::new("metadata_modal"))
            .backdrop_color(t.scrim)
            .frame(modal_frame(t))
            .show(ctx, |ui| {
                ui.set_width(430.0);
                modal_header(
                    ui,
                    t,
                    "Document details",
                    "Written into the PDF when you export.",
                    &mut close,
                );
                ui.add_space(18.0);
                let width = ui.available_width();
                for (label, value) in [
                    ("Title", &mut self.meta_edit.title),
                    ("Author", &mut self.meta_edit.author),
                    ("Subject", &mut self.meta_edit.subject),
                    ("Keywords", &mut self.meta_edit.keywords),
                ] {
                    ui.label(RichText::new(label).font(medium(11.5)).color(t.muted));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .hint_text("Optional")
                            .desired_width(width)
                            .min_size(Vec2::new(0.0, 38.0)),
                    );
                    ui.add_space(10.0);
                }
                ui.add_space(8.0);
                hairline(ui, t);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if activated(
                            &Btn::primary("Save details")
                                .width(132.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            save = true;
                        }
                        ui.add_space(6.0);
                        if activated(
                            &Btn::secondary("Cancel")
                                .width(88.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
            });
        if close || resp.should_close() {
            self.show_meta = false;
        }
        if save {
            self.meta = self.meta_edit.clone();
            self.show_meta = false;
            self.set_status("Document details updated.", StatusKind::Success);
        }
    }

    fn crop_modal(&mut self, ctx: &egui::Context, a: &mut Vec<Action>) {
        if !self.show_crop {
            return;
        }
        let t = Tokens::for_mode(self.dark);
        let count = self.selected.len();
        if count == 0 {
            self.show_crop = false;
            return;
        }
        let mut close = false;
        let mut apply = false;
        let resp = egui::Modal::new(Id::new("crop_modal"))
            .backdrop_color(t.scrim)
            .frame(modal_frame(t))
            .show(ctx, |ui| {
                ui.set_width(584.0);
                modal_header(
                    ui,
                    t,
                    "Crop and scale",
                    &format!("Applies to {count} selected {}.", page_word(count)),
                    &mut close,
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    let preview_size = Vec2::new(190.0, 250.0);
                    self.crop_preview(ui, preview_size, t);
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.set_width(310.0);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        for (label, idx) in [
                            ("Left", 0),
                            ("Right", 1),
                            ("Top", 2),
                            ("Bottom", 3),
                        ] {
                            ui.add(
                                egui::Slider::new(&mut self.crop_edit[idx], 0.0..=45.0)
                                    .text(label)
                                    .suffix(" %")
                                    .fixed_decimals(0),
                            );
                        }
                        ui.add(
                            egui::Slider::new(&mut self.scale_edit, 10.0..=400.0)
                                .text("Scale")
                                .suffix(" %")
                                .fixed_decimals(0),
                        );
                        ui.add_space(2.0);
                        if activated(
                            &Btn::ghost("Reset crop and scale")
                                .height(28.0)
                                .font(body(11.5))
                                .show(ui, t),
                        ) {
                            self.crop_edit = [0.0; 4];
                            self.scale_edit = 100.0;
                        }
                        ui.add_space(4.0);
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "Margins are a percentage of each page. Scale is applied after cropping.",
                                )
                                .font(body(11.0))
                                .color(t.muted)
                                .line_height(Some(16.0)),
                            )
                            .wrap(),
                        );
                    });
                });
                ui.add_space(12.0);
                hairline(ui, t);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if activated(
                            &Btn::primary(&format!("Apply to {count} {}", page_word(count)))
                                .width(152.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            apply = true;
                        }
                        ui.add_space(6.0);
                        if activated(
                            &Btn::secondary("Cancel")
                                .width(88.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
            });
        if close || resp.should_close() {
            self.show_crop = false;
        }
        if apply {
            a.push(Action::ApplyCrop);
        }
    }

    fn crop_preview(&mut self, ui: &mut egui::Ui, size: Vec2, t: Tokens) {
        let Some(item) = self.selected.iter().next().and_then(|&i| self.pages.get(i)).cloned() else {
            return;
        };
        let thumb = item
            .source_id
            .map(|sid| self.ensure_thumb(sid, item.source_page_num));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(RADIUS_CARD), t.surface_hover);
        let inner = rect.shrink(18.0);
        let swap = item.rotation == 90 || item.rotation == 270;
        let (pw, ph) = if swap {
            (item.height_pt, item.width_pt)
        } else {
            (item.width_pt, item.height_pt)
        };
        let s = (inner.width() / pw).min(inner.height() / ph);
        let paper = Rect::from_center_size(inner.center(), Vec2::new(pw * s, ph * s));
        painter.add(
            t.paper_shadow()
                .as_shape(paper.translate(Vec2::new(0.0, 2.0)), RADIUS_PAPER),
        );
        painter.rect_filled(paper, CornerRadius::same(RADIUS_PAPER), t.paper);
        if let Some(Thumb::Ready { tex, w, h }) = &thumb {
            let unrot = if swap {
                Vec2::new(paper.width(), paper.height())
            } else {
                paper.size()
            };
            let img_rect = Rect::from_center_size(paper.center(), unrot);
            let angle = (item.rotation as f32).to_radians();
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(paper)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.set_clip_rect(paper.intersect(ui.clip_rect()));
                    egui::Image::new(egui::load::SizedTexture::new(
                        tex.id(),
                        Vec2::new(*w, *h),
                    ))
                    .rotate(angle, Vec2::splat(0.5))
                    .paint_at(ui, img_rect);
                },
            );
        }
        painter.rect_stroke(
            paper,
            CornerRadius::same(RADIUS_PAPER),
            Stroke::new(1.0, t.line),
            StrokeKind::Inside,
        );

        let [l, r, top, bottom] = self.crop_edit;
        let crop = Rect::from_min_max(
            egui::pos2(paper.left() + paper.width() * l / 100.0, paper.top() + paper.height() * top / 100.0),
            egui::pos2(
                paper.right() - paper.width() * r / 100.0,
                paper.bottom() - paper.height() * bottom / 100.0,
            ),
        );
        let shade = Color32::from_rgba_unmultiplied(12, 10, 7, 100);
        for dim in [
            Rect::from_min_max(paper.min, egui::pos2(paper.right(), crop.top())),
            Rect::from_min_max(egui::pos2(paper.left(), crop.bottom()), paper.max),
            Rect::from_min_max(egui::pos2(paper.left(), crop.top()), egui::pos2(crop.left(), crop.bottom())),
            Rect::from_min_max(egui::pos2(crop.right(), crop.top()), egui::pos2(paper.right(), crop.bottom())),
        ] {
            if dim.width() > 0.0 && dim.height() > 0.0 {
                painter.rect_filled(dim, 0.0, shade);
            }
        }
        painter.rect_stroke(
            crop,
            0.0,
            Stroke::new(1.5, t.accent),
            StrokeKind::Middle,
        );
        let handle = 7.0;
        for corner in [
            crop.left_top(),
            crop.right_top(),
            crop.left_bottom(),
            crop.right_bottom(),
        ] {
            painter.circle_filled(corner, handle * 0.5, t.surface);
            painter.circle_stroke(corner, handle * 0.5, Stroke::new(1.5, t.accent));
        }
        painter.text(
            egui::pos2(rect.center().x, rect.bottom() - 10.0),
            Align2::CENTER_CENTER,
            "Preview",
            body(10.5),
            t.muted,
        );
    }

    fn shortcuts_modal(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let t = Tokens::for_mode(self.dark);
        let mut close = false;
        let resp = egui::Modal::new(Id::new("shortcuts_modal"))
            .backdrop_color(t.scrim)
            .frame(modal_frame(t))
            .show(ctx, |ui| {
                ui.set_width(620.0);
                modal_header(
                    ui,
                    t,
                    "Keyboard shortcuts",
                    "Ctrl is Cmd on macOS. Shortcuts pause while you type in a field.",
                    &mut close,
                );
                ui.add_space(16.0);
                ui.columns(2, |cols| {
                    shortcut_group(&mut cols[0], t, "FILE");
                    shortcut_row(&mut cols[0], t, "Add pages", "Ctrl O");
                    shortcut_row(&mut cols[0], t, "Save PDF", "Ctrl S");
                    shortcut_row(&mut cols[0], t, "Open and replace", "Ctrl Shift O");
                    cols[0].add_space(14.0);
                    shortcut_group(&mut cols[0], t, "EDIT");
                    shortcut_row(&mut cols[0], t, "Undo", "Ctrl Z");
                    shortcut_row(&mut cols[0], t, "Redo", "Ctrl Shift Z");
                    shortcut_row(&mut cols[0], t, "Copy", "Ctrl C");
                    shortcut_row(&mut cols[0], t, "Cut", "Ctrl X");
                    shortcut_row(&mut cols[0], t, "Paste after", "Ctrl V");
                    shortcut_row(&mut cols[0], t, "Duplicate", "Ctrl D");
                    shortcut_row(&mut cols[0], t, "Delete", "Del");

                    shortcut_group(&mut cols[1], t, "PAGES");
                    shortcut_row(&mut cols[1], t, "Select all", "Ctrl A");
                    shortcut_row(&mut cols[1], t, "Deselect all", "Esc");
                    shortcut_row(&mut cols[1], t, "Move left", "Alt Left");
                    shortcut_row(&mut cols[1], t, "Move right", "Alt Right");
                    cols[1].add_space(14.0);
                    shortcut_group(&mut cols[1], t, "VIEW");
                    shortcut_row(&mut cols[1], t, "Zoom in", "Ctrl +");
                    shortcut_row(&mut cols[1], t, "Zoom out", "Ctrl -");
                    shortcut_row(&mut cols[1], t, "Actual size", "Ctrl 0");
                    shortcut_row(&mut cols[1], t, "Toggle sidebar", "Ctrl B");
                    shortcut_row(&mut cols[1], t, "Switch theme", "Ctrl Shift L");
                    shortcut_row(&mut cols[1], t, "Shortcuts", "F1");
                    shortcut_row(&mut cols[1], t, "Quit", "Ctrl Q");
                });
                ui.add_space(12.0);
                hairline(ui, t);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if activated(
                            &Btn::secondary("Close")
                                .width(92.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
            });
        if close || resp.should_close() {
            self.show_shortcuts = false;
        }
    }

    fn about_modal(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let t = Tokens::for_mode(self.dark);
        let mut close = false;
        let resp = egui::Modal::new(Id::new("about_modal"))
            .backdrop_color(t.scrim)
            .frame(modal_frame(t))
            .show(ctx, |ui| {
                ui.set_width(410.0);
                ui.horizontal(|ui| {
                    logo_mark(ui, t, Vec2::new(42.0, 50.0));
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("PDF Wizard")
                                .font(serif_font(32.0))
                                .color(t.ink),
                        );
                        ui.add_space(1.0);
                        ui.label(
                            RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION")))
                                .font(mono(10.5))
                                .color(t.muted),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if activated(
                            &Btn::icon(icons::CLOSE).tip("Close").show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
                ui.add_space(16.0);
                ui.add(
                    Label::new(
                        RichText::new(
                            "A quiet workbench for arranging pages. Open PDFs and images, put every page in order, then export a clean PDF or booklet.",
                        )
                        .font(body(13.5))
                        .color(t.muted)
                        .line_height(Some(20.0)),
                    )
                    .wrap(),
                );
                ui.add_space(16.0);
                hairline(ui, t);
                ui.add_space(14.0);
                ui.label(
                    RichText::new("Renders with hayro. Exports with lopdf. Built with egui.")
                        .font(body(11.5))
                        .color(t.muted),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new("MIT licensed. Instrument Sans and Newsreader under the SIL Open Font License; Lucide icons under ISC.")
                        .font(body(11.0))
                        .color(t.muted)
                        .line_height(Some(16.0)),
                );
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if activated(
                            &Btn::secondary("Close")
                                .width(92.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
            });
        if close || resp.should_close() {
            self.show_about = false;
        }
    }

    fn replace_modal(&mut self, ctx: &egui::Context, a: &mut Vec<Action>) {
        if !self.show_replace {
            return;
        }
        let t = Tokens::for_mode(self.dark);
        let mut close = false;
        let mut confirm = false;
        let pending = self.pending_replace.clone().unwrap_or_default();
        let resp = egui::Modal::new(Id::new("replace_modal"))
            .backdrop_color(t.scrim)
            .frame(modal_frame(t))
            .show(ctx, |ui| {
                ui.set_width(442.0);
                modal_header(
                    ui,
                    t,
                    "Replace this document?",
                    "The pages on the bench will be cleared. Undo history and copied pages go with them.",
                    &mut close,
                );
                ui.add_space(16.0);
                ui.label(
                    RichText::new(format!(
                        "Currently {} · {} {}.",
                        truncate_middle(&self.document_name(), 34),
                        self.pages.len(),
                        page_word(self.pages.len())
                    ))
                    .font(body(12.0))
                    .color(t.muted),
                );
                ui.add_space(10.0);
                Frame::new()
                    .fill(t.surface_hover)
                    .corner_radius(CornerRadius::same(RADIUS_CONTROL))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for path in pending.iter().take(4) {
                            let name = path
                                .file_name()
                                .map_or("file".to_string(), |n| n.to_string_lossy().to_string());
                            ui.label(
                                RichText::new(truncate_middle(&name, 46))
                                    .font(mono(10.5))
                                    .color(t.ink),
                            );
                        }
                        if pending.len() > 4 {
                            ui.label(
                                RichText::new(format!("and {} more", pending.len() - 4))
                                    .font(body(11.0))
                                    .color(t.muted),
                            );
                        }
                    });
                ui.add_space(16.0);
                hairline(ui, t);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if activated(
                            &Btn::danger_filled("Replace pages")
                                .width(144.0)
                                .height(40.0)
                                .enabled(!pending.is_empty())
                                .show(ui, t),
                        ) {
                            confirm = true;
                        }
                        ui.add_space(6.0);
                        if activated(
                            &Btn::secondary("Cancel")
                                .width(88.0)
                                .height(40.0)
                                .show(ui, t),
                        ) {
                            close = true;
                        }
                    });
                });
            });
        if close || resp.should_close() {
            self.pending_replace = None;
            self.show_replace = false;
        }
        if confirm {
            a.push(Action::ConfirmReplace);
        }
    }

    fn draw_drop_overlay(&self, ctx: &egui::Context, t: Tokens) {
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(LayerId::new(Order::Middle, Id::new("drop_overlay")));
        painter.rect_filled(screen, 0, Color32::from_rgba_unmultiplied(14, 12, 8, 96));
        let card = Rect::from_center_size(screen.center(), Vec2::new(368.0, 184.0));
        painter.add(
            t.modal_shadow()
                .as_shape(card, CornerRadius::same(RADIUS_MODAL)),
        );
        painter.rect_filled(card, CornerRadius::same(RADIUS_MODAL), t.surface);
        painter.rect_stroke(
            card,
            CornerRadius::same(RADIUS_MODAL),
            Stroke::new(1.5, t.accent),
            StrokeKind::Inside,
        );
        painter.text(
            egui::pos2(card.center().x, card.top() + 48.0),
            Align2::CENTER_CENTER,
            icons::UPLOAD,
            icon_font(30.0),
            t.accent,
        );
        painter.text(
            egui::pos2(card.center().x, card.top() + 92.0),
            Align2::CENTER_CENTER,
            "Drop to add pages",
            serif_font(24.0),
            t.ink,
        );
        painter.text(
            egui::pos2(card.center().x, card.top() + 123.0),
            Align2::CENTER_CENTER,
            "PDF, PNG, JPEG, or WebP",
            body(12.0),
            t.muted,
        );
    }

    fn sync_title(&mut self, ctx: &egui::Context) {
        let want = if self.pages.is_empty() {
            "PDF Wizard".to_string()
        } else {
            format!(
                "{} · {} {} · PDF Wizard",
                truncate_middle(&self.document_name(), 40),
                self.pages.len(),
                page_word(self.pages.len())
            )
        };
        if want != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.window_title = want;
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_bg(&ctx);
        self.poll_thumbs(&ctx);
        let mut actions: Vec<Action> = Vec::new();
        let modal_open = self.modal_open();
        if !modal_open {
            self.handle_input(&ctx, &mut actions);
        }
        let t = Tokens::for_mode(self.dark);

        egui::Panel::top("header")
            .exact_size(HEADER_H)
            .show_separator_line(false)
            .frame(Frame::new().fill(t.header))
            .show(ui, |ui| self.header(ui, &mut actions));

        egui::Panel::bottom("status")
            .exact_size(STATUS_H)
            .show_separator_line(false)
            .frame(Frame::new().fill(t.ground))
            .show(ui, |ui| self.status_bar(ui));

        let mut sidebar_open = self.show_sidebar;
        let sidebar = egui::Panel::left("bench")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .show_separator_line(true)
            .frame(
                Frame::new()
                    .fill(t.panel)
                    .inner_margin(Margin::symmetric(20, 16)),
            )
            .show_collapsible(ui, &mut sidebar_open, |ui| {
                self.sidebar(ui, &mut actions);
            });
        drop(sidebar);
        self.show_sidebar = sidebar_open;

        egui::CentralPanel::default()
            .frame(Frame::new().fill(t.canvas))
            .show(ui, |ui| self.canvas(ui, &mut actions));

        self.selection_tray(&ctx, &mut actions);
        self.modals(&ctx, &mut actions);

        let file_hover = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if file_hover && !self.modal_open() && self.busy.is_none() {
            self.draw_drop_overlay(&ctx, t);
        }

        let had_actions = !actions.is_empty();
        for action in actions {
            self.run(action, &ctx);
        }
        self.sync_title(&ctx);
        if had_actions {
            ctx.request_repaint();
        }
    }
}

fn tray_icon(
    ui: &mut egui::Ui,
    t: Tokens,
    icon: &str,
    tip: &str,
    a: &mut Vec<Action>,
    action: Action,
) {
    if activated(&Btn::icon(icon).tip(tip).show(ui, t)) {
        a.push(action);
    }
}

fn shortcut_group(ui: &mut egui::Ui, t: Tokens, label: &str) {
    section_label(ui, t, label);
    ui.add_space(5.0);
}

fn shortcut_row(ui: &mut egui::Ui, t: Tokens, label: &str, keys: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(body(12.0)).color(t.muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            key_chip(ui, t, keys);
        });
    });
}

impl eframe::App for PdfWizardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_columns_min_one() {
        assert_eq!(grid_columns(50.0, 200.0, 10.0), 1);
        assert_eq!(grid_columns(0.0, 200.0, 10.0), 1);
    }

    #[test]
    fn grid_columns_fits_width() {
        assert_eq!(grid_columns(1000.0, 200.0, 10.0), 4);
    }

    #[test]
    fn range_select_is_order_independent() {
        assert_eq!(apply_range_select(2, 5), vec![2, 3, 4, 5]);
        assert_eq!(apply_range_select(5, 2), vec![2, 3, 4, 5]);
        assert_eq!(apply_range_select(3, 3), vec![3]);
    }

    #[test]
    fn shift_click_after_delete_never_selects_oob_or_panics() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (0..10).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.commit(pages);
        app.handle_card_click(8, false, false);
        app.selected = (4..10).collect();
        app.delete_selected();
        assert_eq!(app.pages.len(), 4);
        app.handle_card_click(1, true, false);
        assert!(app.selected.iter().all(|&idx| idx < app.pages.len()));
        app.rotate_selected(90);
    }

    #[test]
    fn ctrl_click_toggles_selection_without_a_mode() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (0..4).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.commit(pages);
        app.handle_card_click(0, false, false);
        app.handle_card_click(2, false, true);
        assert_eq!(app.sel_vec(), vec![0, 2]);
        app.handle_card_click(0, false, true);
        assert_eq!(app.sel_vec(), vec![2]);
    }

    #[test]
    fn empty_selection_actions_set_status_message() {
        let mut app = PdfWizardApp::empty();
        app.commit(vec![pdf_engine::blank_page(1, A4_W, A4_H)]);
        let ctx = egui::Context::default();
        for act in [Action::Delete, Action::Cut, Action::Copy] {
            app.status.clear();
            app.run(act, &ctx);
            assert!(app.status.starts_with("Select pages to"));
        }
        assert_eq!(app.pages.len(), 1);
    }

    #[test]
    fn ctrl_v_pastes_internal_clipboard_without_os_paste_event() {
        let mut app = PdfWizardApp::empty();
        app.commit(vec![pdf_engine::blank_page(1, A4_W, A4_H)]);
        app.select_only(0);
        let ctx = egui::Context::default();
        app.run(Action::Copy, &ctx);
        assert_eq!(app.clipboard.len(), 1);

        let mut actions = Vec::new();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: Key::V,
            physical_key: Some(Key::V),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        let mut out = ctx.run_ui(raw, |ui| {
            app.handle_input(ui.ctx(), &mut actions);
        });
        out.textures_delta.clear();
        assert!(actions.iter().any(|a| matches!(a, Action::PasteAfter)));
    }

    #[test]
    fn open_replace_prunes_stale_sources_and_thumbs() {
        let mut app = PdfWizardApp::empty();
        let dir = std::env::temp_dir().join(format!("pdfwiz_app_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("one.pdf");
        let f2 = dir.join("two.pdf");
        std::fs::write(&f1, pdf_engine::testutil::make_pdf(2)).unwrap();
        std::fs::write(&f2, pdf_engine::testutil::make_pdf(1)).unwrap();

        app.add_loaded(&[f1.clone()], true);
        assert_eq!(app.sources.len(), 1);
        app.thumbs.insert((0, 1), Thumb::Loading);

        app.add_loaded(&[f2], true);
        assert_eq!(app.sources.len(), 1);
        assert!(!app.sources.contains_key(&0));
        assert!(!app.thumbs.contains_key(&(0, 1)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_replace_clears_clipboard() {
        let mut app = PdfWizardApp::empty();
        let dir = std::env::temp_dir().join(format!("pdfwiz_clip_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f1 = dir.join("one.pdf");
        let f2 = dir.join("two.pdf");
        std::fs::write(&f1, pdf_engine::testutil::make_pdf(1)).unwrap();
        std::fs::write(&f2, pdf_engine::testutil::make_pdf(1)).unwrap();

        app.add_loaded(&[f1], true);
        app.select_only(0);
        let ctx = egui::Context::default();
        app.run(Action::Copy, &ctx);
        assert_eq!(app.clipboard.len(), 1);

        app.add_loaded(&[f2], true);
        assert!(app.clipboard.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn step_move_selected_updates_pages_and_selection() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (1..=3).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.commit(pages);
        app.select_only(1);
        let ctx = egui::Context::default();
        app.run(Action::StepLeft, &ctx);
        assert_eq!(
            app.pages.iter().map(|p| p.uid).collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
        assert_eq!(app.sel_vec(), vec![0]);

        app.run(Action::StepRight, &ctx);
        assert_eq!(
            app.pages.iter().map(|p| p.uid).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(app.sel_vec(), vec![1]);
    }

    #[test]
    fn step_move_at_start_does_not_push_history() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (1..=3).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.pages = pages.clone();
        app.history = History::new(pages);
        app.select_only(0);
        let ctx = egui::Context::default();
        app.run(Action::StepLeft, &ctx);
        assert!(app.history.undo().is_none());
    }

    #[test]
    fn card_header_text_compacts_at_low_zoom() {
        let mut item = pdf_engine::blank_page(1, A4_W, A4_H);
        item.rotate(90);
        item.crop.left = 0.1;
        assert_eq!(card_header_text(0, &item, 80.0), "#1");
        assert_eq!(card_header_text(0, &item, 190.0), "#1 · R90 · Crop");
    }

    #[test]
    fn touch_targets_meet_minimum() {
        const { assert!(CARD_BTN_H >= 36.0) };
        const { assert!(TOUCH_MIN >= 36.0) };
        assert!(TRAY_H >= 44.0);
    }

    #[test]
    fn sidebar_toggles_and_modal_flags_are_exclusive() {
        let mut app = PdfWizardApp::empty();
        let ctx = egui::Context::default();
        assert!(app.show_sidebar);
        app.run(Action::ToggleSidebar, &ctx);
        assert!(!app.show_sidebar);
        app.run(Action::ToggleSidebar, &ctx);
        assert!(app.show_sidebar);

        app.show_about = true;
        app.run(Action::ShowShortcuts, &ctx);
        assert!(app.show_shortcuts);
        assert!(!app.show_about);
    }

    #[test]
    fn suggested_name_uses_source_stem() {
        let mut app = PdfWizardApp::empty();
        let dir = std::env::temp_dir().join(format!("pdfwiz_name_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Quarterly Report.pdf");
        std::fs::write(&file, pdf_engine::testutil::make_pdf(2)).unwrap();
        app.add_loaded(&[file], true);
        assert_eq!(app.suggested_name("arranged"), "Quarterly-Report-arranged.pdf");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failed_load_keeps_existing_pages_and_reports_error() {
        let mut app = PdfWizardApp::empty();
        app.commit(vec![pdf_engine::blank_page(1, A4_W, A4_H)]);
        let dir = std::env::temp_dir().join(format!("pdfwiz_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("broken.pdf");
        std::fs::write(&bad, b"not a pdf").unwrap();
        app.add_loaded(&[bad], false);
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.status_kind, StatusKind::Error);
        std::fs::remove_dir_all(&dir).ok();
    }
}
