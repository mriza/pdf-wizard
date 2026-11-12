#![allow(dead_code)]
use crate::model::{CropMargins, Metadata, PageItem, SourceId};
use anyhow::{anyhow, bail, Result};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceDocument {
    pub id: SourceId,
    pub file_name: String,
    pub raw_bytes: Arc<Vec<u8>>,
    pub doc: Arc<Document>,
    pub page_count: usize,
}

pub fn load_pdf(id: SourceId, file_name: &str, bytes: Vec<u8>) -> Result<SourceDocument> {
    let doc = Document::load_mem(&bytes)
        .map_err(|e| anyhow!("{file_name}: cannot read PDF ({e})"))?;
    if doc.is_encrypted() {
        bail!("{file_name}: password-protected PDFs are not supported");
    }
    let page_count = doc.get_pages().len();
    if page_count == 0 {
        bail!("{file_name}: PDF has no pages");
    }
    Ok(SourceDocument {
        id,
        file_name: file_name.to_string(),
        raw_bytes: Arc::new(bytes),
        doc: Arc::new(doc),
        page_count,
    })
}

/// Walks up the /Parent chain to find an inheritable page attribute.
fn inherited(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut cur = page_id;
    for _ in 0..64 {
        let d = doc.get_dictionary(cur).ok()?;
        if let Ok(v) = d.get(key) {
            return Some(v.clone());
        }
        cur = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn resolve_numbers(doc: &Document, obj: &Object) -> Option<Vec<f32>> {
    let (_, obj) = doc.dereference(obj).ok()?;
    let arr = obj.as_array().ok()?;
    arr.iter()
        .map(|o| doc.dereference(o).ok().and_then(|(_, o)| o.as_float().ok()))
        .collect()
}

pub fn page_size(doc: &SourceDocument, page_num: u32) -> (f32, f32) {
    let default = (595.0, 842.0);
    let Some(&pid) = doc.doc.get_pages().get(&page_num) else { return default };
    let Some(mb) = inherited(&doc.doc, pid, b"MediaBox") else { return default };
    match resolve_numbers(&doc.doc, &mb) {
        Some(v) if v.len() == 4 => ((v[2] - v[0]).abs(), (v[3] - v[1]).abs()),
        _ => default,
    }
}

type IdMap = HashMap<ObjectId, ObjectId>;

fn copy_ref(src: &Document, dst: &mut Document, map: &mut IdMap, id: ObjectId) -> ObjectId {
    if let Some(&n) = map.get(&id) {
        return n;
    }
    let new_id = dst.new_object_id();
    map.insert(id, new_id);
    let obj = match src.get_object(id) {
        Ok(o) => remap(src, dst, map, o),
        Err(_) => Object::Null,
    };
    dst.objects.insert(new_id, obj);
    new_id
}

fn remap_dict(src: &Document, dst: &mut Document, map: &mut IdMap, d: &Dictionary) -> Dictionary {
    let mut out = Dictionary::new();
    for (k, v) in d.iter() {
        out.set(k.clone(), remap(src, dst, map, v));
    }
    out
}

fn remap(src: &Document, dst: &mut Document, map: &mut IdMap, obj: &Object) -> Object {
    match obj {
        Object::Reference(id) => Object::Reference(copy_ref(src, dst, map, *id)),
        Object::Array(a) => Object::Array(a.iter().map(|o| remap(src, dst, map, o)).collect()),
        Object::Dictionary(d) => Object::Dictionary(remap_dict(src, dst, map, d)),
        Object::Stream(s) => {
            let dict = remap_dict(src, dst, map, &s.dict);
            let mut ns = Stream::new(dict, s.content.clone());
            ns.allows_compression = s.allows_compression;
            Object::Stream(ns)
        }
        other => other.clone(),
    }
}

fn pdf_text(s: &str) -> Object {
    // PDFDocEncoding for ASCII, UTF-16BE (with BOM) otherwise.
    if s.is_ascii() {
        Object::string_literal(s)
    } else {
        let mut v = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        Object::String(v, StringFormat::Hexadecimal)
    }
}

fn box_of(doc: &Document, page: &Dictionary, key: &[u8]) -> Option<[f32; 4]> {
    let v = resolve_numbers(doc, page.get(key).ok()?)?;
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

fn deref_dict(doc: &Document, obj: &Object) -> Dictionary {
    match doc.dereference(obj) {
        Ok((_, Object::Dictionary(d))) => d.clone(),
        _ => Dictionary::new(),
    }
}

fn num_array(b: [f32; 4]) -> Object {
    Object::Array(b.iter().map(|&x| Object::Real(x)).collect())
}

/// Wraps a source page's content as a Form XObject in the destination document.
fn make_form_with_bbox(
    src: &Document,
    dst: &mut Document,
    map: &mut IdMap,
    spid: ObjectId,
    bbox: [f32; 4],
) -> ObjectId {
    let content = src.get_page_content(spid);
    let res = match inherited(src, spid, b"Resources") {
        Some(r) => remap(src, dst, map, &r),
        None => Object::Dictionary(Dictionary::new()),
    };
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => num_array(bbox),
        "Resources" => res,
    };
    dst.add_object(Stream::new(dict, content))
}

fn make_form(src: &Document, dst: &mut Document, map: &mut IdMap, spid: ObjectId) -> ObjectId {
    let bbox = inherited(src, spid, b"MediaBox")
        .and_then(|o| resolve_numbers(src, &o))
        .filter(|v| v.len() == 4)
        .map(|v| [v[0], v[1], v[2], v[3]])
        .unwrap_or([0.0, 0.0, A4_W, A4_H]);
    make_form_with_bbox(src, dst, map, spid, bbox)
}

fn content_refs(dst: &Document, page: &Dictionary) -> Vec<Object> {
    match page.get(b"Contents") {
        Ok(Object::Reference(id)) => match dst.get_object(*id) {
            Ok(Object::Array(a)) => a.clone(),
            _ => vec![Object::Reference(*id)],
        },
        Ok(Object::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

fn stream_obj(dst: &mut Document, text: String) -> Object {
    Object::Reference(dst.add_object(Stream::new(Dictionary::new(), text.into_bytes())))
}

/// Maps visual crop margins (relative to the thumbnail rendered with `source_rotate`)
/// into PDF unrotated edge trims `(left, bottom, right, top)`.
fn unrotated_crop_trims(c: CropMargins, source_rotate: i64) -> (f32, f32, f32, f32) {
    match source_rotate.rem_euclid(360) {
        90 => (c.top, c.left, c.bottom, c.right),
        180 => (c.right, c.top, c.left, c.bottom),
        270 => (c.bottom, c.right, c.top, c.left),
        _ => (c.left, c.bottom, c.right, c.top),
    }
}

/// Applies crop, scale and overlays to a page dictionary already copied into the destination document.
fn apply_transforms(
    dst: &mut Document,
    page: &mut Dictionary,
    item: &PageItem,
    source_rotate: i64,
    sources: &HashMap<SourceId, SourceDocument>,
    maps: &mut HashMap<SourceId, IdMap>,
) -> Result<()> {
    let mut mb = box_of(dst, page, b"MediaBox").unwrap_or([0.0, 0.0, item.width_pt, item.height_pt]);
    let mut cb = box_of(dst, page, b"CropBox");
    let c = item.crop;
    if c != CropMargins::default() {
        let base = cb.unwrap_or(mb);
        let (w, h) = (base[2] - base[0], base[3] - base[1]);
        let (tl, tb, tr, tt) = unrotated_crop_trims(c, source_rotate);
        cb = Some([base[0] + tl * w, base[1] + tb * h, base[2] - tr * w, base[3] - tt * h]);
    }
    let scaled = (item.scale - 1.0).abs() > f32::EPSILON;
    if scaled {
        let s = item.scale;
        mb = mb.map(|v| v * s);
        cb = cb.map(|b| b.map(|v| v * s));
    }
    page.set("MediaBox", num_array(mb));
    if let Some(cb) = cb {
        page.set("CropBox", num_array(cb));
    }
    if !scaled && item.overlays.is_empty() {
        return Ok(());
    }

    // Inline the resources so adding overlay XObjects never touches resources shared with other pages.
    let mut res = page.get(b"Resources").map(|r| deref_dict(dst, r)).unwrap_or_default();
    let mut xobjs = res.get(b"XObject").map(|x| deref_dict(dst, x)).unwrap_or_default();
    let (mut under, mut over) = (Vec::new(), Vec::new());
    for (i, ov) in item.overlays.iter().enumerate() {
        let sd = sources.get(&ov.source_id).ok_or_else(|| anyhow!("missing overlay source {}", ov.source_id))?;
        let &spid = sd
            .doc
            .get_pages()
            .get(&ov.page_number)
            .ok_or_else(|| anyhow!("{}: overlay page {} not found", sd.file_name, ov.page_number))?;
        let map = maps.entry(ov.source_id).or_default();
        let form = make_form(&sd.doc, dst, map, spid);
        let name = format!("WzOv{i}");
        xobjs.set(name.clone(), Object::Reference(form));
        let s = stream_obj(dst, format!("q /{name} Do Q"));
        if ov.is_underlay { under.push(s) } else { over.push(s) }
    }
    if !xobjs.is_empty() {
        res.set("XObject", xobjs);
    }
    page.set("Resources", res);

    let mut contents = Vec::new();
    if scaled {
        let s = item.scale;
        contents.push(stream_obj(dst, format!("q {s} 0 0 {s} 0 0 cm")));
    }
    contents.extend(under);
    contents.push(stream_obj(dst, "q".into()));
    contents.extend(content_refs(dst, page));
    contents.push(stream_obj(dst, "Q".into()));
    contents.extend(over);
    if scaled {
        contents.push(stream_obj(dst, "Q".into()));
    }
    page.set("Contents", Object::Array(contents));
    Ok(())
}

fn finish_document(mut dst: Document, pages_id: ObjectId, kids: Vec<Object>, meta: &Metadata) -> Result<Vec<u8>> {
    let count = kids.len() as i64;
    dst.objects.insert(
        pages_id,
        dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}.into(),
    );
    let catalog = dst.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    dst.trailer.set("Root", catalog);

    let mut info = Dictionary::new();
    info.set("Producer", Object::string_literal("PDF Wizard"));
    for (k, v) in [
        ("Title", &meta.title),
        ("Author", &meta.author),
        ("Subject", &meta.subject),
        ("Keywords", &meta.keywords),
    ] {
        if !v.is_empty() {
            info.set(k, pdf_text(v));
        }
    }
    let info_id = dst.add_object(info);
    dst.trailer.set("Info", info_id);

    let mut out = Vec::new();
    dst.save_to(&mut out)?;
    Ok(out)
}

pub fn export_pages(
    pages: &[PageItem],
    sources: &HashMap<SourceId, SourceDocument>,
    meta: &Metadata,
) -> Result<Vec<u8>> {
    if pages.is_empty() {
        bail!("no pages to export");
    }
    let mut dst = Document::with_version("1.5");
    let pages_id = dst.new_object_id();
    let mut maps: HashMap<SourceId, IdMap> = HashMap::new();
    let mut kids = Vec::new();

    for item in pages {
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        let mut rotate = 0i64;

        match item.source_id {
            None => {
                page.set(
                    "MediaBox",
                    vec![0.into(), 0.into(), item.width_pt.into(), item.height_pt.into()],
                );
                page.set("Resources", Dictionary::new());
            }
            Some(sid) => {
                let sd = sources.get(&sid).ok_or_else(|| anyhow!("missing source document {sid}"))?;
                let src: &Document = &sd.doc;
                let &spid = src
                    .get_pages()
                    .get(&item.source_page_num)
                    .ok_or_else(|| anyhow!("{}: page {} not found", sd.file_name, item.source_page_num))?;
                let map = maps.entry(sid).or_default();
                let sdict = src.get_dictionary(spid)?;
                for key in [&b"Contents"[..], b"Resources", b"MediaBox", b"CropBox"] {
                    let v = sdict.get(key).ok().cloned().or_else(|| inherited(src, spid, key));
                    if let Some(v) = v {
                        let nv = remap(src, &mut dst, map, &v);
                        page.set(key.to_vec(), nv);
                    }
                }
                if !page.has(b"Resources") {
                    page.set("Resources", Dictionary::new());
                }
                if !page.has(b"MediaBox") {
                    page.set("MediaBox", vec![0.into(), 0.into(), 595.into(), 842.into()]);
                }
                if let Some(r) = inherited(src, spid, b"Rotate") {
                    if let Ok((_, r)) = src.dereference(&r) {
                        rotate = r.as_i64().unwrap_or(0);
                    }
                }
            }
        }
        let total = (rotate + item.rotation as i64).rem_euclid(360);
        if total != 0 {
            page.set("Rotate", total);
        }
        apply_transforms(&mut dst, &mut page, item, rotate, sources, &mut maps)?;
        kids.push(Object::Reference(dst.add_object(page)));
    }
    finish_document(dst, pages_id, kids, meta)
}

pub const A4_W: f32 = 595.0;
pub const A4_H: f32 = 842.0;

pub fn blank_page(uid: u64, w: f32, h: f32) -> PageItem {
    PageItem::blank(uid, w, h)
}

/// Turns an image (PNG/JPEG/WEBP) into a one-page source document, scaled down to fit A4 if larger.
pub fn image_to_source(id: SourceId, file_name: &str, bytes: &[u8]) -> Result<SourceDocument> {
    let img = image::load_from_memory(bytes).map_err(|e| anyhow!("{file_name}: cannot read image ({e})"))?;
    let (pw, ph) = (img.width(), img.height());
    let jpeg_passthrough = image::guess_format(bytes).ok() == Some(image::ImageFormat::Jpeg)
        && matches!(img.color(), image::ColorType::Rgb8 | image::ColorType::L8);

    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => pw as i64,
        "Height" => ph as i64,
        "BitsPerComponent" => 8,
    };
    let mut stream = if jpeg_passthrough {
        let gray = img.color() == image::ColorType::L8;
        dict.set("ColorSpace", if gray { "DeviceGray" } else { "DeviceRGB" });
        dict.set("Filter", "DCTDecode");
        Stream::new(dict, bytes.to_vec())
    } else {
        // Composite any transparency over white; PDF image data is plain RGB.
        let mut raw = Vec::with_capacity((pw * ph * 3) as usize);
        for px in img.to_rgba8().pixels() {
            let a = px[3] as u32;
            for c in 0..3 {
                raw.push(((px[c] as u32 * a + 255 * (255 - a)) / 255) as u8);
            }
        }
        dict.set("ColorSpace", "DeviceRGB");
        Stream::new(dict, raw)
    };
    if !jpeg_passthrough {
        stream.compress()?;
    }

    let (lw, lh) = if pw >= ph { (A4_H, A4_W) } else { (A4_W, A4_H) };
    let s = (lw / pw as f32).min(lh / ph as f32).min(1.0);
    let (w, h) = (pw as f32 * s, ph as f32 * s);

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let img_id = doc.add_object(stream);
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q").into_bytes(),
    ));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content,
        "MediaBox" => num_array([0.0, 0.0, w, h]),
        "Resources" => dictionary! {"XObject" => dictionary! {"Im0" => img_id}},
    });
    let out = finish_document(doc, pages_id, vec![Object::Reference(page)], &Metadata::default())?;
    load_pdf(id, file_name, out)
}

/// Imposes pages 2-up on landscape sheets in saddle-stitch order, honoring `/Rotate` and `CropBox`.
pub fn export_booklet(
    pages: &[PageItem],
    sources: &HashMap<SourceId, SourceDocument>,
    meta: &Metadata,
) -> Result<Vec<u8>> {
    let flat = Document::load_mem(&export_pages(pages, sources, meta)?)?;
    let ids: Vec<ObjectId> = flat.get_pages().values().copied().collect();
    let page_geom = |pid: ObjectId| -> ([f32; 4], i64, f32, f32) {
        let Ok(d) = flat.get_dictionary(pid) else {
            return ([0.0, 0.0, A4_W, A4_H], 0, A4_W, A4_H);
        };
        let mb = box_of(&flat, d, b"MediaBox").unwrap_or([0.0, 0.0, A4_W, A4_H]);
        let bbox = box_of(&flat, d, b"CropBox").unwrap_or(mb);
        let rot = d
            .get(b"Rotate")
            .ok()
            .and_then(|r| flat.dereference(r).ok())
            .and_then(|(_, o)| o.as_i64().ok())
            .unwrap_or(0)
            .rem_euclid(360);
        let (vw, vh) = ((bbox[2] - bbox[0]).abs(), (bbox[3] - bbox[1]).abs());
        let (pw, ph) = if rot == 90 || rot == 270 { (vh, vw) } else { (vw, vh) };
        (bbox, rot, pw, ph)
    };
    let slot_w = ids.iter().map(|&p| page_geom(p).2).fold(0.0, f32::max);
    let slot_h = ids.iter().map(|&p| page_geom(p).3).fold(0.0, f32::max);

    let mut dst = Document::with_version("1.5");
    let pages_id = dst.new_object_id();
    let mut map = IdMap::new();
    let mut kids = Vec::new();
    for sheet in crate::arrange::booklet_order(ids.len()).chunks(2) {
        let mut xobjs = Dictionary::new();
        let mut content = String::new();
        for (slot, entry) in sheet.iter().enumerate() {
            let Some(idx) = entry else { continue };
            let pid = ids[*idx];
            let (bbox, rot, pw, ph) = page_geom(pid);
            let form = make_form_with_bbox(&flat, &mut dst, &mut map, pid, bbox);
            let name = format!("Fm{slot}");
            xobjs.set(name.clone(), Object::Reference(form));
            let sx = slot as f32 * slot_w + (slot_w - pw) / 2.0;
            let sy = (slot_h - ph) / 2.0;
            let [x0, y0, x1, y1] = bbox;
            let (a, b, c, d, e, f) = match rot {
                90 => (0.0, -1.0, 1.0, 0.0, sx - y0, sy + x1),
                180 => (-1.0, 0.0, 0.0, -1.0, sx + x1, sy + y1),
                270 => (0.0, 1.0, -1.0, 0.0, sx + y1, sy - x0),
                _ => (1.0, 0.0, 0.0, 1.0, sx - x0, sy - y0),
            };
            content.push_str(&format!("q {a} {b} {c} {d} {e} {f} cm /{name} Do Q\n"));
        }
        let cid = dst.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let page = dst.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => cid,
            "MediaBox" => num_array([0.0, 0.0, slot_w * 2.0, slot_h]),
            "Resources" => dictionary! {"XObject" => xobjs},
        });
        kids.push(Object::Reference(page));
    }
    finish_document(dst, pages_id, kids, meta)
}

fn unique_extract_path(dir: &Path, stem: &str, index: usize) -> PathBuf {
    let base = dir.join(format!("{stem}_page_{index}.pdf"));
    if !base.exists() {
        return base;
    }
    let mut suffix = 1usize;
    loop {
        let candidate = dir.join(format!("{stem}_page_{index}_{suffix}.pdf"));
        if !candidate.exists() {
            return candidate;
        }
        suffix += 1;
    }
}

pub fn extract_separate(
    pages: &[PageItem],
    sources: &HashMap<SourceId, SourceDocument>,
    dir: &Path,
    meta: &Metadata,
) -> Result<Vec<PathBuf>> {
    if pages.is_empty() {
        bail!("no pages to extract");
    }
    let mut written = Vec::new();
    for (i, item) in pages.iter().enumerate() {
        let stem = item
            .source_id
            .and_then(|s| sources.get(&s))
            .and_then(|s| Path::new(&s.file_name).file_stem().map(|x| x.to_string_lossy().to_string()))
            .unwrap_or_else(|| "blank".to_string());
        let bytes = export_pages(std::slice::from_ref(item), sources, meta)?;
        let path = unique_extract_path(dir, &stem, i + 1);
        std::fs::write(&path, bytes)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
pub(crate) mod testutil {
    use lopdf::{dictionary, Document, Object, Stream};

    /// Build a document whose page n (1-based) content contains the text `P<n>`.
    pub fn make_doc(pages: usize) -> Document {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for i in 0..pages {
            let content = doc.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 12 Tf 72 700 Td (P{}) Tj ET", i + 1).into_bytes(),
            ));
            let page = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Resources" => dictionary! {},
            });
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages_id,
            dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => pages as i64}.into(),
        );
        let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", catalog);
        doc
    }

    pub fn to_bytes(mut doc: Document) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    pub fn make_pdf(pages: usize) -> Vec<u8> {
        to_bytes(make_doc(pages))
    }

    /// One page; /Rotate and /MediaBox live on the parent /Pages node only.
    pub fn make_inherited_pdf() -> Vec<u8> {
        let mut doc = make_doc(1);
        let page_id = *doc.get_pages().get(&1).unwrap();
        let parent = doc.get_dictionary(page_id).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(page_id).unwrap().remove(b"MediaBox");
        let p = doc.get_dictionary_mut(parent).unwrap();
        p.set("Rotate", 90);
        p.set("MediaBox", vec![0.into(), 0.into(), 300.into(), 400.into()]);
        to_bytes(doc)
    }

    pub fn page_text(doc: &Document, n: u32) -> String {
        let id = *doc.get_pages().get(&n).unwrap();
        String::from_utf8_lossy(&doc.get_page_content(id)).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::model::PageItem;

    fn sources(bytes: Vec<u8>, name: &str) -> HashMap<SourceId, SourceDocument> {
        let mut m = HashMap::new();
        m.insert(0, load_pdf(0, name, bytes).unwrap());
        m
    }

    fn item(uid: u64, n: u32) -> PageItem {
        PageItem::from_source(uid, 0, n, 595.0, 842.0, format!("p{n}"))
    }

    #[test]
    fn load_counts_pages() {
        let s = load_pdf(0, "a.pdf", make_pdf(3)).unwrap();
        assert_eq!(s.page_count, 3);
    }

    #[test]
    fn load_rejects_garbage() {
        assert!(load_pdf(0, "x.pdf", b"not a pdf at all".to_vec()).is_err());
    }

    #[test]
    fn export_preserves_order() {
        let src = sources(make_pdf(3), "a.pdf");
        let out = export_pages(&[item(1, 3), item(2, 1)], &src, &Metadata::default()).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 2);
        assert!(page_text(&doc, 1).contains("P3"));
        assert!(page_text(&doc, 2).contains("P1"));
    }

    #[test]
    fn export_applies_rotation() {
        let mut d = make_doc(1);
        let pid = *d.get_pages().get(&1).unwrap();
        d.get_dictionary_mut(pid).unwrap().set("Rotate", 90);
        let src = sources(to_bytes(d), "a.pdf");
        let mut it = item(1, 1);
        it.rotate(90);
        let out = export_pages(&[it], &src, &Metadata::default()).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let pid = *doc.get_pages().get(&1).unwrap();
        assert_eq!(doc.get_dictionary(pid).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 180);
    }

    #[test]
    fn export_resolves_inherited_rotate_and_mediabox() {
        let src = sources(make_inherited_pdf(), "a.pdf");
        assert_eq!(page_size(&src[&0], 1), (300.0, 400.0));
        let out = export_pages(&[item(1, 1)], &src, &Metadata::default()).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let pid = *doc.get_pages().get(&1).unwrap();
        let d = doc.get_dictionary(pid).unwrap();
        assert_eq!(d.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
        let mb = d.get(b"MediaBox").unwrap().as_array().unwrap();
        assert_eq!(mb[2].as_float().unwrap(), 300.0);
        assert_eq!(mb[3].as_float().unwrap(), 400.0);
    }

    #[test]
    fn export_empty_is_error() {
        let src = sources(make_pdf(1), "a.pdf");
        assert!(export_pages(&[], &src, &Metadata::default()).is_err());
    }

    #[test]
    fn export_sets_producer_and_metadata() {
        let src = sources(make_pdf(1), "a.pdf");
        let meta = Metadata { title: "T".into(), ..Default::default() };
        let out = export_pages(&[item(1, 1)], &src, &meta).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let info_id = doc.trailer.get(b"Info").unwrap().as_reference().unwrap();
        let info = doc.get_dictionary(info_id).unwrap();
        assert_eq!(info.get(b"Producer").unwrap().as_str().unwrap(), b"PDF Wizard");
        assert_eq!(info.get(b"Title").unwrap().as_str().unwrap(), b"T");
    }

    #[test]
    fn extract_separate_writes_named_files() {
        let src = sources(make_pdf(3), "report.pdf");
        let dir = std::env::temp_dir().join(format!("pdfwiz_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = extract_separate(&[item(1, 2), item(2, 3)], &src, &dir, &Metadata::default()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_name().unwrap(), "report_page_1.pdf");
        assert_eq!(files[1].file_name().unwrap(), "report_page_2.pdf");
        for f in &files {
            assert_eq!(Document::load(f).unwrap().get_pages().len(), 1);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
    fn boxf(d: &lopdf::Dictionary, key: &[u8]) -> Vec<f32> {
        d.get(key).unwrap().as_array().unwrap().iter().map(|o| o.as_float().unwrap()).collect()
    }
    fn out_page(out: &[u8], n: u32) -> (Document, lopdf::ObjectId) {
        let doc = Document::load_mem(out).unwrap();
        let id = *doc.get_pages().get(&n).unwrap();
        (doc, id)
    }

    #[test]
    fn export_crop_sets_cropbox() {
        let src = sources(make_pdf(1), "a.pdf");
        let mut it = item(1, 1);
        it.crop.left = 0.1;
        it.crop.top = 0.25;
        let out = export_pages(&[it], &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 1);
        let cb = boxf(doc.get_dictionary(id).unwrap(), b"CropBox");
        assert!((cb[0] - 59.5).abs() < 0.01);
        assert!((cb[3] - 631.5).abs() < 0.01);
        assert!((cb[2] - 595.0).abs() < 0.01);
    }

    #[test]
    fn export_scale_scales_boxes_and_wraps_content() {
        let src = sources(make_pdf(1), "a.pdf");
        let mut it = item(1, 1);
        it.scale = 0.5;
        let out = export_pages(&[it], &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 1);
        let mb = boxf(doc.get_dictionary(id).unwrap(), b"MediaBox");
        assert!((mb[2] - 297.5).abs() < 0.01 && (mb[3] - 421.0).abs() < 0.01);
        let t = page_text(&doc, 1);
        assert!(t.contains("0.5 0 0 0.5 0 0 cm") && t.contains("P1"));
    }

    #[test]
    fn export_blank_page_has_a4_size() {
        let src = sources(make_pdf(1), "a.pdf");
        let b = blank_page(9, A4_W, A4_H);
        let out = export_pages(&[b], &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 1);
        assert_eq!(boxf(doc.get_dictionary(id).unwrap(), b"MediaBox"), vec![0.0, 0.0, 595.0, 842.0]);
    }

    #[test]
    fn export_overlay_adds_xobject_and_orders_content() {
        let src = sources(make_pdf(2), "a.pdf");
        for underlay in [false, true] {
            let mut it = item(1, 1);
            it.overlays.push(crate::model::OverlaySpec { source_id: 0, page_number: 2, is_underlay: underlay });
            let out = export_pages(&[it], &src, &Metadata::default()).unwrap();
            let (doc, id) = out_page(&out, 1);
            let (res, _) = doc.get_page_resources(id).unwrap();
            let xo = res.unwrap().get(b"XObject").unwrap().as_dict().unwrap();
            assert!(!xo.is_empty());
            let t = page_text(&doc, 1);
            let (d, p) = (t.find("Do").unwrap(), t.find("P1").unwrap());
            assert_eq!(d < p, underlay, "underlay={underlay}: {t}");
        }
    }

    #[test]
    fn export_overlay_does_not_leak_into_shared_resources() {
        let src = sources(make_pdf(2), "a.pdf");
        let mut with = item(1, 1);
        with.overlays.push(crate::model::OverlaySpec { source_id: 0, page_number: 2, is_underlay: false });
        let out = export_pages(&[with, item(2, 1)], &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 2);
        let (res, _) = doc.get_page_resources(id).unwrap();
        assert!(res.is_none_or(|r| !r.has(b"XObject")));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(w, h).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn image_import_creates_single_page() {
        let s = image_to_source(5, "pic.png", &png(100, 50)).unwrap();
        assert_eq!((s.page_count, page_size(&s, 1)), (1, (100.0, 50.0)));
        let m = HashMap::from([(5, s)]);
        let it = PageItem::from_source(1, 5, 1, 100.0, 50.0, "x".into());
        let out = export_pages(&[it], &m, &Metadata::default()).unwrap();
        assert!(page_text(&Document::load_mem(&out).unwrap(), 1).contains("Do"));
    }

    #[test]
    fn image_import_large_fits_a4_landscape() {
        let s = image_to_source(5, "big.png", &png(2000, 1000)).unwrap();
        let (w, h) = page_size(&s, 1);
        assert!(w <= 842.0 + 0.01 && h <= 595.0 + 0.01 && (w / h - 2.0).abs() < 0.01);
    }

    #[test]
    fn image_import_rejects_garbage() {
        assert!(image_to_source(5, "x.png", b"nope").is_err());
    }

    #[test]
    fn booklet_has_half_as_many_double_width_sheets() {
        let src = sources(make_pdf(6), "a.pdf");
        let pages: Vec<PageItem> = (1..=6).map(|n| item(n as u64, n)).collect();
        let out = export_booklet(&pages, &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 1);
        assert_eq!(doc.get_pages().len(), 4);
        assert_eq!(boxf(doc.get_dictionary(id).unwrap(), b"MediaBox"), vec![0.0, 0.0, 1190.0, 842.0]);
    }

    #[test]
    fn export_crop_honors_intrinsic_rotate() {
        for (rot, expected) in [
            // Visual top=0.1, visual left=0.2 on MediaBox [0, 0, 595, 842]
            (90, [59.5, 168.4, 595.0, 842.0]),    // visual top -> x0, visual left -> y0
            (180, [0.0, 84.2, 476.0, 842.0]),     // visual top -> y0, visual left -> x1
            (270, [0.0, 0.0, 535.5, 673.6]),      // visual top -> x1, visual left -> y1
        ] {
            let mut d = make_doc(1);
            let pid = *d.get_pages().get(&1).unwrap();
            d.get_dictionary_mut(pid).unwrap().set("Rotate", rot);
            let src = sources(to_bytes(d), "a.pdf");
            let mut it = item(1, 1);
            it.crop.top = 0.1;
            it.crop.left = 0.2;
            let out = export_pages(&[it], &src, &Metadata::default()).unwrap();
            let (doc, id) = out_page(&out, 1);
            let cb = boxf(doc.get_dictionary(id).unwrap(), b"CropBox");
            for k in 0..4 {
                assert!(
                    (cb[k] - expected[k]).abs() < 0.1,
                    "rot={rot} idx={k}: got {}, expected {}",
                    cb[k],
                    expected[k]
                );
            }
        }
    }

    #[test]
    fn booklet_honors_rotate_and_crop() {
        let src = sources(make_pdf(4), "a.pdf");
        let mut pages: Vec<PageItem> = (1..=4).map(|n| item(n as u64, n)).collect();
        pages[0].rotate(90);
        pages[0].crop = CropMargins { left: 0.1, right: 0.1, top: 0.0, bottom: 0.0 };
        let out = export_booklet(&pages, &src, &Metadata::default()).unwrap();
        let (doc, id) = out_page(&out, 1);
        // Rotated 90 deg page has visual width 842 (taller than 595), so slot_w=842 and sheet width=1684.
        let mb = boxf(doc.get_dictionary(id).unwrap(), b"MediaBox");
        assert!((mb[2] - 1684.0).abs() < 0.1, "got width {}", mb[2]);
        assert!(page_text(&doc, 1).contains("cm"));
    }

    #[test]
    fn extract_separate_avoids_overwriting_existing_file() {
        let src = sources(make_pdf(1), "report.pdf");
        let dir = std::env::temp_dir().join(format!("pdfwiz_ext_unique_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("report_page_1.pdf");
        std::fs::write(&existing, b"keep-me").unwrap();

        let files = extract_separate(&[item(1, 1)], &src, &dir, &Metadata::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "report_page_1_1.pdf");
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep-me");
        std::fs::remove_dir_all(&dir).ok();
    }
}
