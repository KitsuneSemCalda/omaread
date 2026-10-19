//! EPUB CFI — the structural subset Omaread needs.
//!
//! Positions anchor to CFI rather than page numbers because pages change when
//! the font size does (CONTEXT.md §4).
//!
//! ponytail: structural steps only — `epubcfi(/6/N!/4/2/6)`, with an optional
//! `:offset` parsed but not generated. No assertions, ranges, or side bias.
//! Round-trips exactly against our own DOM, which is what persistence needs;
//! interop with other readers is best-effort. Add character offsets when
//! highlights need sub-paragraph precision (Phase 7).

use blitz_dom::{BaseDocument, NodeData};

/// A parsed CFI: which spine item, and the element path inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfi {
    /// Zero-based spine index.
    pub spine: usize,
    /// Element steps inside the content document, in CFI numbering (2 = first
    /// element child, 4 = second, …).
    pub steps: Vec<usize>,
    /// Character offset into a text node, when present.
    pub offset: Option<usize>,
}

impl Cfi {
    pub fn to_string(&self) -> String {
        let mut s = format!("epubcfi(/6/{}!", (self.spine + 1) * 2);
        for step in &self.steps {
            s.push('/');
            s.push_str(&step.to_string());
        }
        if let Some(o) = self.offset {
            s.push(':');
            s.push_str(&o.to_string());
        }
        s.push(')');
        s
    }

    pub fn parse(s: &str) -> Option<Self> {
        let inner = s.strip_prefix("epubcfi(")?.strip_suffix(')')?;
        let (spine_part, doc_part) = inner.split_once('!')?;

        // Spine step is the last number of the pre-`!` path: /6/14 -> 14.
        let spine_step: usize = spine_part.rsplit('/').find(|p| !p.is_empty())?.parse().ok()?;
        if spine_step < 2 {
            return None;
        }
        let spine = spine_step / 2 - 1;

        let (path, offset) = match doc_part.split_once(':') {
            Some((p, o)) => (p, Some(o.parse().ok()?)),
            None => (doc_part, None),
        };

        let steps: Option<Vec<usize>> = path
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().ok())
            .collect();
        let steps = steps?;
        if steps.is_empty() {
            return None;
        }

        Some(Cfi { spine, steps, offset })
    }
}

/// Build a CFI for a node in a laid-out document.
pub fn of_node(dom: &BaseDocument, node_id: usize, spine: usize) -> Option<Cfi> {
    let mut steps = Vec::new();
    let mut id = node_id;

    loop {
        let node = dom.get_node(id)?;
        let Some(parent_id) = node.parent else { break };
        let parent = dom.get_node(parent_id)?;

        // CFI counts element children from 1, doubled.
        let index = parent
            .children
            .iter()
            .filter(|c| {
                dom.get_node(**c)
                    .is_some_and(|n| matches!(n.data, NodeData::Element(_)))
            })
            .position(|c| *c == id)?;
        steps.push((index + 1) * 2);
        id = parent_id;
    }

    steps.reverse();
    if steps.is_empty() {
        return None;
    }
    Some(Cfi { spine, steps, offset: None })
}

/// A CFI for a point in a laid-out document: a text element and the visible
/// character in it.
///
/// An anonymous block (`chapter::kids`) is not in the document, so a CFI cannot
/// name one. It names the element blitz split instead, and the offset runs on
/// through that element's anonymous blocks in order. `locate` undoes this.
pub fn at(dom: &BaseDocument, node: usize, offset: Option<usize>, spine: usize) -> Option<Cfi> {
    use crate::chapter::{anonymous_blocks, text_layout, visible_count};
    let mut node = node;
    let mut offset = offset;
    if matches!(dom.get_node(node)?.data, NodeData::AnonymousBlock(_)) {
        let owner = dom.get_node(node)?.parent?;
        let before: usize = anonymous_blocks(dom, owner)
            .into_iter()
            .take_while(|&b| b != node)
            .filter_map(|b| text_layout(dom, b))
            .map(|tl| visible_count(&tl.text))
            .sum();
        (node, offset) = (owner, offset.map(|o| o + before));
    }
    let mut c = of_node(dom, node, spine)?;
    c.offset = offset;
    Some(c)
}

/// Where a CFI points in a laid-out document: the text element, and the
/// visible character in it. The inverse of `at`.
pub fn locate(dom: &BaseDocument, cfi: &Cfi) -> Option<(usize, Option<usize>)> {
    use crate::chapter::{anonymous_blocks, text_layout, visible_count};
    let node = resolve(dom, cfi)?;
    let blocks = anonymous_blocks(dom, node);
    let Some(mut off) = cfi.offset.filter(|_| !blocks.is_empty()) else {
        return Some((node, cfi.offset));
    };
    for (i, &b) in blocks.iter().enumerate() {
        let n = text_layout(dom, b).map_or(0, |tl| visible_count(&tl.text));
        if off < n || i + 1 == blocks.len() {
            return Some((b, Some(off)));
        }
        off -= n;
    }
    None
}

/// Walk a CFI's steps back to a node id.
pub fn resolve(dom: &BaseDocument, cfi: &Cfi) -> Option<usize> {
    let mut id = dom.root_node().id;

    for &step in &cfi.steps {
        if step < 2 {
            return None;
        }
        let node = dom.get_node(id)?;
        let nth = step / 2 - 1;
        id = *node
            .children
            .iter()
            .filter(|c| {
                dom.get_node(**c)
                    .is_some_and(|n| matches!(n.data, NodeData::Element(_)))
            })
            .nth(nth)?;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_string() {
        let c = Cfi { spine: 6, steps: vec![4, 2, 6], offset: None };
        let s = c.to_string();
        assert_eq!(s, "epubcfi(/6/14!/4/2/6)");
        assert_eq!(Cfi::parse(&s), Some(c));
    }

    #[test]
    fn parses_a_character_offset() {
        let c = Cfi::parse("epubcfi(/6/14!/4/2/1:341)").unwrap();
        assert_eq!(c.spine, 6);
        assert_eq!(c.steps, vec![4, 2, 1]);
        assert_eq!(c.offset, Some(341));
        assert_eq!(c.to_string(), "epubcfi(/6/14!/4/2/1:341)");
    }

    #[test]
    fn rejects_junk() {
        for bad in [
            "",
            "epubcfi()",
            "epubcfi(/6/14)",             // no indirection
            "epubcfi(/6/0!/4)",           // spine step below 2
            "epubcfi(/6/14!/x/2)",        // non-numeric step
            "epubcfi(/6/14!)",            // no document path
            "/6/14!/4/2",                 // missing wrapper
            "epubcfi(/6/14!/4/2:abc)",    // non-numeric offset
        ] {
            assert!(Cfi::parse(bad).is_none(), "accepted junk: {bad:?}");
        }
    }

    #[test]
    fn spine_index_survives_the_round_trip() {
        for spine in [0usize, 1, 9, 130] {
            let c = Cfi { spine, steps: vec![4], offset: None };
            assert_eq!(Cfi::parse(&c.to_string()).unwrap().spine, spine);
        }
    }
}
