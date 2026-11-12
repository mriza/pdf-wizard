#![allow(dead_code)]
use crate::model::PageItem;

/// 0-based indices of odd-numbered pages (page 1, 3, 5, ...).
pub fn select_odd(pages: &[PageItem]) -> Vec<usize> {
    (0..pages.len()).step_by(2).collect()
}

/// 0-based indices of even-numbered pages (page 2, 4, 6, ...).
pub fn select_even(pages: &[PageItem]) -> Vec<usize> {
    (1..pages.len()).step_by(2).collect()
}

pub fn invert_selection(len: usize, sel: &[usize]) -> Vec<usize> {
    (0..len).filter(|i| !sel.contains(i)).collect()
}

pub fn reverse(pages: &mut Vec<PageItem>) {
    pages.reverse();
}

/// Swaps pages 1<->2, 3<->4, ...; a trailing odd page stays in place.
pub fn swap_odd_even(pages: &mut Vec<PageItem>) {
    for pair in pages.chunks_exact_mut(2) {
        pair.swap(0, 1);
    }
}

/// Moves the selected pages (keeping their relative order) so they sit before the
/// element that was at `target` in the original list (`target == len` means the end).
pub fn move_pages(pages: &mut Vec<PageItem>, selected: &[usize], target: usize) {
    let mut sel: Vec<usize> = selected.iter().copied().filter(|&i| i < pages.len()).collect();
    sel.sort_unstable();
    sel.dedup();
    let before = sel.iter().filter(|&&i| i < target).count();
    let mut moved = Vec::with_capacity(sel.len());
    for &i in sel.iter().rev() {
        moved.push(pages.remove(i));
    }
    moved.reverse();
    let at = (target - before).min(pages.len());
    for (k, p) in moved.into_iter().enumerate() {
        pages.insert(at + k, p);
    }
}

/// Shifts selected pages by 1 position left (`delta < 0`) or right (`delta > 0`),
/// returning the updated sorted selected indices.
pub fn step_move_pages(pages: &mut Vec<PageItem>, selected: &[usize], delta: isize) -> Vec<usize> {
    let mut sel: Vec<usize> = selected.iter().copied().filter(|&i| i < pages.len()).collect();
    sel.sort_unstable();
    sel.dedup();
    if sel.is_empty() || delta == 0 {
        return sel;
    }
    let mut out = std::collections::BTreeSet::new();
    if delta < 0 {
        for i in sel {
            if i > 0 && !out.contains(&(i - 1)) {
                pages.swap(i, i - 1);
                out.insert(i - 1);
            } else {
                out.insert(i);
            }
        }
    } else {
        for i in sel.into_iter().rev() {
            if i + 1 < pages.len() && !out.contains(&(i + 1)) {
                pages.swap(i, i + 1);
                out.insert(i + 1);
            } else {
                out.insert(i);
            }
        }
    }
    out.into_iter().collect()
}

/// Saddle-stitch order over `n` pages padded to a multiple of 4.
/// `None` is a blank padding page. Output is read in groups of 4 = one sheet (front: a,b; back: c,d).
pub fn booklet_order(n: usize) -> Vec<Option<usize>> {
    let padded = n.div_ceil(4) * 4;
    let mut out = Vec::with_capacity(padded);
    for k in 0..padded / 4 {
        for idx in [padded - 1 - 2 * k, 2 * k, 2 * k + 1, padded - 2 - 2 * k] {
            out.push(if idx < n { Some(idx) } else { None });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> Vec<PageItem> {
        (0..n).map(|i| PageItem::blank(i as u64, 595.0, 842.0)).collect()
    }
    fn uids(p: &[PageItem]) -> Vec<u64> {
        p.iter().map(|x| x.uid).collect()
    }

    #[test]
    fn select_odd_even() {
        let p = list(5);
        assert_eq!(select_odd(&p), vec![0, 2, 4]);
        assert_eq!(select_even(&p), vec![1, 3]);
    }

    #[test]
    fn invert_selection_basic() {
        assert_eq!(invert_selection(5, &[0, 3]), vec![1, 2, 4]);
    }

    #[test]
    fn swap_odd_even_keeps_trailing() {
        let mut p = list(5);
        swap_odd_even(&mut p);
        assert_eq!(uids(&p), vec![1, 0, 3, 2, 4]);
    }

    #[test]
    fn reverse_reverses() {
        let mut p = list(3);
        reverse(&mut p);
        assert_eq!(uids(&p), vec![2, 1, 0]);
    }

    #[test]
    fn move_pages_to_end() {
        let mut p = list(3);
        move_pages(&mut p, &[0], 3);
        assert_eq!(uids(&p), vec![1, 2, 0]);
    }

    #[test]
    fn move_pages_multiple_forward_keeps_order() {
        let mut p = list(5);
        move_pages(&mut p, &[3, 4], 1);
        assert_eq!(uids(&p), vec![0, 3, 4, 1, 2]);
    }

    #[test]
    fn move_pages_onto_itself_is_noop() {
        let mut p = list(4);
        move_pages(&mut p, &[1, 2], 2);
        assert_eq!(uids(&p), vec![0, 1, 2, 3]);
    }

    #[test]
    fn booklet_order_6_pads_to_8() {
        assert_eq!(
            booklet_order(6),
            vec![None, Some(0), Some(1), None, Some(5), Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn booklet_order_4_is_one_sheet() {
        assert_eq!(booklet_order(4), vec![Some(3), Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn step_move_left_and_right() {
        let mut p = list(5);
        let sel = step_move_pages(&mut p, &[1, 2], -1);
        assert_eq!(uids(&p), vec![1, 2, 0, 3, 4]);
        assert_eq!(sel, vec![0, 1]);

        let sel = step_move_pages(&mut p, &sel, 1);
        assert_eq!(uids(&p), vec![0, 1, 2, 3, 4]);
        assert_eq!(sel, vec![1, 2]);
    }

    #[test]
    fn step_move_at_boundary_is_noop() {
        let mut p = list(4);
        assert_eq!(step_move_pages(&mut p, &[0, 1], -1), vec![0, 1]);
        assert_eq!(uids(&p), vec![0, 1, 2, 3]);
        assert_eq!(step_move_pages(&mut p, &[2, 3], 1), vec![2, 3]);
        assert_eq!(uids(&p), vec![0, 1, 2, 3]);
    }

    #[test]
    fn step_move_non_contiguous_preserves_order() {
        let mut p = list(5);
        let sel = step_move_pages(&mut p, &[1, 3], -1);
        assert_eq!(uids(&p), vec![1, 0, 3, 2, 4]);
        assert_eq!(sel, vec![0, 2]);
    }
}

