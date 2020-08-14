pub use tree_sitter::Parser;

use tree_sitter::Language;

extern "C" {
    fn tree_sitter_move() -> Language;
}

pub fn language() -> Language {
    unsafe { tree_sitter_move() }
}

pub fn parser() -> Parser {
    let language = unsafe { tree_sitter_move() };
    let mut parser = Parser::new();
    parser.set_language(language).unwrap();
    parser
}

// // TODO: make it const.
// pub static NODE_TYPES: Lazy<Vec<DataType>> = Lazy::new(|| {
//     let node_types = include_str!("../../../tree-sitter-move/src/node-types.json");
//     serde_json::from_str(node_types).unwrap()
// });
