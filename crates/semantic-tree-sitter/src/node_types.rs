use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
