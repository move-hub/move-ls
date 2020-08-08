#![allow(unused)]

use super::tree_sitter_move::Parser;
use crate::{node_resolver::NodeResolver, tree_sitter_move::parser};
use anyhow::{bail, ensure, Result};
use parking_lot::RwLock;
use serde::export::Formatter;
use std::cell::Cell;
use tower_lsp::lsp_types;
use tree_sitter::{InputEdit, Point, Query, Tree};
use xi_rope::{rope::BaseMetric, Cursor, DeltaBuilder, Interval, Rope, RopeDelta};

pub struct MoveDocument {
    parser: Option<Parser>,
    rope: Rope,
    tree: Tree,
    version: u64,
}

unsafe impl Send for MoveDocument {}
unsafe impl Sync for MoveDocument {}

impl MoveDocument {
    pub fn new<S: AsRef<str>>(version: u64, s: S) -> Result<Self> {
        let rope = Rope::from(s.as_ref());
        let mut parser = parser();

        let tree = match parser.parse_with(&mut |offset, _pos| get_chunk(&rope, offset), None) {
            None => {
                parser.reset();
                bail!("Fail to parse input into move syntax tree");
            }
            Some(t) => t,
        };
        let parser = Some(parser);

        Ok(Self {
            parser,
            tree,
            rope,
            version,
        })
    }

    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn check_version(&self, new_version: u64) -> anyhow::Result<()> {
        ensure!(
            self.version < new_version,
            "version outdated, current: {}, candidate: {}",
            self.version,
            new_version
        );
        Ok(())
    }

    pub fn incr_version(&mut self, new_version: u64) {
        self.version = new_version;
    }

    pub fn resolve(&self, pos: lsp_types::Position) {
        let offset = position_to_offset(&self.rope, pos);
        let leaf = self
            .tree
            .root_node()
            .descendant_for_byte_range(offset, offset);
        if let Some(n) = leaf {
            NodeResolver::resolve(&n, &self.tree.root_node());
        }
    }

    /// The content changes describe single state changes to the document.
    /// So if there are two content changes c1 (at array index 0) and
    /// c2 (at array index 1) for a document in state S then c1 moves the document from
    /// S to S' and c2 from S' to S''. So c1 is computed on the state S and c2 is computed
    /// on the state S'.
    pub fn edit_many<S: AsRef<str>>(&mut self, edits: impl Iterator<Item = (lsp_types::Range, S)>) {
        for (range, text) in edits {
            // TODO: better handle this.
            let _ = self.edit(range, text);
        }
    }

    /// FIXME: As lsp use utf16 for it text position.(see https://github.com/microsoft/language-server-protocol/issues/376)
    /// We need to adjust range to utf8, as rope store text using rust String which is based on utf8.
    /// Once it's solved, we can use incremental doc sync.
    pub fn edit<S: AsRef<str>>(&mut self, range: lsp_types::Range, text: S) -> Result<()> {
        let iv = Interval {
            start: position_to_offset(&self.rope, range.start),
            end: position_to_offset(&self.rope, range.end),
        };
        self.rope.edit(iv, text.as_ref());

        let new_end_offset = iv.start + text.as_ref().as_bytes().len();
        let new_end_position = {
            let line = self.rope.line_of_offset(new_end_offset);
            let line_offset = self.rope.offset_of_line(line);
            let column = new_end_offset - line_offset;
            Point { row: line, column }
        };
        {
            let mut t = &mut self.tree;
            let edit = InputEdit {
                start_byte: iv.start,
                old_end_byte: iv.end,
                new_end_byte: new_end_offset,
                start_position: position_to_point(range.start),
                old_end_position: position_to_point(range.end),
                new_end_position,
            };
            t.edit(&edit);
        }

        self.reparse()
    }

    pub fn reset_with(&mut self, text: impl AsRef<str>) -> Result<()> {
        if let Some(p) = self.parser.as_mut() {
            p.reset()
        }
        self.rope = Rope::from(text.as_ref());
        self.reparse()
    }

    fn reparse(&mut self) -> Result<()> {
        let mut parser = self.parser.take().unwrap();
        let tree = parser.parse_with(
            &mut |offset, _pos| get_chunk(&self.rope, offset),
            Some(&self.tree),
        );
        // TODO: make it panic safe.
        self.parser = Some(parser);

        match tree {
            None => bail!("Fail to parse input into move syntax tree"),
            Some(t) => {
                self.tree = t;
                Ok(())
            }
        }
    }
}

impl std::fmt::Display for MoveDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.rope)
    }
}

/// Transform lsp Position to offset.
/// Notice: character of Position is char-indexed, not byte-indexed.
pub fn position_to_offset(rope: &Rope, pos: lsp_types::Position) -> usize {
    let lsp_types::Position { line, character } = pos;
    let offset_of_line_start = rope.offset_of_line(line as usize);

    if character == 0 {
        offset_of_line_start
    } else {
        let mut cursor = Cursor::new(rope, offset_of_line_start);
        let pos = cursor.iter::<BaseMetric>().nth(character as usize - 1);

        pos.unwrap()
    }
}

pub fn get_chunk(rope: &Rope, offset: usize) -> &str {
    let c = Cursor::new(&rope, offset);
    if let Some((node, idx)) = c.get_leaf() {
        &node[idx..]
    } else {
        ""
    }
}

#[inline]
fn position_to_point(p: lsp_types::Position) -> Point {
    Point {
        row: p.line as usize,
        column: p.character as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn test_position_to_offset() {
        let text = "s≤s≤\ns≤s";
        let rope = Rope::from(text);
        let mut c = Cursor::new(&rope, rope.offset_of_line(0));
        let mut iter = c.iter::<BaseMetric>();

        let mut char_index = text.char_indices();
        let _ = char_index.next();
        for (idx, char) in char_index {
            let pos = iter.next();
            assert_eq!(pos, Some(idx));
        }
        let last = iter.next();
        assert!(last.is_some());

        let text = "ss\nss";
        let rope = Rope::from(text);
        for (pos, expected_offset) in vec![
            (
                lsp_types::Position {
                    line: 1,
                    character: 0,
                },
                3,
            ),
            (
                lsp_types::Position {
                    line: 1,
                    character: 1,
                },
                4,
            ),
        ] {
            let offset = position_to_offset(&rope, pos);
            assert_eq!(offset, expected_offset);
        }
    }

    #[test]
    fn test_edit() {
        let mut doc = MoveDocument::new(1, "").unwrap();
        let range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let new_text = "address 0x1 {}".to_string();
        doc.edit(range, new_text.clone());

        assert_eq!(format!("{}", &doc), new_text);
    }

    #[test]
    fn test_edit_with_utf8() {
        let mut doc = MoveDocument::new(1, "module Abc {}").unwrap();
        let range = Range::new(Position::new(0, 8), Position::new(0, 9));
        let new_text = "≤".to_string();
        doc.edit(range, new_text);

        assert_eq!(format!("{}", &doc), "module A≤c {}");
        let tree_node = doc.tree.root_node();
        println!("tree: {:?}", tree_node);
    }

    #[test]
    fn test_edit_many() {
        let mut doc = MoveDocument::new(1, "address 0x1 {}").unwrap();
        let delete_range = Range::new(Position::new(0, 0), Position::new(0, 14));

        let add_range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let new_text = "module A {}".to_string();
        let edits = vec![(delete_range, ""), (add_range, &new_text)].into_iter();
        doc.edit_many(edits);

        assert_eq!(format!("{}", &doc), new_text);
        let tree_range = doc.tree.root_node().byte_range();

        assert_eq!(tree_range.start, 0);
        assert_eq!(tree_range.end, new_text.len());

        let module_def = doc.tree.root_node().child(0).unwrap();
        assert_eq!(module_def.named_child_count(), 2);
        assert!(!module_def.has_error());
    }
}
