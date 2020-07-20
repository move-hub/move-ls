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
