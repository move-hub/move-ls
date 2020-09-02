use crate::{
    move_document::{get_chunk, position_to_offset},
    node_resolver::NodeResolver,
    tree_sitter_move::parser,
};
use move_lang::{
    compiled_unit::CompiledUnit,
    errors::{Errors, FilesSourceText},
    parser::ast,
    shared::Address,
    CommentMap,
};
use std::path::{Path, PathBuf};
use tower_lsp::{lsp_types, lsp_types::Location};
use xi_rope::Rope;

pub mod config_query;
pub mod move_ast_query;
pub mod syntax_tree_query;
pub mod text_source_query;

use config_query::*;
use move_ast_query::*;
use std::{borrow::Cow, collections::HashMap};
use syntax_tree_query::*;
use text_source_query::*;

pub type FileId = PathBuf;

#[salsa::database(ConfigStorage, SourceStorage, AstStorage, SyntaxTreeQueryStorage)]
#[derive(Default)]
pub struct RootDatabase {
    storage: salsa::Storage<Self>,
    sources: HashMap<FileId, Rope>,
}
impl salsa::Database for RootDatabase {}

impl SourceReader for RootDatabase {
    fn read(&self, file_id: FileId) -> Cow<str> {
        match self.sources.get(&file_id) {
            Some(rope) => rope.slice_to_cow(..),
            None => {
                // read from fs
                let content = std::fs::read_to_string(file_id).ok().unwrap_or_default();
                Cow::from(content)
            }
        }
    }

    fn did_change(&mut self, file_id: &Path) {
        SourceTextQuery
            .in_db_mut(self)
            .invalidate(&file_id.to_path_buf())
    }
}

impl RootDatabase {
    pub fn update_source(&mut self, fileid: FileId, rope: Rope) {
        self.sources.insert(fileid.clone(), rope);
        self.did_change(fileid.as_path());
    }

    pub fn close_source(&mut self, fielid: FileId) {
        self.sources.remove(&fielid);
    }

    pub fn compile_file(
        &self,
        sender: Option<Address>,
        file_path: PathBuf,
    ) -> (FilesSourceText, Result<Vec<CompiledUnit>, Errors>) {
        let (sources, cfg_program) = self.check_file(sender, file_path);
        let compiled_result = cfg_program.and_then(move_lang::to_bytecode::translate::program);
        (sources, compiled_result)
    }

    // TODO: refactor this and check_file.
    pub fn check_all(
        &self,
        sender: Option<Address>,
    ) -> (
        FilesSourceText,
        Result<move_lang::cfgir::ast::Program, Errors>,
    ) {
        let (sources, parsed_program) = self.parse_file(None);
        let sender = sender.or_else(|| self.sender());
        let checked = move_lang::check_program(parsed_program.map(|(p, _c)| p), sender);
        (sources, checked)
    }

    pub fn check_file(
        &self,
        sender: Option<Address>,
        file_path: PathBuf,
    ) -> (
        FilesSourceText,
        Result<move_lang::cfgir::ast::Program, Errors>,
    ) {
        let (sources, parsed_program) = self.parse_file(Some(file_path));
        let sender = sender.or_else(|| self.sender());
        let checked = move_lang::check_program(parsed_program.map(|(p, _c)| p), sender);
        (sources, checked)
    }

    fn parse_file(
        &self,
        file_path: Option<PathBuf>,
    ) -> (FilesSourceText, Result<(ast::Program, CommentMap), Errors>) {
        let mut errors = Errors::new();

        let deps: Vec<PathBuf> = self.stdlib_files();
        let mut lib_definitions = Vec::new();
        let mut source_texts = FilesSourceText::default();

        for dep in deps {
            let fname = self.leak_str(dep.clone());
            let source_text = self.source_text(dep.clone());
            source_texts.insert(fname, source_text.clone());
            match self.ast(dep.clone()) {
                Err(mut e) => {
                    errors.append(&mut e);
                }
                Ok((defs, _comments)) => {
                    lib_definitions.extend(defs);
                    // source_comments.insert(self.leak_str(dep.clone()), comments);
                }
            }
        }

        let mut module_files: Vec<PathBuf> = self.module_files();
        if let Some(fp) = file_path {
            if !module_files.contains(&fp) {
                module_files.push(fp);
            }
        }

        let mut source_definitions = Vec::new();
        let mut source_comments = CommentMap::new();
        for source_file_path in module_files {
            let fname = self.leak_str(source_file_path.clone());
            let source_text = self.source_text(source_file_path.clone());
            source_texts.insert(fname, source_text.clone());
            match self.ast(source_file_path.clone()) {
                Err(mut e) => {
                    errors.append(&mut e);
                }
                Ok((defs, comments)) => {
                    source_definitions.extend(defs);
                    source_comments.insert(self.leak_str(source_file_path.clone()), comments);
                }
            }
        }

        let program = ast::Program {
            lib_definitions,
            source_definitions,
        };
        if errors.is_empty() {
            (source_texts, Ok((program, source_comments)))
        } else {
            (source_texts, Err(errors))
        }
    }
}

#[allow(unused)]
fn goto_definition(
    db: &dyn TextSource,
    doc: PathBuf,
    pos: lsp_types::Position,
) -> Option<Location> {
    let text = db.source_text(doc);
    let rope = Rope::from(text);
    let tree = parser().parse_with(&mut |offset, _pos| get_chunk(&rope, offset), None)?;
    let offset = position_to_offset(&rope, pos)?;
    let leaf = tree.root_node().descendant_for_byte_range(offset, offset)?;
    let resolved_result = NodeResolver::resolve(&leaf, &tree.root_node())?;

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    pub trait InputChangeNotifier {
        fn read(&self, key: u32) -> u32;
        fn did_change_value(&mut self, key: u32);
    }
    #[salsa::query_group(LazyInputTestStorage)]
    pub trait LazyInputQueryGroup: InputChangeNotifier {
        fn read_value(&self, key: u32) -> u32;
        fn mul2(&self, key: u32) -> u32;
    }

    fn read_value(db: &dyn LazyInputQueryGroup, key: u32) -> u32 {
        db.salsa_runtime()
            .report_synthetic_read(salsa::Durability::LOW);
        db.read(key)
    }
    fn mul2(db: &dyn LazyInputQueryGroup, key: u32) -> u32 {
        db.read_value(key) * 2
    }

    #[salsa::database(LazyInputTestStorage)]
    #[derive(Default)]
    pub struct TestDatabase {
        storage: salsa::Storage<Self>,
        numbers: HashMap<u32, u32>,
    }
    impl salsa::Database for TestDatabase {}
    impl InputChangeNotifier for TestDatabase {
        fn read(&self, key: u32) -> u32 {
            *self.numbers.get(&key).unwrap()
        }

        fn did_change_value(&mut self, key: u32) {
            ReadValueQuery.in_db_mut(self).invalidate(&key)
        }
    }
    impl TestDatabase {
        pub fn set_key(&mut self, k: u32, v: u32) {
            self.numbers.insert(k, v);
            self.did_change_value(k);
        }
    }

    #[test]
    pub fn test_invalidate() {
        let mut db = TestDatabase::default();

        // init
        db.set_key(1, 11);
        assert_eq!(db.read_value(1), 11);
        assert_eq!(db.mul2(1), 22);

        // invalidate key
        db.set_key(1, 111);
        // it's ok
        assert_eq!(db.read_value(1), 111);

        // but, this will fail.
        assert_eq!(db.mul2(1), 222);
    }

    #[test]
    pub fn test_check_file() {
        let mut db = RootDatabase::default();
        db.set_stdlib_files(vec![]);
        db.set_module_files(vec![]);
        db.set_sender(Address::parse_str("0x01").ok());
        let path = PathBuf::from("/test.move");

        {
            let source = r"
            module A {
            }
            ";

            db.update_source(path.clone(), Rope::from_str(source).unwrap());
            let (sources, ast) = db.check_file(None, path.clone());
            assert_eq!(sources.len(), 1);

            assert!(ast.is_ok());
        }

        {
            let wrong_source = r"
            m A {}
            ";
            db.update_source(path.clone(), Rope::from_leaf(wrong_source.to_string()));
            let (sources, ast) = db.check_file(None, path.clone());
            assert_eq!(sources.len(), 1);
            assert!(ast.is_err());
        }
    }

    #[test]
    pub fn test_ast() {
        let mut db = RootDatabase::default();
        db.set_stdlib_files(vec![]);
        db.set_module_files(vec![]);
        db.set_sender(Address::parse_str("0x01").ok());

        let path = PathBuf::from("/test.move");
        {
            let source = r"
        module A {
        }
        ";

            db.update_source(path.clone(), Rope::from_str(source).unwrap());
            let source_text = db.source_text(path.clone());
            assert_eq!(source_text.as_str(), source);
            let ast = db.ast(path.clone());
            assert!(ast.is_ok());
        }

        {
            let wrong_source = r"
        m A {}
        ";
            db.update_source(path.clone(), Rope::from_leaf(wrong_source.to_string()));

            let new_source_text = db.source_text(path.clone());
            assert_eq!(new_source_text.as_str(), wrong_source);
            let new_ast = db.ast(path.clone());
            assert!(new_ast.is_err());
        }
    }
}
