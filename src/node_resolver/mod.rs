use crate::tree_sitter_move::language;
use tree_sitter::{Query, QueryCursor, Range};

pub const USE_QUERY: &str = include_str!("../queries/use.scm");
pub mod module_resolver;

pub struct NodeResolver;

pub enum Resolved {
    Module {
        name: Range,
        address: Option<Range>,
    },
    StructIdentifier {
        name: Range,
        module: Option<Range>,
    },
    FunctionIdentifier {
        name: Range,
        module: Option<Range>,
        address: Option<Range>,
    },
}

impl NodeResolver {
    pub fn resolve(n: &tree_sitter::Node, root: &tree_sitter::Node) -> Option<Resolved> {
        match n.kind() {
            "module_identifier" => {
                let range = n.range();

                let address = if n.parent().map(|p| p.kind() == "module_access").is_some() {
                    let prev_sibling = n.prev_named_sibling();
                    let address_range = prev_sibling
                        .filter(|s| s.kind() == "address_literal")
                        .map(|s| s.range());
                    address_range
                } else {
                    None
                };
                Some(Resolved::Module {
                    name: range,
                    address,
                })
            }

            _ => None,
        }
    }

    pub fn resolve_use(node: &tree_sitter::Node) -> Vec<UseInfo> {
        let use_query = Query::new(language(), USE_QUERY).unwrap();
        let mut cursor = QueryCursor::new();
        let matched = cursor.matches(&use_query, node.clone(), |n| "");

        let mut uses = vec![];
        for mat in matched {
            let mut use_info = [None; 5];
            for cap in mat.captures {
                let idx = cap.index;
                let node = cap.node;
                use_info[idx as usize] = Some(node.range());
            }

            let [addr, module, module_alias, member, member_alias] = use_info;
            uses.push(UseInfo {
                addr: addr.unwrap(),
                module: module.unwrap(),
                module_alias,
                member,
                member_alias,
            });
        }
        uses
    }
}

pub struct UseInfo {
    addr: Range,
    module: Range,
    module_alias: Option<Range>,
    member: Option<Range>,
    member_alias: Option<Range>,
}
