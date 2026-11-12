#![allow(dead_code)]

pub type SourceId = usize;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CropMargins {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlaySpec {
    pub source_id: SourceId,
    pub page_number: u32,
    pub is_underlay: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
}

#[derive(Clone, Debug)]
pub struct PageItem {
    pub uid: u64,
    pub source_id: Option<SourceId>,
    pub source_page_num: u32,
    pub rotation: i32,
    pub crop: CropMargins,
    pub scale: f32,
    pub width_pt: f32,
    pub height_pt: f32,
    pub overlays: Vec<OverlaySpec>,
    pub label: String,
}

impl PageItem {
    pub fn blank(uid: u64, width_pt: f32, height_pt: f32) -> Self {
        PageItem {
            uid,
            source_id: None,
            source_page_num: 0,
            rotation: 0,
            crop: CropMargins::default(),
            scale: 1.0,
            width_pt,
            height_pt,
            overlays: Vec::new(),
            label: "Blank page".to_string(),
        }
    }

    pub fn rotate(&mut self, delta: i32) {
        self.rotation = (self.rotation + delta).rem_euclid(360);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_wraps_negative() {
        let mut p = PageItem::blank(1, 595.0, 842.0);
        p.rotate(-90);
        assert_eq!(p.rotation, 270);
    }

    #[test]
    fn rotate_accumulates() {
        let mut p = PageItem::blank(1, 595.0, 842.0);
        p.rotate(90);
        p.rotate(180);
        assert_eq!(p.rotation, 270);
    }
}
