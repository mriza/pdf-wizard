use crate::arrange;
use crate::history::History;
use crate::model::{CropMargins, Metadata, OverlaySpec, PageItem, SourceId};
use crate::pdf_engine::{self, SourceDocument, A4_H, A4_W};
use crate::renderer::Renderer;
use eframe::egui::{self, Key, Modifiers, Rect, Sense, Vec2};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

const THUMB_PX: u32 = 360;
const SPACING: f32 = 14.0;
const TOUCH_HEIGHT: f32 = 44.0;

/// Configures generous spacing, hit targets, and scrollbars for touchscreen use.
pub fn apply_touch_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.interact_size = Vec2::new(TOUCH_HEIGHT, TOUCH_HEIGHT);
        s.spacing.button_padding = Vec2::new(14.0, 10.0);
        s.spacing.item_spacing = Vec2::new(10.0, 10.0);
        s.spacing.slider_width = 180.0;
        s.spacing.slider_rail_height = 14.0;
        s.spacing.scroll.bar_width = 14.0;
        s.spacing.scroll.handle_min_length = TOUCH_HEIGHT;
    });
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
    crop_edit: [f32; 4],
    scale_edit: f32,
    dragging: bool,
    drop_target: Option<usize>,
}

fn is_pdf(p: &Path) -> bool {
    p.extension().map_or(false, |e| e.eq_ignore_ascii_case("pdf"))
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
            crop_edit: [0.0; 4],
            scale_edit: 100.0,
            dragging: false,
            drop_target: None,
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        apply_touch_style(&cc.egui_ctx);
        let mut app = Self::empty();
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
        let mut items = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let name = path.file_name().map_or("file".to_string(), |n| n.to_string_lossy().to_string());
            let id = self.next_source;
            let result = std::fs::read(path).map_err(anyhow::Error::from).and_then(|bytes| {
                if is_pdf(path) {
                    pdf_engine::load_pdf(id, &name, bytes)
                } else {
                    pdf_engine::image_to_source(id, &name, &bytes)
                }
            });
            match result {
                Ok(src) => {
                    self.next_source += 1;
                    for n in 1..=src.page_count as u32 {
                        let (w, h) = pdf_engine::page_size(&src, n);
                        let uid = self.uid();
                        items.push(PageItem::from_source(uid, id, n, w, h, format!("{name} - p.{n}")));
                    }
                    self.sources.insert(id, src);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        self.status = if errors.is_empty() {
            format!("Loaded {} page(s)", items.len())
        } else {
            format!("Error: {}", errors.join("; "))
        };
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
                let replace = matches!(action, Action::Open);
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("PDF and images", &["pdf", "png", "jpg", "jpeg", "webp"])
                    .pick_files()
                {
                    self.add_loaded(&paths, replace);
                }
            }
            Action::ImportImages => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                    .pick_files()
                {
                    self.add_loaded(&paths, false);
                }
            }
            Action::DropFiles(paths) => {
                let ok: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| {
                        is_pdf(p)
                            || p.extension().map_or(false, |e| {
                                ["png", "jpg", "jpeg", "webp"].iter().any(|x| e.eq_ignore_ascii_case(x))
                            })
                    })
                    .collect();
                if ok.is_empty() {
                    self.status = "Dropped files are not PDF or image files".into();
                } else {
                    self.add_loaded(&ok, false);
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
                ctx.set_visuals(if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() });
                apply_touch_style(ctx);
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
        }
    }

    fn renumber(&mut self, mut p: PageItem) -> PageItem {
        p.uid = self.uid();
        p
    }

    fn add_loaded(&mut self, paths: &[PathBuf], replace: bool) {
        let loaded = self.load_paths(paths);
        if loaded.is_empty() {
            return;
        }
        if replace {
            self.pages = loaded;
            self.history = History::new(self.pages.clone());
            self.selected.clear();
            self.anchor = None;
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
        let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(default_name).save_file()
        else {
            return;
        };
        let result = if booklet {
            pdf_engine::export_booklet(&pages, &self.sources, &self.meta)
        } else {
            pdf_engine::export_pages(&pages, &self.sources, &self.meta)
        };
        self.status = match result.and_then(|b| std::fs::write(&path, b).map_err(Into::into)) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Error: {e}"),
        };
    }

    fn extract_to_folder(&mut self, pages: Vec<PageItem>) {
        let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
        self.status = match pdf_engine::extract_separate(&pages, &self.sources, &dir, &self.meta) {
            Ok(files) => format!("Wrote {} file(s) to {}", files.len(), dir.display()),
            Err(e) => format!("Error: {e}"),
        };
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
        ui.horizontal_wrapped(|ui| {
            touch_btn(ui, true, "+ Open", false, || a.push(Action::Open));
            touch_btn(ui, true, "Append", false, || a.push(Action::Append));
            ui.separator();
            touch_btn(ui, true, "Undo", false, || a.push(Action::Undo));
            touch_btn(ui, true, "Redo", false, || a.push(Action::Redo));
            ui.separator();
            touch_btn(ui, true, "Select", self.select_mode, || a.push(Action::ToggleSelectMode));
            touch_btn(ui, true, "Reorder", self.reorder_mode, || a.push(Action::ToggleReorderMode));
            ui.separator();
            touch_btn(ui, !self.pages.is_empty(), "Save", false, || a.push(Action::Save));
            ui.menu_button("More", |ui| {
                egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                    ui.label("Files & Pages");
                    mi(ui, a, "Import Image(s)...", "", Action::ImportImages);
                    mi(ui, a, "Insert Blank Page", "", Action::InsertBlank);
                    mi(ui, a, "Split All Pages to Folder...", "", Action::SplitAll);
                    mi(ui, a, "Generate Booklet...", "", Action::Booklet);
                    mi(ui, a, "Edit Metadata...", "", Action::Metadata);
                    ui.separator();
                    ui.label("Selection");
                    mi(ui, a, "Select All", "Ctrl+A", Action::SelectAll);
                    mi(ui, a, "Deselect All", "", Action::SelectNone);
                    mi(ui, a, "Invert Selection", "", Action::Invert);
                    mi(ui, a, "Select Odd Pages", "", Action::SelectOdd);
                    mi(ui, a, "Select Even Pages", "", Action::SelectEven);
                    mi(ui, a, "Select Same Source", "", Action::SelectSame);
                    ui.separator();
                    ui.label("Clipboard & Arrange");
                    mi(ui, a, "Copy", "Ctrl+C", Action::Copy);
                    mi(ui, a, "Cut", "Ctrl+X", Action::Cut);
                    mi(ui, a, "Paste After", "Ctrl+V", Action::PasteAfter);
                    mi(ui, a, "Paste as Overlay", "", Action::PasteOverlay);
                    mi(ui, a, "Paste as Underlay", "", Action::PasteUnderlay);
                    mi(ui, a, "Rotate 180", "", Action::Rotate180);
                    mi(ui, a, "Reverse Order", "", Action::Reverse);
                    mi(ui, a, "Swap Odd/Even Pages", "", Action::SwapOddEven);
                    ui.separator();
                    ui.label("Zoom & Theme");
                    ui.horizontal(|ui| {
                        touch_btn(ui, true, " - ", false, || a.push(Action::ZoomOut));
                        touch_btn(ui, true, "Reset", false, || a.push(Action::ZoomReset));
                        touch_btn(ui, true, " + ", false, || a.push(Action::ZoomIn));
                    });
                    ui.add(egui::Slider::new(&mut self.zoom, 80.0..=400.0).show_value(false));
                    let theme_lbl = if self.dark { "Switch to Light Theme" } else { "Switch to Dark Theme" };
                    mi(ui, a, theme_lbl, "", Action::ToggleTheme);
                    ui.separator();
                    mi(ui, a, "Quit", "", Action::Quit);
                });
            });
        });
    }

    fn selection_bar(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        let count = self.selected.len();
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{count} sel"));
            ui.separator();
            touch_btn(ui, true, "<", false, || a.push(Action::StepLeft));
            touch_btn(ui, true, ">", false, || a.push(Action::StepRight));
            ui.separator();
            touch_btn(ui, true, "Rotate L", false, || a.push(Action::RotateLeft));
            touch_btn(ui, true, "Rotate R", false, || a.push(Action::RotateRight));
            touch_btn(ui, true, "Crop", false, || a.push(Action::CropDialog));
            touch_btn(ui, true, "Dup", false, || a.push(Action::Duplicate));
            ui.menu_button("Extract", |ui| {
                mi(ui, a, "Single PDF...", "", Action::ExtractSingle);
                mi(ui, a, "Separate PDFs...", "", Action::ExtractSeparate);
            });
            touch_btn(ui, true, "Delete", false, || a.push(Action::Delete));
            touch_btn(ui, true, "Clear", false, || a.push(Action::SelectNone));
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!(
                "{} pages | {} source(s) | {}",
                self.pages.len(),
                self.sources.len(),
                self.status
            ));
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, a: &mut Vec<Action>) {
        if self.pages.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.25).max(20.0));
                ui.heading("PDF Wizard");
                ui.add_space(8.0);
                ui.label("Tap Open to choose PDF or image files, or drop files onto this window.");
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    let pad = ((w - 380.0) * 0.5).max(0.0);
                    ui.add_space(pad);
                    if ui.add_sized([200.0, 48.0], egui::Button::new("Open PDF or Images")).clicked() {
                        a.push(Action::Open);
                    }
                    if ui.add_sized([165.0, 48.0], egui::Button::new("Insert Blank Page")).clicked() {
                        a.push(Action::InsertBlank);
                    }
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
        let header_h = 38.0;
        let footer_h = 24.0;
        let btn_w = (size.x * 0.34).clamp(32.0, 40.0);

        let check_rect = Rect::from_min_size(rect.min + Vec2::splat(4.0), Vec2::new(btn_w, header_h - 4.0));
        let handle_rect = Rect::from_min_size(
            egui::pos2(rect.right() - btn_w - 4.0, rect.top() + 4.0),
            Vec2::new(btn_w, header_h - 4.0),
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
            let thumb = item.source_id.map(|sid| self.ensure_thumb(sid, item.source_page_num));
            let painter = ui.painter_at(rect);
            let visuals = ui.visuals().clone();
            let bg = if selected { visuals.selection.bg_fill } else { visuals.faint_bg_color };
            painter.rect_filled(rect, 8.0, bg);
            if selected {
                painter.rect_stroke(
                    rect,
                    8.0,
                    egui::Stroke::new(2.5, visuals.selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
            }

            // Top-left touch checkbox
            let chk_fill = if selected { visuals.selection.stroke.color } else { visuals.widgets.inactive.bg_fill };
            painter.rect_filled(check_rect.shrink(3.0), 6.0, chk_fill);
            painter.text(
                check_rect.center(),
                egui::Align2::CENTER_CENTER,
                if selected { "✓" } else { " " },
                egui::FontId::proportional(16.0),
                if selected { egui::Color32::WHITE } else { visuals.text_color() },
            );
            if !selected {
                painter.rect_stroke(
                    check_rect.shrink(3.0),
                    6.0,
                    egui::Stroke::new(1.5, visuals.widgets.inactive.fg_stroke.color),
                    egui::StrokeKind::Inside,
                );
            }

            // Top-right touch drag handle
            painter.rect_filled(handle_rect.shrink(3.0), 6.0, visuals.widgets.inactive.bg_fill);
            painter.text(
                handle_rect.center(),
                egui::Align2::CENTER_CENTER,
                "⋮⋮",
                egui::FontId::proportional(15.0),
                visuals.text_color(),
            );

            // Top-center page number & transform tags
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
            painter.text(
                title_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("#{}{}", i + 1, tags),
                egui::FontId::proportional(12.5),
                visuals.text_color(),
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
                visuals.weak_text_color(),
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
                ui.label("Applied when saving or extracting.");
                ui.add_space(8.0);
                if ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("Done")).clicked() {
                    close_meta = true;
                }
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
                    if ui.add_sized([160.0, TOUCH_HEIGHT], egui::Button::new("Apply to selected")).clicked() {
                        apply = true;
                    }
                    if ui.add_sized([110.0, TOUCH_HEIGHT], egui::Button::new("Reset")).clicked() {
                        self.crop_edit = [0.0; 4];
                        self.scale_edit = 100.0;
                    }
                });
            });
        self.show_crop = open && !apply;
        if apply {
            self.apply_crop();
        }
    }
}

fn touch_btn(ui: &mut egui::Ui, enabled: bool, label: &str, active: bool, on_click: impl FnOnce()) {
    let mut btn = egui::Button::new(label).min_size(Vec2::new(0.0, TOUCH_HEIGHT));
    if active {
        btn = btn.fill(ui.visuals().selection.bg_fill);
    }
    if ui.add_enabled(enabled, btn).clicked() {
        on_click();
    }
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
        self.poll_thumbs(&ctx);
        let mut actions: Vec<Action> = Vec::new();
        self.handle_input(&ctx, &mut actions);
        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui, &mut actions));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if !self.selected.is_empty() {
            egui::Panel::bottom("selection_bar").show(ui, |ui| self.selection_bar(ui, &mut actions));
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
}
