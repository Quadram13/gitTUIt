use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone)]
pub struct FileLeaf {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeRowKind {
    Directory,
    File,
}

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub kind: TreeRowKind,
    pub status: Option<String>,
    pub expanded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ChangedFilesTree {
    files: Vec<FileLeaf>,
    expanded_dirs: HashSet<String>,
    rows: Vec<TreeRow>,
}

impl ChangedFilesTree {
    pub fn clear(&mut self) {
        self.files.clear();
        self.expanded_dirs.clear();
        self.rows.clear();
    }

    pub fn set_files(&mut self, files: Vec<FileLeaf>) {
        self.files = files
            .into_iter()
            .map(|mut leaf| {
                leaf.path = normalize_tree_path(&leaf.path);
                leaf
            })
            .filter(|leaf| !leaf.path.is_empty())
            .collect();
        self.rebuild_rows();
    }

    pub fn expand_all_dirs(&mut self) {
        self.expanded_dirs = collect_all_dirs(&self.files);
        self.rebuild_rows();
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, idx: usize) -> Option<&TreeRow> {
        self.rows.get(idx)
    }

    pub fn row_path(&self, idx: usize) -> Option<&str> {
        self.rows.get(idx).map(|row| row.path.as_str())
    }

    pub fn find_row_index_by_path(&self, path: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }

    pub fn selected_file_path(&self, idx: usize) -> Option<String> {
        self.rows
            .get(idx)
            .and_then(|row| (row.kind == TreeRowKind::File).then_some(row.path.clone()))
    }

    pub fn expand_selected_dir(&mut self, idx: usize) -> bool {
        let Some(row) = self.rows.get(idx) else {
            return false;
        };
        if row.kind != TreeRowKind::Directory || row.expanded {
            return false;
        }
        self.expanded_dirs.insert(row.path.clone());
        self.rebuild_rows();
        true
    }

    pub fn collapse_selected_dir(&mut self, idx: usize) -> bool {
        let Some(row) = self.rows.get(idx) else {
            return false;
        };
        if row.kind != TreeRowKind::Directory || !row.expanded {
            return false;
        }
        self.expanded_dirs.remove(row.path.as_str());
        self.rebuild_rows();
        true
    }

    fn rebuild_rows(&mut self) {
        self.rows = build_rows(&self.files, &self.expanded_dirs);
    }
}

fn normalize_tree_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn collect_all_dirs(files: &[FileLeaf]) -> HashSet<String> {
    let mut dirs = HashSet::new();
    for file in files {
        let mut parts = file.path.split('/').collect::<Vec<_>>();
        if parts.len() <= 1 {
            continue;
        }
        parts.pop();
        let mut current = String::new();
        for segment in parts {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            dirs.insert(current.clone());
        }
    }
    dirs
}

fn build_rows(files: &[FileLeaf], expanded_dirs: &HashSet<String>) -> Vec<TreeRow> {
    let mut all_dirs = BTreeSet::new();
    let mut file_entries = Vec::new();
    for file in files {
        let path = file.path.clone();
        file_entries.push((path.clone(), file.status.clone()));
        let mut parts = path.split('/').collect::<Vec<_>>();
        if parts.len() <= 1 {
            continue;
        }
        parts.pop();
        let mut current = String::new();
        for segment in parts {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            all_dirs.insert(current.clone());
        }
    }

    let mut dir_children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for dir in all_dirs {
        let parent = dir.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
        dir_children.entry(parent.to_string()).or_default().push(dir);
    }
    for children in dir_children.values_mut() {
        children.sort();
    }

    let mut files_by_parent: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (path, status) in file_entries {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        files_by_parent.entry(parent).or_default().push((path, status));
    }
    for children in files_by_parent.values_mut() {
        children.sort_by(|(left, _), (right, _)| left.cmp(right));
    }

    let mut rows = Vec::new();
    add_rows(
        "",
        0,
        expanded_dirs,
        &dir_children,
        &files_by_parent,
        &mut rows,
    );
    rows
}

fn add_rows(
    parent: &str,
    depth: usize,
    expanded_dirs: &HashSet<String>,
    dir_children: &BTreeMap<String, Vec<String>>,
    files_by_parent: &BTreeMap<String, Vec<(String, String)>>,
    rows: &mut Vec<TreeRow>,
) {
    if let Some(children) = dir_children.get(parent) {
        for child in children {
            let name = child
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(child.as_str())
                .to_string();
            let expanded = expanded_dirs.contains(child);
            rows.push(TreeRow {
                path: child.clone(),
                name,
                depth,
                kind: TreeRowKind::Directory,
                status: None,
                expanded,
            });
            if expanded {
                add_rows(
                    child,
                    depth + 1,
                    expanded_dirs,
                    dir_children,
                    files_by_parent,
                    rows,
                );
            }
        }
    }

    if let Some(files) = files_by_parent.get(parent) {
        for (path, status) in files {
            let name = path
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(path.as_str())
                .to_string();
            rows.push(TreeRow {
                path: path.clone(),
                name,
                depth,
                kind: TreeRowKind::File,
                status: Some(status.clone()),
                expanded: false,
            });
        }
    }
}
