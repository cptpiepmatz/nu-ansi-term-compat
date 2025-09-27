use std::path::PathBuf;

use gix::{
    bstr::{BStr, ByteSlice},
    objs::tree::EntryRef,
    traverse::tree::{Visit, visit::Action},
};

#[derive(Debug, Default)]
struct Visitor {
    crate_count: usize,
}

impl Visit for Visitor {
    fn pop_back_tracked_path_and_set_current(&mut self) {}
    fn pop_front_tracked_path_and_set_current(&mut self) {}
    fn push_back_tracked_path_component(&mut self, _: &BStr) {}
    fn push_path_component(&mut self, _: &BStr) {}
    fn pop_path_component(&mut self) {}

    fn visit_tree(&mut self, _: &EntryRef<'_>) -> Action {
        Action::Continue
    }

    fn visit_nontree(&mut self, entry: &EntryRef<'_>) -> Action {
        let name = entry.filename.as_bstr();
        if !name.contains_str(".") {
            self.crate_count += 1;
        }
        Action::Continue
    }
}

pub fn count_crates(path: impl Into<PathBuf>) -> anyhow::Result<usize> {
    let repo = gix::open(path)?;
    let mut head = repo.head()?;
    let commit = head.peel_to_commit_in_place()?;
    let tree = commit.tree()?;

    let mut visitor = Visitor::default();
    tree.traverse().depthfirst(&mut visitor)?;
    Ok(visitor.crate_count)
}
