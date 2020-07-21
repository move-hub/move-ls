#[macro_use]
extern crate log;

pub mod error_diagnostic;
pub mod lsp_server;
mod move_document;
pub mod tree_sitter_move;
pub mod utils;

pub mod config;
pub mod node_resolver;
mod salsa;
mod tests;
