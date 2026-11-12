use crate::arrange;
use crate::history::History;
use crate::model::{CropMargins, Metadata, OverlaySpec, PageItem, SourceId};
use crate::pdf_engine::{self, SourceDocument, A4_H, A4_W};
use crate::renderer::Renderer;
use crate::theme::{self, AntTokens, BtnKind, TOUCH_HEIGHT};
use eframe::egui::{self, Key, Modifiers, Rect, Sense, Vec2};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

const THUMB_PX: u32 = 360;
const SPACING: f32 = 16.0;
const CARD_HEADER_H: f32 = 40.0;
const CARD_BTN_H: f32 = 36.0;

fn card_header_text(index: usize, item: &PageItem, card_width: f32) -> String {
    if card_width < 130.0 {
        return format!("#{}", index + 1);
    }
    let mut tags = String::new();
    if item.rotation != 0 {
        tags += &format!(" R{}", item.rotation);
    }
    if item.crop != CropMargins::default() {
        tags += " crop";
    }
    if (item.scale - 1.0).abs() > f32::EPSILON {
        tags += &format!(" {:.0}%", item.scale * 100.0);
    }
    if !item.overlays.is_empty() {
        tags += " ovl";
    }
    format!("#{}{}", index + 1, tags)
}

/// Configures generous spacing, hit targets, and scrollbars for touchscreen use.
#[cfg(test)]
pub fn apply_touch_style(ctx: &egui::Context) {
    theme::apply_ant_theme(ctx, true);
}

/// Number of thumbnail columns that fit in `avail_width` (at least 1).
pub fn grid_columns(avail_width: f32, thumb: f32, spacing: f32) -> usize {
    (((avail_width + spacing) / (thumb + spacing)).floor() as usize).max(1)
}

/// Inclusive, ascending range of indices between `anchor` and `clicked`.
pub fn apply_range_select(anchor: usize, clicked: usize) -> Vec<usize> {
    (anchor.min(clicked)..=anchor.max(clicked)).collect()
}

#[derive(Clone)]
enum Thumb {
    Loading,
    Failed,
    Ready { tex: egui::TextureHandle, w: f32, h: f32 },
}

#[derive(Clone)]
enum Action {
    Open,
    Append,
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
    Reverse,
    SwapOddEven,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTheme,
    StepLeft,
    StepRight,
    ToggleSelectMode,
    ToggleReorderMode,
    ToggleTools,
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
    select_mode: bool,
    reorder_mode: bool,
    status: String,
    meta: Metadata,
    show_meta: bool,
    show_crop: bool,
    show_tools: bool,
    crop_edit: [f32; 4],
    scale_edit: f32,
    dragging: bool,
    drop_target: Option<usize>,
    busy: Option<String>,
    bg_rx: Option<std::sync::mpsc::Receiver<BgResult>>,
}

fn is_pdf(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

enum BgResult {
    Loaded {
        sources: HashMap<SourceId, SourceDocument>,
        items: Vec<PageItem>,
        next_source: SourceId,
        next_uid: u64,
        status: String,
        replace: bool,
    },
    Message(String),
}

fn load_paths_into(
    paths: &[PathBuf],
    next_source: &mut SourceId,
    next_uid: &mut u64,
) -> (HashMap<SourceId, SourceDocument>, Vec<PageItem>, String) {
    let mut new_sources = HashMap::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path.file_name().map_or("file".to_string(), |n| n.to_string_lossy().to_string());
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
                    items.push(PageItem::from_source(*next_uid, id, n, w, h, format!("{name} - p.{n}")));
                }
                new_sources.insert(id, src);
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    let status = if errors.is_empty() {
        format!("Loaded {} page(s)", items.len())
    } else {
        format!("Error: {}", errors.join("; "))
    };
    (new_sources, items, status)
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
            zoom: 190.0,
            dark: true,
            select_mode: false,
            reorder_mode: false,
            status: "Ready. Tap Open or drop PDF files to begin.".into(),
            meta: Metadata::default(),
            show_meta: false,
            show_crop: false,
            show_tools: false,
            crop_edit: [0.0; 4],
            scale_edit: 100.0,
            dragging: false,
            drop_target: None,
            busy: None,
            bg_rx: None,
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::empty();
        theme::apply_ant_theme(&cc.egui_ctx, app.dark);
        let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
        if !args.is_empty() {
            let loaded = app.load_paths(&args);
            if !loaded.is_empty() {
                app.commit(loaded);
            }
        }
        app
    }

    fn uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
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
        self.selected.iter().filter_map(|&i| self.pages.get(i).cloned()).collect()
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
        match (shift, command || self.select_mode, self.anchor) {
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

    /// Loads PDFs and images; never changes workspace state on failure. Errors go to the status bar.
    fn load_paths(&mut self, paths: &[PathBuf]) -> Vec<PageItem> {
        let (new_sources, items, status) =
            load_paths_into(paths, &mut self.next_source, &mut self.next_uid);
        self.sources.extend(new_sources);
        self.status = status;
        items
    }

    fn ensure_thumb(&mut self, sid: SourceId, page: u32) -> Thumb {
        if let Some(t) = self.thumbs.get(&(sid, page)) {
            return t.clone();
        }
        if let Some(src) = self.sources.get(&sid) {
            self.renderer.request(sid, src.raw_bytes.clone(), page, THUMB_PX);
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
                    let ci = egui::ColorImage::from_rgba_premultiplied([img.width, img.height], &img.rgba);
                    let tex = ctx.load_texture(
                        format!("thumb-{}-{}", r.source_id, r.page_num),
                        ci,
                        egui::TextureOptions::LINEAR,
                    );
                    Thumb::Ready { tex, w: img.width as f32, h: img.height as f32 }
                }
                None => Thumb::Failed,
            };
            self.thumbs.insert((r.source_id, r.page_num), thumb);
        }
        if self.thumbs.values().any(|t| matches!(t, Thumb::Loading)) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
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
            let table = [
                (cmd, Key::A, Action::SelectAll),
                (cmd, Key::C, Action::Copy),
                (cmd, Key::X, Action::Cut),
                (cmd, Key::V, Action::PasteAfter),
                (cmd, Key::D, Action::Duplicate),
                (cmd, Key::O, Action::Open),
                (cmd, Key::S, Action::Save),
                (Modifiers::NONE, Key::Delete, Action::Delete),
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
                    egui::Event::Paste(_) if !actions.iter().any(|a| matches!(a, Action::PasteAfter)) => {
                        actions.push(Action::PasteAfter);
                    }
                    _ => {}
                }
            }
        });
    }

    // ---------------------------------------------------------------- actions

    fn run(&mut self, action: Action, ctx: &egui::Context) {
        let has_sel = !self.selected.is_empty();
        match action {
            Action::Open | Action::Append => {
                if self.busy.is_some() {
                    self.status = "Busy, please wait...".into();
                    return;
                }
                let replace = matches!(action, Action::Open);
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("PDF and images", &["pdf", "png", "jpg", "jpeg", "webp"])
                    .pick_files()
                {
                    self.start_load_bg(paths, replace);
                }
            }
            Action::ImportImages => {
                if self.busy.is_some() {
                    self.status = "Busy, please wait...".into();
                    return;
                }
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                    .pick_files()
                {
                    self.start_load_bg(paths, false);
                }
            }
            Action::DropFiles(paths) => {
                if self.busy.is_some() {
                    self.status = "Busy, please wait...".into();
                    return;
                }
                let ok: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| {
                        is_pdf(p)
                            || p.extension().is_some_and(|e| {
                                ["png", "jpg", "jpeg", "webp"].iter().any(|x| e.eq_ignore_ascii_case(x))
                            })
                    })
                    .collect();
                if ok.is_empty() {
                    self.status = "Dropped files are not PDF or image files".into();
                } else {
                    self.start_load_bg(ok, false);
                }
            }
            Action::Save => {
                if self.pages.is_empty() {
                    self.status = "Nothing to save".into();
                } else {
                    let pages = self.pages.clone();
                    self.save_pdf_dialog(pages, "output.pdf", false);
                }
            }
            Action::ExtractSingle => {
                if !has_sel {
                    self.status = "No pages selected".into();
                } else {
                    let pages = self.selected_items();
                    self.save_pdf_dialog(pages, "extracted.pdf", false);
                }
            }
            Action::ExtractSeparate => {
                if !has_sel {
                    self.status = "No pages selected".into();
                } else {
                    let pages = self.selected_items();
                    self.extract_to_folder(pages);
                }
            }
            Action::SplitAll => {
                if self.pages.is_empty() {
                    self.status = "Nothing to split".into();
                } else {
                    let pages = self.pages.clone();
                    self.extract_to_folder(pages);
                }
            }
            Action::Booklet => {
                let pages = if has_sel { self.selected_items() } else { self.pages.clone() };
                if pages.is_empty() {
                    self.status = "Nothing to export".into();
                } else {
                    self.save_pdf_dialog(pages, "booklet.pdf", true);
                }
            }
            Action::Metadata => self.show_meta = true,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::Undo => match self.history.undo() {
                Some(p) => self.restore(p, "Undo"),
                None => self.status = "Nothing to undo".into(),
            },
            Action::Redo => match self.history.redo() {
                Some(p) => self.restore(p, "Redo"),
                None => self.status = "Nothing to redo".into(),
            },
            Action::Copy => {
                if has_sel {
                    self.clipboard = self.selected_items();
                    ctx.copy_text("pdf-wizard-pages".into());
                    self.status = format!("Copied {} page(s)", self.clipboard.len());
                } else {
                    self.status = "No pages selected".into();
                }
            }
            Action::Cut => {
                if has_sel {
                    self.clipboard = self.selected_items();
                    ctx.copy_text("pdf-wizard-pages".into());
                    self.delete_selected();
                } else {
                    self.status = "No pages selected".into();
                }
            }
            Action::PasteAfter => {
                if self.clipboard.is_empty() {
                    self.status = "Clipboard is empty".into();
                } else {
                    let at = self.selected.iter().next_back().map_or(self.pages.len(), |&i| i + 1).min(self.pages.len());
                    let clones: Vec<PageItem> = self.clipboard.clone().into_iter().map(|p| self.renumber(p)).collect();
                    let mut pages = self.pages.clone();
                    let n = clones.len();
                    for (k, p) in clones.into_iter().enumerate() {
                        pages.insert(at + k, p);
                    }
                    self.commit(pages);
                    self.selected = (at..at + n).collect();
                    self.status = format!("Pasted {n} page(s)");
                }
            }
            Action::PasteOverlay | Action::PasteUnderlay => {
                let under = matches!(action, Action::PasteUnderlay);
                let src = self.clipboard.first().and_then(|p| p.source_id.map(|s| (s, p.source_page_num)));
                match (src, has_sel) {
                    (Some((sid, num)), true) => {
                        let mut pages = self.pages.clone();
                        for &i in &self.selected {
                            if let Some(p) = pages.get_mut(i) {
                                p.overlays.push(OverlaySpec { source_id: sid, page_number: num, is_underlay: under });
                            }
                        }
                        self.commit(pages);
                        self.status = "Overlay applied".into();
                    }
                    _ => self.status = "Copy a PDF page and select target pages first".into(),
                }
            }
            Action::Duplicate => {
                if has_sel {
                    let mut pages = Vec::new();
                    let mut new_sel = BTreeSet::new();
                    for (i, p) in self.pages.clone().into_iter().enumerate() {
                        let dup = self.selected.contains(&i).then(|| self.renumber(p.clone()));
                        pages.push(p);
                        if let Some(d) = dup {
                            new_sel.insert(pages.len());
                            pages.push(d);
                        }
                    }
                    self.commit(pages);
                    self.selected = new_sel;
                    self.status = "Duplicated".into();
                } else {
                    self.status = "No pages selected".into();
                }
            }
            Action::Delete => {
                if has_sel {
                    self.delete_selected();
                } else {
                    self.status = "No pages selected".into();
                }
            }
            Action::InsertBlank => {
                let at = self.selected.iter().next_back().map_or(self.pages.len(), |&i| i + 1);
                let (w, h) = self
                    .pages
                    .get(at.saturating_sub(1))
                    .map_or((A4_W, A4_H), |p| (p.width_pt, p.height_pt));
                let blank = pdf_engine::blank_page(self.uid(), w, h);
                let mut pages = self.pages.clone();
                pages.insert(at, blank);
                self.commit(pages);
                self.select_only(at);
                self.status = "Inserted blank page".into();
            }
            Action::SelectAll => self.selected = (0..self.pages.len()).collect(),
            Action::SelectNone => self.selected.clear(),
            Action::Invert => {
                self.selected = arrange::invert_selection(self.pages.len(), &self.sel_vec()).into_iter().collect()
            }
            Action::SelectOdd => self.selected = arrange::select_odd(&self.pages).into_iter().collect(),
            Action::SelectEven => self.selected = arrange::select_even(&self.pages).into_iter().collect(),
            Action::SelectSame => {
                let ids: Vec<Option<SourceId>> = self.selected_items().iter().map(|p| p.source_id).collect();
                self.selected = self
                    .pages
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| ids.contains(&p.source_id))
                    .map(|(i, _)| i)
                    .collect();
            }
            Action::RotateLeft => self.rotate_selected(-90),
            Action::RotateRight => self.rotate_selected(90),
            Action::Rotate180 => self.rotate_selected(180),
            Action::CropDialog => {
                if let Some(p) = self.selected.iter().next().and_then(|&i| self.pages.get(i)) {
                    self.crop_edit = [p.crop.left * 100.0, p.crop.right * 100.0, p.crop.top * 100.0, p.crop.bottom * 100.0];
                    self.scale_edit = p.scale * 100.0;
                    self.show_crop = true;
                } else {
                    self.status = "Select pages to crop or scale".into();
                }
            }
            Action::Reverse => {
                let mut pages = self.pages.clone();
                if self.selected.len() >= 2 {
                    let idx: Vec<usize> = self.sel_vec().into_iter().filter(|&i| i < pages.len()).collect();
                    let mut chosen: Vec<PageItem> = idx.iter().map(|&i| pages[i].clone()).collect();
                    chosen.reverse();
                    for (k, &i) in idx.iter().enumerate() {
                        pages[i] = chosen[k].clone();
                    }
                } else {
                    arrange::reverse(&mut pages);
                    self.selected.clear();
                }
                self.commit(pages);
            }
            Action::SwapOddEven => {
                let mut pages = self.pages.clone();
                arrange::swap_odd_even(&mut pages);
                self.commit(pages);
                self.selected.clear();
            }
            Action::ZoomIn => self.zoom = (self.zoom * 1.2).min(400.0),
            Action::ZoomOut => self.zoom = (self.zoom / 1.2).max(80.0),
            Action::ZoomReset => self.zoom = 190.0,
            Action::ToggleTheme => {
                self.dark = !self.dark;
                theme::apply_ant_theme(ctx, self.dark);
            }
            Action::StepLeft => self.step_move_selected(-1),
            Action::StepRight => self.step_move_selected(1),
            Action::ToggleSelectMode => {
                self.select_mode = !self.select_mode;
                self.status = if self.select_mode {
                    "Select mode ON: tap cards to multi-select".into()
                } else {
                    "Select mode OFF".into()
                };
            }
            Action::ToggleReorderMode => {
                self.reorder_mode = !self.reorder_mode;
                self.status = if self.reorder_mode {
                    "Reorder mode ON: drag any card to move".into()
                } else {
                    "Reorder mode OFF: drag ⋮⋮ handle to move, swipe cards to scroll".into()
                };
            }
            Action::ToggleTools => {
                self.show_tools = !self.show_tools;
            }
        }
    }

    fn renumber(&mut self, mut p: PageItem) -> PageItem {
        p.uid = self.uid();
        p
    }

    #[cfg(test)]
    fn add_loaded(&mut self, paths: &[PathBuf], replace: bool) {
        let loaded = self.load_paths(paths);
        let status = self.status.clone();
        self.apply_loaded(HashMap::new(), loaded, status, replace);
    }

    fn apply_loaded(
        &mut self,
        new_sources: HashMap<SourceId, SourceDocument>,
        loaded: Vec<PageItem>,
        status: String,
        replace: bool,
    ) {
        self.sources.extend(new_sources);
        self.status = status;
        if loaded.is_empty() {
            return;
        }
        if replace {
            self.pages = loaded;
            self.history = History::new(self.pages.clone());
            self.selected.clear();
            self.anchor = None;
            self.clipboard.clear();
            let keep: std::collections::HashSet<SourceId> =
                self.pages.iter().filter_map(|p| p.source_id).collect();
            self.sources.retain(|id, _| keep.contains(id));
            self.thumbs.retain(|(id, _), _| keep.contains(id));
            self.renderer.retain_sources(keep);
        } else {
            let mut pages = self.pages.clone();
            pages.extend(loaded);
            self.commit(pages);
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
        self.spawn_bg("Loading files...", move || {
            let (sources, items, status) = load_paths_into(&paths, &mut next_source, &mut next_uid);
            BgResult::Loaded { sources, items, next_source, next_uid, status, replace }
        });
    }

    fn poll_bg(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.bg_rx.as_ref() else { return };
        match rx.try_recv() {
            Ok(res) => {
                self.busy = None;
                self.bg_rx = None;
                match res {
                    BgResult::Loaded { sources, items, next_source, next_uid, status, replace } => {
                        self.next_source = self.next_source.max(next_source);
                        self.next_uid = self.next_uid.max(next_uid);
                        self.apply_loaded(sources, items, status, replace);
                    }
                    BgResult::Message(msg) => self.status = msg,
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.busy = None;
                self.bg_rx = None;
            }
        }
    }

    fn restore(&mut self, pages: Vec<PageItem>, msg: &str) {
        self.pages = pages;
        self.selected.clear();
        self.anchor = None;
        self.status = msg.into();
    }

    fn delete_selected(&mut self) {
        let pages: Vec<PageItem> = self
            .pages
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.selected.contains(i))
            .map(|(_, p)| p.clone())
            .collect();
        let n = self.selected.len();
        self.selected.clear();
        self.anchor = None;
        self.commit(pages);
        self.status = format!("Removed {n} page(s)");
    }

    fn rotate_selected(&mut self, delta: i32) {
        if self.selected.is_empty() {
            self.status = "No pages selected".into();
            return;
        }
        let mut pages = self.pages.clone();
        for &i in &self.selected {
            if let Some(p) = pages.get_mut(i) {
                p.rotate(delta);
            }
        }
        self.commit(pages);
    }

    fn save_pdf_dialog(&mut self, pages: Vec<PageItem>, default_name: &str, booklet: bool) {
        if self.busy.is_some() {
            self.status = "Busy, please wait...".into();
            return;
        }
        let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(default_name).save_file()
        else {
            return;
        };
        let sources = self.sources.clone();
        let meta = self.meta.clone();
        self.spawn_bg("Exporting PDF...", move || {
            let result = if booklet {
                pdf_engine::export_booklet(&pages, &sources, &meta)
            } else {
                pdf_engine::export_pages(&pages, &sources, &meta)
            };
            let msg = match result.and_then(|b| std::fs::write(&path, b).map_err(Into::into)) {
                Ok(()) => format!("Saved {}", path.display()),
                Err(e) => format!("Error: {e}"),
            };
            BgResult::Message(msg)
        });
    }

    fn extract_to_folder(&mut self, pages: Vec<PageItem>) {
        if self.busy.is_some() {
            self.status = "Busy, please wait...".into();
            return;
        }
        let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
        let sources = self.sources.clone();
        let meta = self.meta.clone();
        self.spawn_bg("Extracting pages...", move || {
            let msg = match pdf_engine::extract_separate(&pages, &sources, &dir, &meta) {
                Ok(files) => format!("Wrote {} file(s) to {}", files.len(), dir.display()),
                Err(e) => format!("Error: {e}"),
            };
            BgResult::Message(msg)
        });
    }

    fn apply_crop(&mut self) {
        let [l, r, t, b] = self.crop_edit.map(|v| v / 100.0);
        let crop = CropMargins { left: l, right: r, top: t, bottom: b };
        let mut pages = self.pages.clone();
        for &i in &self.selected {
            if let Some(p) = pages.get_mut(i) {
                p.crop = crop;
                p.scale = self.scale_edit / 100.0;
            }
        }
        self.commit(pages);
        self.status = "Crop and scale applied".into();
    }

    fn move_selected_to(&mut self, target: usize) {
        let sel = self.sel_vec();
        let mut pages = self.pages.clone();
        arrange::move_pages(&mut pages, &sel, target);
        let unchanged = pages.iter().map(|p| p.uid).eq(self.pages.iter().map(|p| p.uid));
        if unchanged {
            return;
        }
        let at = target - sel.iter().filter(|&&i| i < target).count();
        self.selected = (at..at + sel.len()).collect();
        self.pages = pages;
        self.history.push(self.pages.clone());
        self.status = "Moved pages".into();
    }

    fn step_move_selected(&mut self, delta: isize) {
        if self.selected.is_empty() {
            self.status = "No pages selected".into();
            return;
        }
        let sel = self.sel_vec();
        let mut pages = self.pages.clone();
        let new_sel = arrange::step_move_pages(&mut pages, &sel, delta);
        let unchanged = pages.iter().map(|p| p.uid).eq(self.pages.iter().map(|p| p.uid));
        if unchanged {
            return;
        }
        self.selected = new_sel.into_iter().collect();
        self.anchor = self.selected.iter().next().copied();
        self.pages = pages;
        self.history.push(self.pages.clone());
        self.status = if delta < 0 { "Moved left" } else { "Moved right" }.into();
    }

    // -------------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = AntTokens::for_mode(self.dark);
        ui.horizontal_wrapped(|ui| {
            ant_btn(ui, t, true, "+ Open", BtnKind::Default, || a.push(Action::Open));
            ant_btn(ui, t, true, "Append", BtnKind::Default, || a.push(Action::Append));
            ui.separator();
            ant_btn(ui, t, true, "Undo", BtnKind::Default, || a.push(Action::Undo));
            ant_btn(ui, t, true, "Redo", BtnKind::Default, || a.push(Action::Redo));
            ui.separator();
            let sel_kind = if self.select_mode { BtnKind::Active } else { BtnKind::Default };
            let reo_kind = if self.reorder_mode { BtnKind::Active } else { BtnKind::Default };
            ant_btn(ui, t, true, "Select", sel_kind, || a.push(Action::ToggleSelectMode));
            ant_btn(ui, t, true, "Reorder", reo_kind, || a.push(Action::ToggleReorderMode));
            ui.separator();
            ant_btn(ui, t, !self.pages.is_empty(), "Save", BtnKind::Primary, || a.push(Action::Save));
            let tools_kind = if self.show_tools { BtnKind::Active } else { BtnKind::Default };
            ant_btn(ui, t, true, "Tools", tools_kind, || a.push(Action::ToggleTools));
        });
    }

    fn tools_panel(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = AntTokens::for_mode(self.dark);
        ui.horizontal(|ui| {
            ui.strong(egui::RichText::new("Tools").color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ant_btn_sized(ui, t, [36.0, 36.0], true, "X", BtnKind::Default, || {
                    a.push(Action::ToggleTools);
                });
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width();
            let section = |ui: &mut egui::Ui, title: &str| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(title).small().strong().color(t.text_secondary));
            };

            section(ui, "Files & Pages");
            side_btn(ui, t, w, true, "Import Image(s)...", BtnKind::Default, || a.push(Action::ImportImages));
            side_btn(ui, t, w, true, "Insert Blank Page", BtnKind::Default, || a.push(Action::InsertBlank));
            side_btn(ui, t, w, !self.pages.is_empty(), "Split All Pages to Folder...", BtnKind::Default, || {
                a.push(Action::SplitAll);
            });
            side_btn(ui, t, w, !self.pages.is_empty(), "Generate Booklet...", BtnKind::Default, || {
                a.push(Action::Booklet);
            });
            side_btn(ui, t, w, true, "Edit Metadata...", BtnKind::Default, || a.push(Action::Metadata));

            ui.separator();
            section(ui, "Selection");
            side_btn(ui, t, w, !self.pages.is_empty(), "Select All (Ctrl+A)", BtnKind::Default, || {
                a.push(Action::SelectAll);
            });
            side_btn(ui, t, w, !self.selected.is_empty(), "Deselect All", BtnKind::Default, || {
                a.push(Action::SelectNone);
            });
            side_btn(ui, t, w, !self.pages.is_empty(), "Invert Selection", BtnKind::Default, || {
                a.push(Action::Invert);
            });
            side_btn(ui, t, w, !self.pages.is_empty(), "Select Odd Pages", BtnKind::Default, || {
                a.push(Action::SelectOdd);
            });
            side_btn(ui, t, w, !self.pages.is_empty(), "Select Even Pages", BtnKind::Default, || {
                a.push(Action::SelectEven);
            });
            side_btn(ui, t, w, !self.selected.is_empty(), "Select Same Source", BtnKind::Default, || {
                a.push(Action::SelectSame);
            });

            ui.separator();
            section(ui, "Clipboard & Arrange");
            side_btn(ui, t, w, !self.selected.is_empty(), "Copy (Ctrl+C)", BtnKind::Default, || a.push(Action::Copy));
            side_btn(ui, t, w, !self.selected.is_empty(), "Cut (Ctrl+X)", BtnKind::Default, || a.push(Action::Cut));
            side_btn(ui, t, w, !self.clipboard.is_empty(), "Paste After (Ctrl+V)", BtnKind::Default, || {
                a.push(Action::PasteAfter);
            });
            side_btn(ui, t, w, !self.clipboard.is_empty() && !self.selected.is_empty(), "Paste as Overlay", BtnKind::Default, || {
                a.push(Action::PasteOverlay);
            });
            side_btn(ui, t, w, !self.clipboard.is_empty() && !self.selected.is_empty(), "Paste as Underlay", BtnKind::Default, || {
                a.push(Action::PasteUnderlay);
            });
            side_btn(ui, t, w, !self.selected.is_empty(), "Rotate 180", BtnKind::Default, || {
                a.push(Action::Rotate180);
            });
            side_btn(ui, t, w, self.pages.len() >= 2, "Reverse Order", BtnKind::Default, || a.push(Action::Reverse));
            side_btn(ui, t, w, self.pages.len() >= 2, "Swap Odd/Even Pages", BtnKind::Default, || {
                a.push(Action::SwapOddEven);
            });

            ui.separator();
            section(ui, "Zoom & Theme");
            ui.horizontal(|ui| {
                let bw = ((w - 16.0) / 3.0).max(50.0);
                ant_btn_sized(ui, t, [bw, TOUCH_HEIGHT], true, "-", BtnKind::Default, || a.push(Action::ZoomOut));
                ant_btn_sized(ui, t, [bw, TOUCH_HEIGHT], true, "Reset", BtnKind::Default, || a.push(Action::ZoomReset));
                ant_btn_sized(ui, t, [bw, TOUCH_HEIGHT], true, "+", BtnKind::Default, || a.push(Action::ZoomIn));
            });
            ui.add(egui::Slider::new(&mut self.zoom, 80.0..=400.0).show_value(false));
            let theme_lbl = if self.dark { "Switch to Light Theme" } else { "Switch to Dark Theme" };
            side_btn(ui, t, w, true, theme_lbl, BtnKind::Default, || a.push(Action::ToggleTheme));

            ui.separator();
            side_btn(ui, t, w, true, "Quit", BtnKind::Danger, || a.push(Action::Quit));
        });
    }

    fn selection_bar(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = AntTokens::for_mode(self.dark);
        let count = self.selected.len();
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("{count} selected")).strong().color(t.primary));
            ui.separator();
            ant_btn(ui, t, true, "<", BtnKind::Default, || a.push(Action::StepLeft));
            ant_btn(ui, t, true, ">", BtnKind::Default, || a.push(Action::StepRight));
            ui.separator();
            ant_btn(ui, t, true, "<- Rotate", BtnKind::Default, || a.push(Action::RotateLeft));
            ant_btn(ui, t, true, "Rotate ->", BtnKind::Default, || a.push(Action::RotateRight));
            ant_btn(ui, t, true, "Crop", BtnKind::Default, || a.push(Action::CropDialog));
            ant_btn(ui, t, true, "Dup", BtnKind::Default, || a.push(Action::Duplicate));
            ant_btn(ui, t, true, "Extract PDF", BtnKind::Default, || a.push(Action::ExtractSingle));
            ant_btn(ui, t, true, "Split Sel", BtnKind::Default, || a.push(Action::ExtractSeparate));
            ant_btn(ui, t, true, "Delete", BtnKind::Danger, || a.push(Action::Delete));
            ant_btn(ui, t, true, "Clear", BtnKind::Default, || a.push(Action::SelectNone));
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let t = AntTokens::for_mode(self.dark);
        ui.horizontal_wrapped(|ui| {
            if let Some(msg) = &self.busy {
                ui.spinner();
                ui.label(egui::RichText::new(msg).small().color(t.primary));
                ui.separator();
            }
            ui.label(
                egui::RichText::new(format!("{} pages | {} source(s)", self.pages.len(), self.sources.len()))
                    .small()
                    .color(t.text_secondary),
            );
            ui.separator();
            let status_color = if self.status.starts_with("Error:") {
                t.danger
            } else if self.status.starts_with("Saved ")
                || self.status.starts_with("Wrote ")
                || self.status.starts_with("Loaded ")
            {
                t.success
            } else {
                t.text_secondary
            };
            ui.label(egui::RichText::new(&self.status).small().color(status_color));
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let t = AntTokens::for_mode(self.dark);
        if self.pages.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.22).max(20.0));
                egui::Frame::new()
                    .fill(t.bg_container)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(32, 28))
                    .show(ui, |ui| {
                        ui.set_max_width(440.0);
                        ui.vertical_centered(|ui| {
                            ui.heading(egui::RichText::new("PDF Wizard").color(t.text));
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("Tap Open to choose PDF or image files, or drop files onto this window.")
                                    .color(t.text_secondary),
                            );
                            ui.add_space(20.0);
                            ui.horizontal(|ui| {
                                let w = ui.available_width();
                                let pad = ((w - 376.0) * 0.5).max(0.0);
                                ui.add_space(pad);
                                ant_btn_sized(ui, t, [200.0, 48.0], true, "Open PDF or Images", BtnKind::Primary, || {
                                    a.push(Action::Open);
                                });
                                ant_btn_sized(ui, t, [165.0, 48.0], true, "Insert Blank Page", BtnKind::Default, || {
                                    a.push(Action::InsertBlank);
                                });
                            });
                        });
                    });
            });
            return;
        }
        let card = Vec2::new(self.zoom, self.zoom * 1.35 + 64.0);
        let cols = grid_columns(ui.available_width(), card.x, SPACING);
        self.drop_target = None;
        let n = self.pages.len();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(SPACING);
            for start in (0..n).step_by(cols) {
                ui.horizontal(|ui| {
                    for i in start..(start + cols).min(n) {
                        self.card(ui, i, card, a);
                    }
                });
            }
        });
        if self.dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if ui.input(|i| i.pointer.any_released()) {
                self.dragging = false;
                if let Some(t) = self.drop_target {
                    self.move_selected_to(t);
                }
            }
        }
    }

    fn card(&mut self, ui: &mut egui::Ui, i: usize, size: Vec2, a: &mut Vec<Action>) {
        let item = self.pages[i].clone();
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let selected = self.selected.contains(&i);
        let header_h = CARD_HEADER_H;
        let footer_h = 24.0;
        let btn_w = (size.x * 0.34).clamp(32.0, 40.0);

        let check_rect = Rect::from_min_size(rect.min + Vec2::new(4.0, 2.0), Vec2::new(btn_w, CARD_BTN_H));
        let handle_rect = Rect::from_min_size(
            egui::pos2(rect.right() - btn_w - 4.0, rect.top() + 2.0),
            Vec2::new(btn_w, CARD_BTN_H),
        );
        let body_rect = Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + header_h),
            rect.max,
        );
        let title_rect = Rect::from_min_max(
            egui::pos2(check_rect.right(), rect.top()),
            egui::pos2(handle_rect.left(), rect.top() + header_h),
        );

        let check_resp = ui.interact(check_rect, ui.scope_id().with(("chk", item.uid, i)), Sense::click());
        let handle_resp = ui.interact(handle_rect, ui.scope_id().with(("hdl", item.uid, i)), Sense::drag());
        let title_resp = ui.interact(title_rect, ui.scope_id().with(("ttl", item.uid, i)), Sense::click());
        let body_sense = if self.reorder_mode { Sense::click_and_drag() } else { Sense::click() };
        let body_resp = ui.interact(body_rect, ui.scope_id().with(("bdy", item.uid, i)), body_sense);

        if ui.is_rect_visible(rect) {
            let t = AntTokens::for_mode(self.dark);
            let thumb = item.source_id.map(|sid| self.ensure_thumb(sid, item.source_page_num));
            let painter = ui.painter_at(rect);
            let bg = if selected { t.primary_bg } else { t.bg_container };
            painter.rect_filled(rect, 8.0, bg);
            let border_stroke = if selected {
                egui::Stroke::new(2.0, t.primary)
            } else {
                egui::Stroke::new(1.0, t.border)
            };
            painter.rect_stroke(rect, 8.0, border_stroke, egui::StrokeKind::Inside);

            // Top-left touch checkbox (Ant Design Checkbox)
            let chk_fill = if selected { t.primary } else { t.bg_container };
            painter.rect_filled(check_rect.shrink(3.0), 6.0, chk_fill);
            painter.text(
                check_rect.center(),
                egui::Align2::CENTER_CENTER,
                if selected { "✓" } else { " " },
                egui::FontId::proportional(16.0),
                if selected { egui::Color32::WHITE } else { t.text },
            );
            if !selected {
                painter.rect_stroke(
                    check_rect.shrink(3.0),
                    6.0,
                    egui::Stroke::new(1.5, t.border),
                    egui::StrokeKind::Inside,
                );
            }

            // Top-right touch drag handle
            painter.rect_filled(handle_rect.shrink(3.0), 6.0, t.bg_elevated);
            painter.rect_stroke(
                handle_rect.shrink(3.0),
                6.0,
                egui::Stroke::new(1.0, t.border_secondary),
                egui::StrokeKind::Inside,
            );
            painter.text(
                handle_rect.center(),
                egui::Align2::CENTER_CENTER,
                "⋮⋮",
                egui::FontId::proportional(15.0),
                t.text_secondary,
            );

            // Top-center page number & transform tags (clipped to avoid overlapping side buttons)
            painter.with_clip_rect(title_rect).text(
                title_rect.center(),
                egui::Align2::CENTER_CENTER,
                card_header_text(i, &item, size.x),
                egui::FontId::proportional(12.5),
                if selected { t.primary } else { t.text },
            );

            // Thumbnail area
            let area = Rect::from_min_max(
                egui::pos2(rect.left() + 8.0, rect.top() + header_h + 2.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - footer_h),
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
            painter.rect_filled(paper, 0.0, egui::Color32::WHITE);

            match &thumb {
                Some(Thumb::Ready { tex, w, h }) => {
                    let unrot = if swap { Vec2::new(shown.y, shown.x) } else { shown };
                    let img_rect = Rect::from_center_size(area.center(), unrot);
                    let uv = Rect::from_min_max(
                        egui::pos2(c.left, c.top),
                        egui::pos2(1.0 - c.right, 1.0 - c.bottom),
                    );
                    let angle = (item.rotation as f32).to_radians();
                    egui::Image::new(egui::load::SizedTexture::new(tex.id(), Vec2::new(*w, *h)))
                        .uv(uv)
                        .rotate(angle, Vec2::splat(0.5))
                        .paint_at(ui, img_rect);
                }
                Some(Thumb::Loading) => {
                    painter.text(paper.center(), egui::Align2::CENTER_CENTER, "...", egui::FontId::proportional(14.0), egui::Color32::GRAY);
                }
                Some(Thumb::Failed) => {
                    painter.text(paper.center(), egui::Align2::CENTER_CENTER, "no preview", egui::FontId::proportional(12.0), egui::Color32::GRAY);
                }
                None => {}
            }

            let max_chars = ((size.x / 6.5) as usize).max(8);
            let mut label: String = item.label.chars().take(max_chars).collect();
            if item.label.chars().count() > max_chars {
                label.push('…');
            }
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - footer_h * 0.5),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                t.text_secondary,
            );
        }

        if check_resp.clicked() {
            self.toggle_select(i);
        } else if body_resp.clicked() || title_resp.clicked() {
            let m = ui.input(|inp| inp.modifiers);
            self.handle_card_click(i, m.shift, m.command);
        }
        if (body_resp.secondary_clicked() || title_resp.secondary_clicked()) && !self.selected.contains(&i) {
            self.select_only(i);
        }
        if handle_resp.drag_started() || (self.reorder_mode && body_resp.drag_started()) {
            if !self.selected.contains(&i) {
                self.select_only(i);
            }
            self.dragging = true;
        }
        if self.dragging {
            if let Some(pos) = ui.input(|inp| inp.pointer.hover_pos()) {
                if rect.contains(pos) {
                    let before = pos.x < rect.center().x;
                    self.drop_target = Some(if before { i } else { i + 1 });
                    let x = if before { rect.left() - SPACING / 2.0 } else { rect.right() + SPACING / 2.0 };
                    ui.painter().line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(3.5, ui.visuals().selection.stroke.color),
                    );
                }
            }
        }
        body_resp.context_menu(|ui| {
            mi(ui, a, "Rotate Left", "", Action::RotateLeft);
            mi(ui, a, "Rotate Right", "", Action::RotateRight);
            mi(ui, a, "Crop and Scale...", "", Action::CropDialog);
            ui.separator();
            mi(ui, a, "Copy", "", Action::Copy);
            mi(ui, a, "Cut", "", Action::Cut);
            mi(ui, a, "Paste After", "", Action::PasteAfter);
            mi(ui, a, "Paste as Overlay", "", Action::PasteOverlay);
            mi(ui, a, "Paste as Underlay", "", Action::PasteUnderlay);
            mi(ui, a, "Duplicate", "", Action::Duplicate);
            mi(ui, a, "Delete", "", Action::Delete);
            ui.separator();
            mi(ui, a, "Extract Selected to Single File...", "", Action::ExtractSingle);
            mi(ui, a, "Extract Selected to Separate Files...", "", Action::ExtractSeparate);
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let t = AntTokens::for_mode(self.dark);
        let mut open = self.show_meta;
        let mut close_meta = false;
        egui::Window::new("Document Metadata")
            .open(&mut open)
            .collapsible(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::Grid::new("meta").num_columns(2).spacing([10.0, 12.0]).show(ui, |ui| {
                    for (label, field) in [
                        ("Title", &mut self.meta.title),
                        ("Author", &mut self.meta.author),
                        ("Subject", &mut self.meta.subject),
                        ("Keywords", &mut self.meta.keywords),
                    ] {
                        ui.label(label);
                        ui.add(egui::TextEdit::singleline(field).min_size(Vec2::new(220.0, 38.0)));
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Applied when saving or extracting.").color(t.text_secondary));
                ui.add_space(8.0);
                ant_btn_sized(ui, t, [120.0, TOUCH_HEIGHT], true, "Done", BtnKind::Primary, || {
                    close_meta = true;
                });
            });
        self.show_meta = open && !close_meta;

        let mut open = self.show_crop;
        let mut apply = false;
        egui::Window::new("Crop and Scale")
            .open(&mut open)
            .collapsible(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                for (label, v) in [
                    ("Left %", 0),
                    ("Right %", 1),
                    ("Top %", 2),
                    ("Bottom %", 3),
                ] {
                    ui.add(egui::Slider::new(&mut self.crop_edit[v], 0.0..=49.0).text(label));
                }
                ui.add(egui::Slider::new(&mut self.scale_edit, 10.0..=400.0).text("Scale %"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ant_btn_sized(ui, t, [160.0, TOUCH_HEIGHT], true, "Apply to selected", BtnKind::Primary, || {
                        apply = true;
                    });
                    ant_btn_sized(ui, t, [110.0, TOUCH_HEIGHT], true, "Reset", BtnKind::Default, || {
                        self.crop_edit = [0.0; 4];
                        self.scale_edit = 100.0;
                    });
                });
            });
        self.show_crop = open && !apply;
        if apply {
            self.apply_crop();
        }
    }
}

fn make_ant_button(t: AntTokens, label: &str, kind: BtnKind, min_size: Vec2) -> egui::Button<'_> {
    let (fill, stroke_col, text_col) = match kind {
        BtnKind::Primary => (t.primary, t.primary, egui::Color32::WHITE),
        BtnKind::Active => (t.primary_bg, t.primary, t.primary),
        BtnKind::Danger => (t.danger_bg, t.danger, t.danger),
        BtnKind::Default => (t.bg_container, t.border, t.text),
    };
    egui::Button::new(egui::RichText::new(label).color(text_col))
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke_col))
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(min_size)
}

fn ant_btn(
    ui: &mut egui::Ui,
    t: AntTokens,
    enabled: bool,
    label: &str,
    kind: BtnKind,
    on_click: impl FnOnce(),
) {
    ant_btn_sized(ui, t, Vec2::new(0.0, TOUCH_HEIGHT), enabled, label, kind, on_click);
}

fn ant_btn_sized(
    ui: &mut egui::Ui,
    t: AntTokens,
    size: impl Into<Vec2>,
    enabled: bool,
    label: &str,
    kind: BtnKind,
    on_click: impl FnOnce(),
) {
    let btn = make_ant_button(t, label, kind, size.into());
    if ui.add_enabled(enabled, btn).clicked() {
        on_click();
    }
}

fn side_btn(
    ui: &mut egui::Ui,
    t: AntTokens,
    width: f32,
    enabled: bool,
    label: &str,
    kind: BtnKind,
    on_click: impl FnOnce(),
) {
    ant_btn_sized(ui, t, [width, TOUCH_HEIGHT], enabled, label, kind, on_click);
}

fn mi(ui: &mut egui::Ui, a: &mut Vec<Action>, label: &str, shortcut: &str, act: Action) {
    let mut button = egui::Button::new(label).min_size(Vec2::new(220.0, 42.0));
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    if ui.add(button).clicked() {
        a.push(act);
        ui.close();
    }
}

impl eframe::App for PdfWizardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_bg(&ctx);
        self.poll_thumbs(&ctx);
        let mut actions: Vec<Action> = Vec::new();
        self.handle_input(&ctx, &mut actions);
        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui, &mut actions));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if !self.selected.is_empty() {
            egui::Panel::bottom("selection_bar").show(ui, |ui| self.selection_bar(ui, &mut actions));
        }
        if self.show_tools {
            egui::Panel::right("tools_panel")
                .resizable(false)
                .default_size(270.0)
                .show(ui, |ui| self.tools_panel(ui, &mut actions));
        }
        egui::CentralPanel::default().show(ui, |ui| self.grid(ui, &mut actions));
        self.dialogs(&ctx);
        for action in actions {
            self.run(action, &ctx);
        }
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
    fn empty_selection_actions_set_status_message() {
        let mut app = PdfWizardApp::empty();
        app.commit(vec![pdf_engine::blank_page(1, A4_W, A4_H)]);
        let ctx = egui::Context::default();
        for act in [Action::Delete, Action::Cut, Action::Copy] {
            app.status.clear();
            app.run(act, &ctx);
            assert_eq!(app.status, "No pages selected");
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

        app.add_loaded(&[f1], true);
        assert_eq!(app.sources.len(), 1);
        app.thumbs.insert((0, 1), Thumb::Loading);

        app.add_loaded(&[f2], true);
        assert_eq!(app.sources.len(), 1);
        assert!(!app.sources.contains_key(&0));
        assert!(!app.thumbs.contains_key(&(0, 1)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn select_mode_toggles_without_modifiers() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (0..4).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.commit(pages);
        app.select_mode = true;
        app.handle_card_click(0, false, false);
        app.handle_card_click(2, false, false);
        assert_eq!(app.sel_vec(), vec![0, 2]);
        app.handle_card_click(0, false, false);
        assert_eq!(app.sel_vec(), vec![2]);
    }

    #[test]
    fn step_move_selected_updates_pages_and_selection() {
        let mut app = PdfWizardApp::empty();
        let pages: Vec<PageItem> = (1..=3).map(|i| pdf_engine::blank_page(i, A4_W, A4_H)).collect();
        app.commit(pages);
        app.select_only(1);
        let ctx = egui::Context::default();
        app.run(Action::StepLeft, &ctx);
        assert_eq!(app.pages.iter().map(|p| p.uid).collect::<Vec<_>>(), vec![2, 1, 3]);
        assert_eq!(app.sel_vec(), vec![0]);

        app.run(Action::StepRight, &ctx);
        assert_eq!(app.pages.iter().map(|p| p.uid).collect::<Vec<_>>(), vec![1, 2, 3]);
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
    fn touch_style_sets_min_interact_size() {
        let ctx = egui::Context::default();
        apply_touch_style(&ctx);
        let s = ctx.global_style();
        assert!(s.spacing.interact_size.y >= 44.0);
        assert!(s.spacing.scroll.bar_width >= 12.0);
    }

    #[test]
    fn card_header_text_compacts_at_low_zoom() {
        let mut item = pdf_engine::blank_page(1, A4_W, A4_H);
        item.rotate(90);
        item.crop.left = 0.1;
        assert_eq!(card_header_text(0, &item, 80.0), "#1");
        assert_eq!(card_header_text(0, &item, 190.0), "#1 R90 crop");
    }

    #[test]
    fn card_touch_target_meets_36px_minimum() {
        const { assert!(CARD_BTN_H >= 36.0); }
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
    fn ant_theme_supports_light_and_dark_modes() {
        let ctx = egui::Context::default();
        theme::apply_ant_theme(&ctx, false);
        assert!(!ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.panel_fill, AntTokens::light().bg_layout);
        assert!(ctx.global_style().spacing.interact_size.y >= 44.0);

        theme::apply_ant_theme(&ctx, true);
        assert!(ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.panel_fill, AntTokens::dark().bg_layout);
        assert!(ctx.global_style().spacing.interact_size.y >= 44.0);
    }

    #[test]
    fn toggle_tools_toggles_side_panel() {
        let mut app = PdfWizardApp::empty();
        let ctx = egui::Context::default();
        assert!(!app.show_tools);
        app.run(Action::ToggleTools, &ctx);
        assert!(app.show_tools);
        app.run(Action::ToggleTools, &ctx);
        assert!(!app.show_tools);
    }
}
