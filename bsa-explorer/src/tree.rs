//! Build a folder tree from the flat, `/`-separated entry paths of an archive.
//!
//! Each node keeps the indices of the files stored **directly** in that folder
//! (into the archive's `entries` slice), the child folders, and a recursive file
//! count for display. The tree is what the left-hand panel renders; selecting a
//! node filters the file table to that folder.

use crate::archive::ArchiveEntry;

/// A folder in the archive tree.
#[derive(Default)]
pub struct TreeNode {
    /// This folder's own name (last path component); empty for the root.
    pub name: String,
    /// Full `/`-separated path of this folder (empty for the root).
    pub full: String,
    /// Child folders, sorted by name (case-insensitive).
    pub children: Vec<TreeNode>,
    /// Indices (into `entries`) of files directly in this folder, sorted by path.
    pub files: Vec<usize>,
    /// Number of files in this folder and all its descendants.
    pub subtree_files: usize,
}

/// Build the root node of the folder tree for `entries`.
pub fn build(entries: &[ArchiveEntry]) -> TreeNode {
    let mut root = TreeNode::default();
    for (i, e) in entries.iter().enumerate() {
        let comps: Vec<&str> = e.path.split('/').collect();
        let folder_count = comps.len().saturating_sub(1); // last component is the file
        let mut node = &mut root;
        let mut acc = String::new();
        for f in &comps[..folder_count] {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(f);
            let pos = node.children.iter().position(|c| c.name == *f);
            let idx = match pos {
                Some(p) => p,
                None => {
                    node.children.push(TreeNode {
                        name: (*f).to_string(),
                        full: acc.clone(),
                        ..Default::default()
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
        }
        node.files.push(i);
    }
    sort_node(&mut root, entries);
    count_node(&mut root);
    root
}

fn sort_node(node: &mut TreeNode, entries: &[ArchiveEntry]) {
    node.children.sort_by_key(|c| c.name.to_lowercase());
    node.files
        .sort_by(|&a, &b| entries[a].path.cmp(&entries[b].path));
    for c in &mut node.children {
        sort_node(c, entries);
    }
}

fn count_node(node: &mut TreeNode) -> usize {
    let mut total = node.files.len();
    for c in &mut node.children {
        total += count_node(c);
    }
    node.subtree_files = total;
    total
}

/// Find a folder node by its full `/`-separated path (empty path → root).
pub fn find<'a>(root: &'a TreeNode, full: &str) -> Option<&'a TreeNode> {
    if full.is_empty() {
        return Some(root);
    }
    let mut node = root;
    for comp in full.split('/') {
        node = node.children.iter().find(|c| c.name == comp)?;
    }
    Some(node)
}

/// Collect every file index in a folder subtree (folder + all descendants),
/// used by "extract this folder" and the recursive view.
pub fn subtree_files(node: &TreeNode, out: &mut Vec<usize>) {
    out.extend_from_slice(&node.files);
    for c in &node.children {
        subtree_files(c, out);
    }
}
