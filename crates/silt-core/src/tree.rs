//! The in-memory tree: one arena of fixed-size nodes with `u32` links, names in one buffer,
//! children sorted by allocated size once the scan is aggregated.

use crate::category::{Category, Classifier, DEV_NAMES, Shrink, wild};

pub type NodeId = u32;
pub const NONE: NodeId = u32::MAX;

pub mod flags {
    pub const DIR: u16 = 1;
    /// Junction or symlink: its content belongs to the target, so it is not descended.
    pub const LINK: u16 = 2;
    /// Compressed by the Windows Overlay Filter (Shrink, `compact /exe`, CompactOS).
    pub const WOF: u16 = 4;
    /// NTFS compression (the blue arrows).
    pub const NTFS_COMPRESSED: u16 = 8;
    pub const SPARSE: u16 = 16;
    /// Cloud placeholder (OneDrive and friends).
    pub const CLOUD: u16 = 32;
    /// EFS encrypted.
    pub const ENCRYPTED: u16 = 64;
    /// A second name of a hard-linked file: shown with 0 allocated, the first name carries it.
    pub const HARDLINK: u16 = 128;
    /// Synthetic node (file system metadata the scan could not attribute).
    pub const SYNTHETIC: u16 = 256;
    pub const HIDDEN: u16 = 512;
    pub const SYSTEM_ATTR: u16 = 1024;
}

/// 48 bytes plus the name.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    /// Allocated bytes (the subtree's, for folders).
    pub alloc: u64,
    /// Logical bytes (the subtree's, for folders).
    pub logical: u64,
    pub parent: NodeId,
    pub first_child: NodeId,
    pub next_sibling: NodeId,
    name_off: u32,
    /// Files in the subtree (1 for a file).
    pub files: u32,
    /// Last modification, Unix seconds (the newest in the subtree, for folders).
    pub mtime: u32,
    name_len: u16,
    pub flags: u16,
    pub category: Category,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.flags & flags::DIR != 0
    }
    pub fn has(&self, f: u16) -> bool {
        self.flags & f != 0
    }
}

/// One entry as a scanner reports it. `id` is dense (MFT record number or a counter).
#[derive(Clone, Debug)]
pub struct RawEntry {
    pub id: u64,
    pub parent: u64,
    pub name: String,
    pub alloc: u64,
    pub logical: u64,
    pub mtime: u32,
    pub flags: u16,
}

/// No id: for extra hard-link names, which nothing points at.
pub const NO_ID: u64 = u64::MAX;

pub struct TreeBuilder {
    nodes: Vec<Node>,
    names: String,
    parent_ids: Vec<u64>,
    idmap: Vec<u32>,
    root_id: u64,
}

impl TreeBuilder {
    /// `root_name` is `C:` for a drive.
    pub fn new(root_name: &str, root_id: u64) -> Self {
        let mut b = TreeBuilder {
            nodes: Vec::new(),
            names: String::new(),
            parent_ids: Vec::new(),
            idmap: Vec::new(),
            root_id,
        };
        b.add(&RawEntry {
            id: root_id,
            parent: root_id,
            name: root_name.to_string(),
            alloc: 0,
            logical: 0,
            mtime: 0,
            flags: flags::DIR,
        });
        b
    }

    pub fn reserve(&mut self, n: usize) {
        self.nodes.reserve(n);
        self.parent_ids.reserve(n);
        self.names.reserve(n * 16);
    }

    fn add(&mut self, e: &RawEntry) -> NodeId {
        let idx = self.nodes.len() as NodeId;
        let name_off = self.names.len() as u32;
        // NTFS names are at most 255 UTF-16 units, so this never cuts one.
        let name = &e.name[..e.name.len().min(u16::MAX as usize)];
        self.names.push_str(name);
        self.nodes.push(Node {
            alloc: e.alloc,
            logical: e.logical,
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            name_off,
            files: if e.flags & flags::DIR != 0 { 0 } else { 1 },
            mtime: e.mtime,
            name_len: name.len() as u16,
            flags: e.flags,
            category: Category::Other,
        });
        self.parent_ids.push(e.parent);
        if e.id != NO_ID {
            let i = e.id as usize;
            if i >= self.idmap.len() {
                self.idmap
                    .resize((i + 1).max(self.idmap.len() * 2).max(1024), NONE);
            }
            self.idmap[i] = idx;
        }
        idx
    }

    pub fn push(&mut self, e: RawEntry) {
        if e.id == self.root_id {
            // The scanner's own record of the root folder: keep its size, not a second node.
            let r = &mut self.nodes[0];
            r.alloc += e.alloc;
            r.mtime = e.mtime;
            return;
        }
        self.add(&e);
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Links children to parents, sums sizes up the tree and sorts children by size.
    pub fn finish(self) -> Tree {
        let TreeBuilder {
            mut nodes,
            names,
            parent_ids,
            idmap,
            ..
        } = self;
        for i in (1..nodes.len()).rev() {
            let pid = parent_ids[i];
            let mut p = idmap.get(pid as usize).copied().unwrap_or(NONE);
            if p == NONE || p as usize == i || !nodes[p as usize].is_dir() {
                p = 0; // orphan: attach to the root so its bytes still count
            }
            nodes[i].parent = p;
            nodes[i].next_sibling = nodes[p as usize].first_child;
            nodes[p as usize].first_child = i as NodeId;
        }
        let mut tree = Tree {
            nodes,
            names,
            root: 0,
        };
        tree.aggregate();
        tree
    }
}

pub struct Tree {
    pub nodes: Vec<Node>,
    names: String,
    pub root: NodeId,
}

/// What classification found besides categories.
#[derive(Clone, Debug, Default)]
pub struct Classification {
    /// (clean rule index, folder) for every folder a clean rule matched.
    pub rule_hits: Vec<(u16, NodeId)>,
    /// Folders Shrink offers (one per app, game, toolchain or build folder).
    pub shrink_candidates: Vec<NodeId>,
}

impl Tree {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn name(&self, id: NodeId) -> &str {
        let n = &self.nodes[id as usize];
        &self.names[n.name_off as usize..n.name_off as usize + n.name_len as usize]
    }

    /// Lowercase extension of a file, or "".
    pub fn ext(&self, id: NodeId) -> String {
        let name = self.name(id);
        match name.rfind('.') {
            Some(i) if i > 0 && name.len() - i <= 12 => name[i + 1..].to_ascii_lowercase(),
            _ => String::new(),
        }
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.nodes[id as usize].first_child,
        }
    }

    /// Full path: `C:\Users\me\file.txt` (`C:\` for the root).
    pub fn path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NONE {
            parts.push(self.name(cur));
            cur = self.nodes[cur as usize].parent;
        }
        parts.reverse();
        if parts.len() == 1 {
            return format!("{}\\", parts[0]);
        }
        parts.join("\\")
    }

    pub fn depth(&self, id: NodeId) -> usize {
        let mut d = 0;
        let mut cur = self.nodes[id as usize].parent;
        while cur != NONE {
            d += 1;
            cur = self.nodes[cur as usize].parent;
        }
        d
    }

    /// The node at `path` (case-insensitive), if the scan saw it.
    pub fn find(&self, path: &str) -> Option<NodeId> {
        let mut segs = path.trim_end_matches('\\').split('\\');
        let first = segs.next()?;
        if !first.eq_ignore_ascii_case(self.name(self.root)) {
            return None;
        }
        let mut cur = self.root;
        for seg in segs.filter(|s| !s.is_empty()) {
            cur = self.children(cur).find(|&c| {
                self.name(c).eq_ignore_ascii_case(seg)
                    || self.name(c).to_lowercase() == seg.to_lowercase()
            })?;
        }
        Some(cur)
    }

    pub fn total(&self) -> u64 {
        self.nodes[self.root as usize].alloc
    }

    /// Adds a synthetic leaf (file system metadata) under `parent` and re-aggregates its ancestors.
    pub fn add_synthetic(
        &mut self,
        parent: NodeId,
        name: &str,
        alloc: u64,
        category: Category,
    ) -> NodeId {
        let name_off = self.names.len() as u32;
        self.names.push_str(name);
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            alloc,
            logical: alloc,
            parent,
            first_child: NONE,
            next_sibling: self.nodes[parent as usize].first_child,
            name_off,
            files: 0,
            mtime: 0,
            name_len: name.len() as u16,
            flags: flags::SYNTHETIC,
            category,
        });
        self.nodes[parent as usize].first_child = id;
        let mut cur = parent;
        while cur != NONE {
            self.nodes[cur as usize].alloc += alloc;
            self.nodes[cur as usize].logical += alloc;
            cur = self.nodes[cur as usize].parent;
        }
        self.sort_children(parent);
        id
    }

    fn aggregate(&mut self) {
        // Iterative post-order from the root; unreachable nodes (a cycle in a damaged MFT) stay out.
        let mut stack = vec![(self.root, false)];
        while let Some((n, done)) = stack.pop() {
            if !done {
                stack.push((n, true));
                let mut c = self.nodes[n as usize].first_child;
                while c != NONE {
                    if self.nodes[c as usize].is_dir() {
                        stack.push((c, false));
                    }
                    c = self.nodes[c as usize].next_sibling;
                }
            } else {
                let (mut alloc, mut logical, mut files, mut mtime) = (0u64, 0u64, 0u32, 0u32);
                let mut c = self.nodes[n as usize].first_child;
                while c != NONE {
                    let k = &self.nodes[c as usize];
                    alloc += k.alloc;
                    logical += k.logical;
                    files = files.saturating_add(k.files);
                    mtime = mtime.max(k.mtime);
                    c = k.next_sibling;
                }
                let node = &mut self.nodes[n as usize];
                node.alloc += alloc;
                node.logical += logical;
                node.files = node.files.saturating_add(files);
                node.mtime = node.mtime.max(mtime);
                self.sort_children(n);
            }
        }
    }

    fn sort_children(&mut self, n: NodeId) {
        let mut kids: Vec<NodeId> = self.children(n).collect();
        if kids.len() < 2 {
            return;
        }
        kids.sort_by(|&a, &b| {
            self.nodes[b as usize]
                .alloc
                .cmp(&self.nodes[a as usize].alloc)
        });
        self.nodes[n as usize].first_child = kids[0];
        for w in kids.windows(2) {
            self.nodes[w[0] as usize].next_sibling = w[1];
        }
        self.nodes[*kids.last().unwrap() as usize].next_sibling = NONE;
    }

    /// Assigns every node a category and finds clean-rule folders and shrink candidates.
    pub fn classify(&mut self, c: &Classifier) -> Classification {
        struct Frame {
            node: NodeId,
            inherited: Option<Category>,
            states: Vec<(u32, u16)>,
            /// The parent matched a `Shrink::Children` pattern.
            children_are_candidates: bool,
            in_candidate: bool,
        }
        let mut out = Classification::default();
        let root = self.root;
        self.nodes[root as usize].category = Category::Other;
        let mut stack = vec![Frame {
            node: root,
            inherited: None,
            states: c.root_states(),
            children_are_candidates: false,
            in_candidate: false,
        }];
        while let Some(f) = stack.pop() {
            let kids: Vec<NodeId> = self.children(f.node).collect();
            for k in kids {
                let is_dir = self.nodes[k as usize].is_dir();
                let mut matched: Option<u32> = None;
                let mut next_states = Vec::new();
                if !f.states.is_empty() {
                    let lname = self.name(k).to_lowercase();
                    for &(p, d) in &f.states {
                        let pat = &c.patterns[p as usize];
                        if !wild(&pat.segs[d as usize], &lname) {
                            continue;
                        }
                        if d as usize + 1 == pat.segs.len() {
                            if matched.is_none_or(|m| p < m) {
                                matched = Some(p);
                            }
                        } else if is_dir {
                            next_states.push((p, d + 1));
                        }
                    }
                }
                let pat = matched.map(|p| &c.patterns[p as usize]);
                let mut cat = pat.and_then(|p| p.category).or(f.inherited);
                let mut candidate = false;
                if let Some(p) = pat {
                    if let Some(r) = p.rule {
                        out.rule_hits.push((r, k));
                    }
                    candidate |= p.shrink == Shrink::Itself;
                }
                // A folder of its own pattern (WindowsApps inside Program Files) decides for itself.
                if is_dir && f.children_are_candidates && pat.is_none() {
                    candidate = true;
                }
                if cat.is_none() {
                    if is_dir {
                        if self.is_dev_folder(k) {
                            cat = Some(Category::Developer);
                            candidate = true;
                        }
                    } else {
                        cat = Category::from_ext(&self.ext(k));
                    }
                }
                let category = cat.unwrap_or(Category::Other);
                self.nodes[k as usize].category = category;
                let candidate = candidate
                    && is_dir
                    && !f.in_candidate
                    && !self.nodes[k as usize].has(flags::LINK);
                if candidate {
                    out.shrink_candidates.push(k);
                }
                if is_dir && self.nodes[k as usize].first_child != NONE {
                    stack.push(Frame {
                        node: k,
                        // Folders get a category only from a path rule, a parent or a build-folder name.
                        inherited: cat,
                        states: next_states,
                        children_are_candidates: pat.is_some_and(|p| p.shrink == Shrink::Children),
                        in_candidate: f.in_candidate || candidate,
                    });
                }
            }
        }
        out
    }

    fn is_dev_folder(&self, k: NodeId) -> bool {
        let name = self.name(k);
        let Some(dev) = DEV_NAMES.iter().find(|d| name.eq_ignore_ascii_case(d)) else {
            return false;
        };
        // A `target` folder counts only when Cargo marked it with CACHEDIR.TAG.
        *dev != "target"
            || self
                .children(k)
                .any(|c| self.name(c).eq_ignore_ascii_case("CACHEDIR.TAG"))
    }

    /// Allocated bytes per category (each folder's own index allocation included).
    pub fn category_totals(&self) -> [u64; 11] {
        let mut own: Vec<u64> = self.nodes.iter().map(|n| n.alloc).collect();
        for n in &self.nodes {
            if n.parent != NONE {
                own[n.parent as usize] = own[n.parent as usize].saturating_sub(n.alloc);
            }
        }
        let mut totals = [0u64; 11];
        for (n, o) in self.nodes.iter().zip(own) {
            totals[n.category.index()] += o;
        }
        totals
    }

    /// Every node in the subtree of `id` (itself included), depth first.
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            tree: self,
            stack: vec![id],
        }
    }

    /// Bytes per category inside one folder.
    pub fn subtree_totals(&self, id: NodeId) -> [u64; 11] {
        let mut totals = [0u64; 11];
        for d in self.descendants(id) {
            let n = self.node(d);
            if !n.is_dir() {
                totals[n.category.index()] += n.alloc;
            }
        }
        totals
    }

    /// Memory the tree holds, in bytes.
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.names.capacity()
    }
}

pub struct Children<'a> {
    tree: &'a Tree,
    next: NodeId,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        if self.next == NONE {
            return None;
        }
        let cur = self.next;
        self.next = self.tree.nodes[cur as usize].next_sibling;
        Some(cur)
    }
}

pub struct Descendants<'a> {
    tree: &'a Tree,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let cur = self.stack.pop()?;
        self.stack.extend(self.tree.children(cur));
        Some(cur)
    }
}
