use crate::{
    node_resolver::NodeResolver,
    tree_sitter_move::{language, parser},
};
use std::fs::File;

#[test]
pub fn test_resolve_use() {
    let text = include_str!("./cases/use_query.move");
    let tree = parser().parse(text, None).unwrap();
    let uses = NodeResolver::resolve_use(&tree.root_node());
    assert_eq!(uses.len(), 10);
}
