#![allow(dead_code)]
use crate::model::{Metadata, PageItem, SourceId};
use anyhow::{anyhow, bail, Result};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
                    if let Some((_, r)) = src.dereference(&r).ok() {
                        rotate = r.as_i64().unwrap_or(0);
                    }
                }
            }
        }
        let total = (rotate + item.rotation as i64).rem_euclid(360);
        if total != 0 {
            page.set("Rotate", total);
        }
        kids.push(Object::Reference(dst.add_object(page)));
    }

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
        let path = dir.join(format!("{stem}_page_{}.pdf", i + 1));
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
}
