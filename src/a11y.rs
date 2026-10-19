//! Screen-reader support: the window as an AccessKit tree (CONTEXT.md §7).
//!
//! The tree is built from what the app knows is on screen, not from its
//! documents. Five documents are painted over each other — page, HUD, contents,
//! grid, menu — and blitz-dom's own builder walks one of them, with no names,
//! bounds or actions. The app knows what each control *is*; the tree says so.
//!
//! Every update is a whole tree. It is a few dozen nodes, the adapter raises
//! events only for nodes that changed, and none of it runs unless an assistive
//! technology has asked for the tree.
//!
//! ponytail: no bounds. Focus, names and actions are what a screen reader
//! needs; a magnifier following focus would need `set_bounds` on each node.

use accesskit::{Action, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

/// What a node stands for, so an action request can find its way back.
/// The id is the target: kind in the top byte, index below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Root,
    /// The book list, the contents list, or the open menu's own node.
    List,
    Menu,
    Toolbar,
    /// A HUD control, by its index in `HUD`.
    Hud(usize),
    /// A row of the open menu, in document order.
    MenuRow(usize),
    /// A book card, a contents entry, a search hit or a mark.
    Item(usize),
    Gear,
    Search,
    /// Text being typed: a folder path, a note.
    Typing,
    /// A footnote on show.
    Note,
    /// The page on screen, keyed by chapter and page so that turning gives it a
    /// new id and moves focus — which is what makes a screen reader read it.
    Page(u64),
    /// Block `k` of the page keyed `page`.
    Text(u64, u16),
}

/// The HUD's controls, in reading order.
pub const HUD: [&str; 10] = [
    "bookmark", "contents", "highlight", "unhighlight", "smaller", "bigger", "columns", "library",
    "back", "readout",
];

pub fn id(t: Target) -> NodeId {
    let (kind, index) = match t {
        Target::Root => (0, 0),
        Target::List => (1, 0),
        Target::Menu => (2, 0),
        Target::Toolbar => (3, 0),
        Target::Hud(i) => (4, i as u64),
        Target::MenuRow(i) => (5, i as u64),
        Target::Item(i) => (6, i as u64),
        Target::Gear => (7, 0),
        Target::Search => (8, 0),
        Target::Typing => (9, 0),
        Target::Note => (10, 0),
        Target::Page(p) => (11, p),
        Target::Text(p, k) => (12, p << 16 | k as u64),
    };
    NodeId(kind << 56 | index & ((1 << 56) - 1))
}

/// The target an action request names, for the kinds that act.
pub fn target(id: NodeId) -> Option<Target> {
    let index = id.0 & ((1 << 56) - 1);
    Some(match id.0 >> 56 {
        4 => Target::Hud(index as usize),
        5 => Target::MenuRow(index as usize),
        6 => Target::Item(index as usize),
        7 => Target::Gear,
        _ => return None,
    })
}

pub struct Tree {
    nodes: Vec<(NodeId, Node)>,
    focus: NodeId,
}

impl Tree {
    pub fn new(label: &str) -> Self {
        let mut root = Node::new(Role::Window);
        root.set_label(label);
        Self { nodes: vec![(id(Target::Root), root)], focus: id(Target::Root) }
    }

    /// Add a node under `parent`, which must already be in the tree.
    pub fn add(&mut self, parent: Target, t: Target, role: Role, label: &str) -> &mut Node {
        let (pid, me) = (id(parent), id(t));
        if let Some((_, p)) = self.nodes.iter_mut().find(|(i, _)| *i == pid) {
            p.push_child(me);
        }
        let mut node = Node::new(role);
        if !label.is_empty() {
            node.set_label(label);
        }
        if matches!(role, Role::Button | Role::MenuItem | Role::MenuItemRadio | Role::ListItem) {
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
        }
        self.nodes.push((me, node));
        &mut self.nodes.last_mut().unwrap().1
    }

    /// Where keyboard focus is. Ignored if that node was never added.
    pub fn focus(&mut self, t: Target) {
        self.focus = id(t);
    }

    pub fn finish(self) -> TreeUpdate {
        let focus = match self.nodes.iter().any(|(i, _)| *i == self.focus) {
            true => self.focus,
            false => id(Target::Root),
        };
        TreeUpdate {
            nodes: self.nodes,
            tree: Some(TreeInfo {
                root: id(Target::Root),
                toolkit_name: Some("Omaread".into()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_never_collide() {
        for t in [Target::Hud(3), Target::MenuRow(0), Target::Item(361), Target::Gear] {
            assert_eq!(target(id(t)), Some(t));
        }
        assert_eq!(target(id(Target::Page(7))), None, "a page does not act");
        assert_ne!(id(Target::Item(1)), id(Target::MenuRow(1)));
        assert_ne!(id(Target::Text(1, 0)), id(Target::Text(2, 0)), "a new page, new blocks");
    }

    #[test]
    fn the_tree_hangs_together() {
        let mut t = Tree::new("Omaread");
        t.add(Target::Root, Target::List, Role::List, "Library");
        t.add(Target::List, Target::Item(0), Role::ListItem, "A book");
        t.focus(Target::Item(0));
        let u = t.finish();
        assert_eq!(u.focus, id(Target::Item(0)));
        let list = &u.nodes.iter().find(|(i, _)| *i == id(Target::List)).unwrap().1;
        assert_eq!(list.children(), &[id(Target::Item(0))]);

        let mut t = Tree::new("Omaread");
        t.focus(Target::Item(9));
        assert_eq!(t.finish().focus, id(Target::Root), "focus on a missing node falls back");
    }
}
