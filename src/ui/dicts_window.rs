//! The dictionary tree window (B-02): state of the window; the drawing and
//! the operations live in `app.rs`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::dicts::{Role, Tree};

/// What is selected in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Group(Role),
    Node(Role, PathBuf),
}

/// What the detail line shows for a file (read once per window opening).
#[derive(Debug, Clone, Default)]
pub struct NodeInfo {
    pub lines: usize,
    /// Old data (B-07).
    pub old: usize,
    pub version: Option<u32>,
    pub modified: Option<i64>,
}

#[derive(Default)]
pub struct DictsWindow {
    pub open: bool,
    pub needs_place: bool,
    pub tree: Tree,
    /// The tree as loaded: `dirty` compares against it.
    pub loaded: Tree,
    pub selected: Option<Selection>,
    /// The document the tree was built for.
    pub doc_serial: Option<u64>,
    pub info: HashMap<PathBuf, NodeInfo>,
    pub message: Option<String>,
    /// Fermer with changes: the F-04 prompt.
    pub confirm_close: bool,
}

impl DictsWindow {
    pub fn dirty(&self) -> bool {
        self.tree != self.loaded
    }

    /// Move the selection up or down (a group among the groups, a file
    /// inside its group).
    pub fn move_selected(&mut self, dir: i32) -> bool {
        match self.selected.clone() {
            Some(Selection::Group(r)) => self.tree.move_group(r, dir),
            Some(Selection::Node(r, p)) => self.tree.move_node(r, &p, dir),
            None => false,
        }
    }

    /// Whether the selection is something the user added (removable).
    pub fn selected_removable(&self) -> bool {
        match &self.selected {
            Some(Selection::Node(r, p)) => self
                .tree
                .groups
                .iter()
                .find(|g| g.role == *r)
                .is_some_and(|g| {
                    g.nodes.iter().any(|n| n.added && &n.path == p)
                        || g.added_dirs.iter().any(|d| d == p)
                }),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dicts::{Group, Node};

    #[test]
    fn selection_moves_and_removability() {
        let mk = |role: Role, name: &str, added: bool| Node {
            role,
            path: PathBuf::from(name),
            enabled: true,
            added,
        };
        let tree = Tree {
            groups: vec![
                Group {
                    role: Role::Game,
                    enabled: true,
                    nodes: vec![
                        mk(Role::Game, "/g/a.ximdb", false),
                        mk(Role::Game, "/g/b.ximdb", false),
                    ],
                    added_dirs: vec![],
                },
                Group {
                    role: Role::External,
                    enabled: true,
                    nodes: vec![mk(Role::External, "/x/c.ximdb", true)],
                    added_dirs: vec![],
                },
            ],
        };
        let mut w = DictsWindow {
            tree: tree.clone(),
            loaded: tree,
            ..Default::default()
        };
        assert!(!w.dirty());
        w.selected = Some(Selection::Node(Role::Game, PathBuf::from("/g/b.ximdb")));
        assert!(w.move_selected(-1));
        assert!(w.dirty());
        assert_eq!(w.tree.groups[0].nodes[0].path, PathBuf::from("/g/b.ximdb"));
        assert!(!w.selected_removable());
        w.selected = Some(Selection::Node(Role::External, PathBuf::from("/x/c.ximdb")));
        assert!(w.selected_removable());
        w.selected = Some(Selection::Group(Role::External));
        assert!(w.move_selected(-1));
        assert_eq!(w.tree.groups[0].role, Role::External);
        assert!(!w.move_selected(-1));
    }
}
