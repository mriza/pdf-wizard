#![allow(dead_code)]
use crate::model::SourceId;
use crossbeam_channel::{unbounded, Receiver, Sender};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

pub struct ThumbImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct ThumbResult {
    pub source_id: SourceId,
    pub page_num: u32,
    /// `None` means the page could not be rendered (UI shows a placeholder).
    pub image: Option<ThumbImage>,
}

struct Request {
    source_id: SourceId,
    bytes: Arc<Vec<u8>>,
    page_num: u32,
    max_px: u32,
}

/// Background thumbnail renderer. Render failures (including panics inside the PDF
/// library) come back as `image: None`; the worker thread keeps running.
pub struct Renderer {
    tx: Sender<Request>,
    rx: Receiver<ThumbResult>,
    inflight: Mutex<HashSet<(SourceId, u32, u32)>>,
}

fn render_page(pdf: &Pdf, page_num: u32, max_px: u32) -> Option<ThumbImage> {
    let pages = pdf.pages();
    let page = pages.get((page_num as usize).checked_sub(1)?)?;
    let (w, h) = page.render_dimensions();
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let scale = (max_px as f32 / w.max(h)).min(4.0);
    let cache = RenderCache::new();
    let pixmap = render(
        page,
        &cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    let (pw, ph) = (pixmap.width() as usize, pixmap.height() as usize);
    if pw == 0 || ph == 0 {
        return None;
    }
    Some(ThumbImage { width: pw, height: ph, rgba: pixmap.data_as_u8_slice().to_vec() })
}

fn worker(rx: Receiver<Request>, tx: Sender<ThumbResult>) {
    // Parsed documents are kept per source so each page request does not re-parse the file.
    let mut docs: HashMap<SourceId, Option<Pdf>> = HashMap::new();
    while let Ok(req) = rx.recv() {
        let image = catch_unwind(AssertUnwindSafe(|| {
            let pdf = docs
                .entry(req.source_id)
                .or_insert_with(|| Pdf::new(req.bytes.clone()).ok());
            pdf.as_ref().and_then(|p| render_page(p, req.page_num, req.max_px))
        }))
        .unwrap_or(None);
        let result = ThumbResult { source_id: req.source_id, page_num: req.page_num, image };
        if tx.send(result).is_err() {
            break;
        }
    }
}

impl Renderer {
    pub fn new() -> Self {
        let (req_tx, req_rx) = unbounded();
        let (res_tx, res_rx) = unbounded();
        std::thread::Builder::new()
            .name("thumbnail-renderer".into())
            .spawn(move || worker(req_rx, res_tx))
            .expect("failed to spawn renderer thread");
        Renderer { tx: req_tx, rx: res_rx, inflight: Mutex::new(HashSet::new()) }
    }

    pub fn request(&self, source_id: SourceId, bytes: Arc<Vec<u8>>, page_num: u32, max_px: u32) {
        if !self.inflight.lock().unwrap().insert((source_id, page_num, max_px)) {
            return;
        }
        let _ = self.tx.send(Request { source_id, bytes, page_num, max_px });
    }

    pub fn poll(&self) -> Vec<ThumbResult> {
        let results: Vec<ThumbResult> = self.rx.try_iter().collect();
        if !results.is_empty() {
            let mut inflight = self.inflight.lock().unwrap();
            for r in &results {
                inflight.retain(|&(s, p, _)| !(s == r.source_id && p == r.page_num));
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_engine::testutil::make_pdf;
    use std::time::{Duration, Instant};

    fn wait_for(r: &Renderer, n: usize) -> Vec<ThumbResult> {
        let mut out = Vec::new();
        let start = Instant::now();
        while out.len() < n && start.elapsed() < Duration::from_secs(20) {
            out.extend(r.poll());
            std::thread::sleep(Duration::from_millis(20));
        }
        out
    }

    #[test]
    fn renders_generated_pdf_page() {
        let r = Renderer::new();
        r.request(1, Arc::new(make_pdf(2)), 2, 360);
        let res = wait_for(&r, 1);
        assert_eq!(res.len(), 1);
        assert_eq!((res[0].source_id, res[0].page_num), (1, 2));
        let img = res[0].image.as_ref().expect("page should render");
        assert!(img.width > 0 && img.height > 0);
        assert!(img.width.max(img.height) <= 360);
        assert_eq!(img.rgba.len(), img.width * img.height * 4);
    }

    #[test]
    fn garbage_bytes_yield_none_not_panic() {
        let r = Renderer::new();
        r.request(7, Arc::new(b"definitely not a pdf".to_vec()), 1, 360);
        let res = wait_for(&r, 1);
        assert_eq!(res.len(), 1);
        assert!(res[0].image.is_none());
    }

    #[test]
    fn missing_page_yields_none_and_worker_survives() {
        let r = Renderer::new();
        let pdf = Arc::new(make_pdf(1));
        r.request(1, pdf.clone(), 99, 360);
        r.request(1, pdf, 1, 360);
        let res = wait_for(&r, 2);
        assert_eq!(res.len(), 2);
        assert!(res.iter().any(|x| x.page_num == 99 && x.image.is_none()));
        assert!(res.iter().any(|x| x.page_num == 1 && x.image.is_some()));
    }

    #[test]
    fn duplicate_inflight_requests_are_deduplicated() {
        let r = Renderer::new();
        let pdf = Arc::new(make_pdf(1));
        for _ in 0..5 {
            r.request(1, pdf.clone(), 1, 360);
        }
        let res = wait_for(&r, 1);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(res.len() + r.poll().len(), 1);
    }
}
