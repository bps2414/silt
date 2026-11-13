//! The largest files and folders, with the filters of the Large files screen. System items and
//! what already sits in the Recycle Bin never appear.

use serde::Serialize;

use crate::Category;
use crate::tree::{NONE, NodeId, Tree, flags};

const DAY: u64 = 86_400;

#[derive(Clone, Debug)]
pub struct Filter {
    /// Only these categories; empty means all of them (System and Trash never count).
    pub categories: Vec<Category>,
    pub min_size: u64,
    /// Not modified for at least this many days. The modification date, because Windows keeps
    /// no reliable last-access date. 0 means any age.
    pub min_age_days: u32,
    pub limit: usize,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            categories: Vec::new(),
            min_size: 0,
            min_age_days: 0,
            limit: 200,
        }
    }
}

impl Filter {
    /// "Large and old": the Overview's Large files line. Personal files only: apps, games,
    /// toolchains and caches have their own actions.
    pub fn large_and_old() -> Self {
        Filter {
            categories: vec![
                Category::Documents,
                Category::Photos,
                Category::Videos,
                Category::Music,
                Category::Other,
            ],
            min_size: 100 << 20,
            min_age_days: 180,
            limit: usize::MAX,
        }
    }

    fn accepts(&self, category: Category, alloc: u64, mtime: u32, now: u64) -> bool {
        if matches!(category, Category::System | Category::Trash)
            || alloc == 0
            || alloc < self.min_size
        {
            return false;
        }
        if !self.categories.is_empty() && !self.categories.contains(&category) {
            return false;
        }
        self.min_age_days == 0 || u64::from(mtime) + u64::from(self.min_age_days) * DAY <= now
    }
}

/// One row of the list.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    #[serde(skip)]
    pub id: NodeId,
    pub path: String,
    pub alloc: u64,
    pub logical: u64,
    /// Unix seconds (the newest inside, for a folder).
    pub mtime: u32,
    pub category: Category,
    pub folder: bool,
    pub files: u32,
}

impl Item {
    pub fn new(tree: &Tree, id: NodeId, category: Category) -> Self {
        let n = tree.node(id);
        Item {
            id,
            path: tree.path(id),
            alloc: n.alloc,
            logical: n.logical,
            mtime: n.mtime,
            category,
            folder: n.is_dir(),
            files: n.files,
        }
    }
}

/// The largest files that pass `f`, biggest first.
pub fn files(tree: &Tree, f: &Filter, now: u64) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = tree
        .descendants(tree.root)
        .filter(|&id| {
            let n = tree.node(id);
            !n.is_dir() && !n.has(flags::SYNTHETIC) && f.accepts(n.category, n.alloc, n.mtime, now)
        })
        .collect();
    top(tree, &mut v, f.limit);
    v
}

/// Bytes and count of every file that passes `f` (no limit).
pub fn total(tree: &Tree, f: &Filter, now: u64) -> (u64, u64) {
    tree.descendants(tree.root).fold((0, 0), |(b, n), id| {
        let node = tree.node(id);
        if !node.is_dir()
            && !node.has(flags::SYNTHETIC)
            && f.accepts(node.category, node.alloc, node.mtime, now)
        {
            (b + node.alloc, n + 1)
        } else {
            (b, n)
        }
    })
}

/// The largest folders that pass `f`, biggest first. A folder that is mostly one subfolder is
/// left out: the subfolder is the answer.
pub fn folders(tree: &Tree, dominant: &[Category], f: &Filter, now: u64) -> Vec<NodeId> {
    let mut v = Vec::new();
    for id in tree.descendants(tree.root) {
        let n = tree.node(id);
        if id == tree.root
            || !n.is_dir()
            || n.has(flags::LINK | flags::SYNTHETIC)
            || n.category == Category::System
        {
            continue;
        }
        if !f.accepts(dominant[id as usize], n.alloc, n.mtime, now) {
            continue;
        }
        // Children are sorted by size, so the first folder among them is the largest.
        if let Some(big) = tree.children(id).find(|&c| tree.node(c).is_dir())
            && tree.node(big).alloc * 2 > n.alloc
        {
            continue;
        }
        v.push(id);
    }
    top(tree, &mut v, f.limit);
    v
}

fn top(tree: &Tree, v: &mut Vec<NodeId>, limit: usize) {
    let key = |id: &NodeId| std::cmp::Reverse(tree.node(*id).alloc);
    if v.len() > limit {
        v.select_nth_unstable_by_key(limit, key);
        v.truncate(limit);
    }
    v.sort_unstable_by_key(key);
}

/// The category of each node: a file's own, and for a folder the one holding most of its bytes.
/// One pass; the app computes it once per scan.
pub fn dominant_categories(tree: &Tree) -> Vec<Category> {
    let mut out: Vec<Category> = tree.nodes.iter().map(|n| n.category).collect();
    struct Frame {
        node: NodeId,
        next: NodeId,
        acc: [u64; 11],
    }
    let mut stack = vec![Frame {
        node: tree.root,
        next: tree.node(tree.root).first_child,
        acc: [0; 11],
    }];
    while let Some(top) = stack.last_mut() {
        if top.next != NONE {
            let c = top.next;
            let n = tree.node(c);
            top.next = n.next_sibling;
            if n.is_dir() && n.first_child != NONE {
                stack.push(Frame {
                    node: c,
                    next: n.first_child,
                    acc: [0; 11],
                });
            } else {
                top.acc[n.category.index()] += n.alloc;
            }
            continue;
        }
        let done = stack.pop().expect("non-empty");
        let best = (0..11)
            .max_by_key(|&i| done.acc[i])
            .filter(|&i| done.acc[i] > 0);
        if let Some(i) = best {
            out[done.node as usize] = Category::ALL[i];
        }
        if let Some(parent) = stack.last_mut() {
            for (p, a) in parent.acc.iter_mut().zip(done.acc) {
                *p += a;
            }
        }
    }
    out
}
