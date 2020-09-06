#![allow(unused)]

use super::tree_sitter_move::Parser;
use crate::{node_resolver::NodeResolver, tree_sitter_move::parser};
use anyhow::{bail, ensure, Result};
use parking_lot::RwLock;
use serde::export::Formatter;
use std::cell::Cell;
use tower_lsp::lsp_types;
use tree_sitter::{InputEdit, Node, Point, Query, Tree};
use xi_rope::{
    rope::{BaseMetric, Utf16CodeUnitsMetric},
    Cursor, DeltaBuilder, Interval, LinesMetric, Rope, RopeDelta,
};

#[derive(Clone, Debug)]
pub struct RopeDoc {
    rope: Rope,
    version: u64,
}

impl RopeDoc {
    pub fn new<S: AsRef<str>>(version: u64, s: S) -> Self {
        let rope = Rope::from(s.as_ref());

        Self { rope, version }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
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

    pub fn to_offset(&self, pos: lsp_types::Position) -> Option<usize> {
        position_to_offset(&self.rope, pos)
    }

    pub fn to_position(&self, offset: usize) -> Option<lsp_types::Position> {
        offset_to_position(&self.rope, offset)
    }

    /// Edit the do given the text range to edit, and the edited text.
    /// Return new end offset.
    pub fn edit<S: AsRef<str>>(&mut self, iv: Interval, text: S) -> usize {
        self.rope.edit(iv, text.as_ref());

        let new_end_offset = iv.start + text.as_ref().as_bytes().len();
        new_end_offset
    }
}

pub struct MoveDocument {
    doc: RopeDoc,

    parser: Parser,
    tree: Option<Tree>,
}

unsafe impl Send for MoveDocument {}
unsafe impl Sync for MoveDocument {}

impl MoveDocument {
    pub fn new<S: AsRef<str>>(version: u64, s: S) -> Self {
        let rope = Rope::from(s.as_ref());
        let rope_doc = RopeDoc::new(version, s);

        let mut parser = parser();
        let tree = parser.parse_with(&mut |offset, _pos| get_chunk(rope_doc.rope(), offset), None);
        if tree.is_none() {
            parser.reset();
            warn!("Fail to parse input into move syntax tree");
        };

        Self {
            doc: rope_doc,
            parser,
            tree,
        }
    }

    pub fn doc(&self) -> &RopeDoc {
        &self.doc
    }

    pub fn resolve_to_leaf_node(&self, pos: lsp_types::Position) -> Option<Node> {
        let offset = self.doc.to_offset(pos)?;
        self.tree
            .as_ref()?
            .root_node()
            .descendant_for_byte_range(offset, offset)
    }

    /// The content changes describe single state changes to the document.
    /// So if there are two content changes c1 (at array index 0) and
    /// c2 (at array index 1) for a document in state S then c1 moves the document from
    /// S to S' and c2 from S' to S''. So c1 is computed on the state S and c2 is computed
    /// on the state S'.
    pub fn edit_many<S: AsRef<str>>(
        &mut self,
        version: u64,
        edits: impl Iterator<Item = (lsp_types::Range, S)>,
    ) {
        for (range, text) in edits {
            // TODO: better handle this.
            let _ = self.edit(range, text);
        }
        self.doc.incr_version(version);
    }

    /// FIXME: As lsp use utf16 for it text position.(see https://github.com/microsoft/language-server-protocol/issues/376)
    /// We need to adjust range to utf8, as rope store text using rust String which is based on utf8.
    /// Once it's solved, we can use incremental doc sync.
    pub fn edit<S: AsRef<str>>(&mut self, range: lsp_types::Range, text: S) {
        let old_doc = self.doc.clone();

        // edit rope
        let iv = Interval {
            start: self.doc.to_offset(range.start).unwrap(),
            end: self.doc.to_offset(range.end).unwrap(),
        };
        let new_end_offset = self.doc.edit(iv, text);

        // edit tree if tree exists.
        if let Some(t) = &mut self.tree {
            let old_start_point = offset_to_point(&old_doc.rope, iv.start);
            let old_end_point = offset_to_point(&old_doc.rope, iv.end);
            let new_end_point = offset_to_point(&self.doc.rope, new_end_offset);

            let edit = InputEdit {
                start_byte: iv.start,
                old_end_byte: iv.end,
                new_end_byte: new_end_offset,
                start_position: old_end_point,
                old_end_position: old_end_point,
                new_end_position: new_end_point,
            };
            t.edit(&edit);
        }

        self.reparse_tree();
    }

    pub fn reset_with(&mut self, version: u64, text: impl AsRef<str>) {
        self.doc = RopeDoc::new(version, text);
        self.parser.reset();
        self.reparse_tree();
    }

    fn reparse_tree(&mut self) {
        let rope = self.doc.rope().clone();

        let old_tree = self.tree.clone();
        let tree = self.parser.parse_with(
            &mut |offset, _pos| get_chunk(&rope, offset),
            old_tree.as_ref(),
        );
        if tree.is_none() {
            warn!("Fail to parse input into move syntax tree");
            self.parser.reset();
        }

        self.tree = tree;
    }
}

impl std::fmt::Display for MoveDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.doc.rope())
    }
}

/// Transform lsp Position to offset.
/// Notice: character of Position is char-indexed, not byte-indexed.
pub fn position_to_offset(rope: &Rope, pos: lsp_types::Position) -> Option<usize> {
    let lsp_types::Position { line, character } = pos;
    let max_line = rope.measure::<LinesMetric>();
    if line as usize > max_line {
        return None;
    }
    let offset_of_line_start = rope.count_base_units::<LinesMetric>(line as usize);

    let sub_rope = rope.slice(offset_of_line_start..);

    let offset = sub_rope.count_base_units::<Utf16CodeUnitsMetric>(character as usize);
    Some(offset_of_line_start + offset)
}

pub fn offset_to_position(rope: &Rope, offset: usize) -> Option<lsp_types::Position> {
    let line = rope.line_of_offset(offset);
    let offset_of_line_start = rope.count_base_units::<LinesMetric>(line as usize);
    let sub_rope = rope.slice(offset_of_line_start..offset);
    let columns = sub_rope.count::<Utf16CodeUnitsMetric>(sub_rope.len());
    Some(lsp_types::Position {
        line: line as u64,
        character: columns as u64,
    })
}

pub fn offset_to_point(rope: &Rope, offset: usize) -> Point {
    let row = rope.line_of_offset(offset);
    let line_offset = rope.offset_of_line(row);
    let column = offset - line_offset;
    Point { row, column }
}

pub fn get_chunk(rope: &Rope, offset: usize) -> &str {
    let c = Cursor::new(&rope, offset);
    if let Some((node, idx)) = c.get_leaf() {
        &node[idx..]
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn test_lsp_position_to_offset() {
        let text = "a𐐀b\na𐐀b";

        let rope = Rope::from(text);
        {
            /// "a𐐀b"
            let utf16_pos = vec![
                (lsp_types::Position::new(0, 0), 'a'),
                (lsp_types::Position::new(0, 1), '𐐀'),
                (lsp_types::Position::new(0, 3), 'b'),
                (lsp_types::Position::new(0, 4), '\n'),
                (lsp_types::Position::new(1, 0), 'a'),
                (lsp_types::Position::new(1, 1), '𐐀'),
                (lsp_types::Position::new(1, 3), 'b'),
            ];
            for ((pos, expected_char), (expected_offset, char)) in
                utf16_pos.iter().zip(text.char_indices())
            {
                assert_eq!(position_to_offset(&rope, *pos), Some(expected_offset));
                assert_eq!(offset_to_position(&rope, expected_offset), Some(*pos));
                assert_eq!(&char, expected_char);
            }

            assert_eq!(
                position_to_offset(&rope, lsp_types::Position::new(1, 4)),
                Some(text.len())
            );

            assert_eq!(
                position_to_offset(&rope, lsp_types::Position::new(2, 0)),
                None
            );
            assert_eq!(
                position_to_offset(&rope, lsp_types::Position::new(2, 2)),
                None
            );
        }

        // test line metrics
        let text = "a\nb\n";
        let rope = Rope::from(text);
        {
            let mut cur = Cursor::new(&rope, 0);
            let mut c = cur.iter::<LinesMetric>();
            assert_eq!(c.pos(), 0);
            let next_line = c.next();
            assert_eq!(next_line, Some(2));
            let next_line = c.next();
            assert_eq!(next_line, Some(4));
            assert_eq!(c.next(), None);
        }
    }

    #[test]
    fn test_position_resolve() {
        let mut doc = MoveDocument::new(1, "module Abc {}");
        let pos = Position::new(0, 0);
        let node = doc.resolve_to_leaf_node(pos);
        assert!(node.is_some());
        let node = node.unwrap();
        assert!(!node.is_named());
        println!("kind: {}, range: {:?}", node.kind(), node.range());

        let pos = Position::new(0, 9);
        let node = doc.resolve_to_leaf_node(pos);
        assert!(node.is_some());
        let node = node.unwrap();
        assert!(node.is_named());
        assert_eq!("module_identifier", node.kind());
        println!("kind: {}, range: {:?}", node.kind(), node.range());
    }

    #[test]
    fn test_edit() {
        let mut doc = MoveDocument::new(1, "");
        let range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let new_text = "address 0x1 {}".to_string();
        doc.edit(range, new_text.clone());

        assert_eq!(format!("{}", &doc), new_text);
    }

    #[test]
    fn test_edit_with_utf8() {
        let mut doc = MoveDocument::new(1, "module Abc {}");
        let range = Range::new(Position::new(0, 8), Position::new(0, 9));
        let new_text = "≤".to_string();
        doc.edit(range, new_text);

        assert_eq!(format!("{}", &doc), "module A≤c {}");
        assert!(doc.tree.is_some());
    }

    #[test]
    fn test_edit_many() {
        let mut doc = MoveDocument::new(1, "address 0x1 {}");
        let delete_range = Range::new(Position::new(0, 0), Position::new(0, 14));

        let add_range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let new_text = "module A {}".to_string();
        let edits = vec![(delete_range, ""), (add_range, &new_text)].into_iter();
        doc.edit_many(2, edits);

        assert_eq!(format!("{}", &doc), new_text);
        assert!(doc.tree.is_some());
        let tree = doc.tree.as_ref().unwrap();
        let tree_range = tree.root_node().byte_range();

        assert_eq!(tree_range.start, 0);
        assert_eq!(tree_range.end, new_text.len());

        let module_def = tree.root_node().child(0).unwrap();
        assert_eq!(module_def.named_child_count(), 2);
        assert!(!module_def.has_error());
    }
}
