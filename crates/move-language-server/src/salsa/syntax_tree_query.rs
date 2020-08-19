use crate::salsa::FileId;
use xi_rope::Rope;

#[salsa::query_group(SyntaxTreeQueryStorage)]
pub trait SyntaxTreeQuery: salsa::Database {
    #[salsa::input]
    fn rope(&self, file_id: FileId) -> Rope;
    // #[salsa::input]
    // fn tree(&self, file_id: FileId) -> Tree;

    // fn modules(&self, file_id: FileId) -> Vec<Node>;

    // fn module_use(&self, module_node: Node);
}

// fn modules(db: &dyn SyntaxTreeQuery, file_id: FileId) -> Vec<Node> {
//     let tree = db.tree(file_id);
//     let node: tree_sitter::Node = tree.root_node();
//     let mut cursor = node.walk();
//     for child in node.named_children(&mut cursor) {
//         let node: Node = child;
//         let nodes = match node.kind() {
//             "address_block" => {
//                 let mut cursor = node.walk();
//                 let address = node
//                     .child_by_field_name("address")
//                     .map(|node| get_address(&node))
//                     .map(|range| get_text(&db.rope(file_id), range));
//
//                 node.named_children(&mut cursor).skip(1).collect()
//             }
//             "module_definition" => vec![node],
//             _ => vec![],
//         };
//
//         return nodes;
//     }
//
//     return vec![];
// }
//
// fn module_names(db: &dyn SyntaxTreeQuery, file_id: FileId) -> Vec<(Option<String>, String)> {
//     let tree = db.tree(file_id);
//     let node: tree_sitter::Node = tree.root_node();
//
//     let mut module_names = vec![];
//     let mut cursor = node.walk();
//     for child in node.named_children(&mut cursor) {
//         let node: Node = child;
//         match node.kind() {
//             "address_block" => {
//                 let mut cursor = node.walk();
//                 let address = node
//                     .child_by_field_name("address")
//                     .map(|node| get_address(&node))
//                     .map(|range| get_text(&db.rope(file_id), range));
//
//                 for child in node.named_children(&mut cursor).skip(1) {
//                     let module_name =
//                         get_module_name(&child).map(|r| get_text(&db.rope(file_id), r));
//                     if let Some(name) = module_name {
//                         module_names.push((address.clone(), name));
//                     }
//                 }
//             }
//             "module_definition" => {
//                 let module_name = get_module_name(&node).map(|r| get_text(&db.rope(file_id), r));
//                 if let Some(name) = module_name {
//                     module_names.push((None, name));
//                 }
//             }
//             _ => {}
//         }
//     }
//     module_names
// }
//
// fn get_utf8_text(source: &[u8], range: std::ops::Range<usize>) -> String {
//     let bytes = &source.as_bytes()[range];
//     String::from_utf8_lossy(bytes).to_string()
// }
//
// fn get_text(rope: &Rope, range: Range<usize>) -> String {
//     rope.slice_to_cow(range).to_string()
// }
//
// fn get_address(address_literal: &Node) -> std::ops::Range<usize> {
//     debug_assert!(address_literal.kind() == "address_literal");
//     address_literal.byte_range()
// }
//
// fn get_module_name(node: &Node) -> Option<std::ops::Range<usize>> {
//     debug_assert!(node.kind() == "module_definition");
//     let module_identifier = node.child_by_field_name("name");
//     let module_name = module_identifier.map(|node| node.byte_range());
//     module_name
// }
