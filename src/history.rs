use crate::model::PageItem;

/// Snapshot-based undo/redo over the page list.
pub struct History {
    stack: Vec<Vec<PageItem>>,
    pos: usize,
}

impl History {
    pub fn new(initial: Vec<PageItem>) -> Self {
        History { stack: vec![initial], pos: 0 }
    }

    pub fn push(&mut self, pages: Vec<PageItem>) {
        self.stack.truncate(self.pos + 1);
        self.stack.push(pages);
        self.pos += 1;
    }

    pub fn undo(&mut self) -> Option<Vec<PageItem>> {
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        Some(self.stack[self.pos].clone())
    }

    pub fn redo(&mut self) -> Option<Vec<PageItem>> {
        if self.pos + 1 >= self.stack.len() {
            return None;
        }
        self.pos += 1;
        Some(self.stack[self.pos].clone())
    }

    pub fn current(&self) -> &Vec<PageItem> {
        &self.stack[self.pos]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageItem;

    fn list(n: usize) -> Vec<PageItem> {
        (0..n).map(|i| PageItem::blank(i as u64, 595.0, 842.0)).collect()
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut h = History::new(list(1));
        h.push(list(2));
        assert_eq!(h.undo().unwrap().len(), 1);
        assert_eq!(h.redo().unwrap().len(), 2);
        assert_eq!(h.current().len(), 2);
    }

    #[test]
    fn push_clears_redo() {
        let mut h = History::new(list(1));
        h.push(list(2));
        h.undo();
        h.push(list(3));
        assert!(h.redo().is_none());
    }

    #[test]
    fn undo_at_start_returns_none() {
        let mut h = History::new(list(1));
        assert!(h.undo().is_none());
    }
}
