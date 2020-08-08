pub use tree_sitter::Parser;

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
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

// TODO: make it const.
pub static NODE_TYPES: Lazy<Vec<DataType>> = Lazy::new(|| {
    let node_types = include_str!("../tree-sitter-move/src/node-types.json");
    serde_json::from_str(node_types).unwrap()
});

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataType {
    SumType(SumType),
    ProductType(ProductType),
    LeafType(LeafType),
}

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct SumType {
    #[serde(rename = "type")]
    name: String,
    named: bool,
    subtypes: Vec<Ty>,
}
#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct ProductType {
    #[serde(rename = "type")]
    name: String,
    named: bool,
    fields: HashMap<String, Field>,
    children: Option<Children>,
}

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LeafType(pub Ty);

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Children(pub Field);

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct Field {
    required: bool,
    multiple: bool,
    /// types should not be empty
    types: Vec<Ty>,
}

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct Ty {
    #[serde(rename = "type")]
    name: String,
    named: bool,
}
