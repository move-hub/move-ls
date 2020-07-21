use crate::{
    move_document::{get_chunk, position_to_offset},
    node_resolver::NodeResolver,
    tree_sitter_move::parser,
};
use move_core_types::account_address::AccountAddress;
use move_lang::{
    compiled_unit::CompiledUnit,
    errors::{Errors, FilesSourceText},
    parser::ast,
    shared::Address,
    CommentMap, MatchedFileCommentMap,
};
use salsa::Database;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tower_lsp::{
    lsp_types,
    lsp_types::{Location, Position},
};
use xi_rope::Rope;

#[salsa::query_group(ConfigStorage)]
pub trait Config: salsa::Database {
    #[salsa::input]
    fn stdlib_files(&self) -> Vec<PathBuf>;

    #[salsa::input]
    fn module_files(&self) -> Vec<PathBuf>;

    #[salsa::input]
    fn sender(&self) -> Option<Address>;
}

#[salsa::query_group(SourceStorage)]
pub trait TextSource: salsa::Database {
    #[salsa::input]
    fn source_text(&self, file_name: &'static str) -> String;

    fn leak_str(&self, file_name: PathBuf) -> &'static str;

    fn move_parse(
        &self,
        file_name: PathBuf,
        source: String,
    ) -> Result<(Vec<ast::Definition>, MatchedFileCommentMap), Errors>;
}

fn leak_str(_source: &dyn TextSource, file_name: PathBuf) -> &'static str {
    Box::leak(Box::new(file_name.to_string_lossy().to_string()))
}

#[salsa::database(ConfigStorage, SourceStorage)]
#[derive(Default)]
pub struct RootDatabase {
    storage: salsa::Storage<Self>,
}
impl salsa::Database for RootDatabase {}

impl RootDatabase {
    pub fn compile_file(
        &self,
        file_path: PathBuf,
    ) -> (FilesSourceText, Result<Vec<CompiledUnit>, Errors>) {
        let (sources, cfg_program) = self.check_file(file_path);
        let compiled_result =
            cfg_program.and_then(|p| move_lang::to_bytecode::translate::program(p));
        (sources, compiled_result)
    }

    pub fn check_file(
        &self,
        file_path: PathBuf,
    ) -> (
        FilesSourceText,
        Result<move_lang::cfgir::ast::Program, Errors>,
    ) {
        let (sources, parsed_program) = self.parse_file(file_path.clone());

        let checked = move_lang::check_program(parsed_program.map(|(p, c)| p), self.sender());
        (sources, checked)
    }

    pub fn parse_file(
        &self,
        file_path: PathBuf,
    ) -> (FilesSourceText, Result<(ast::Program, CommentMap), Errors>) {
        let mut errors = Errors::new();

        let deps: Vec<PathBuf> = self.stdlib_files();
        let mut lib_definitions = Vec::new();
        let mut source_texts = FilesSourceText::default();

        for dep in deps {
            let fname = self.leak_str(dep.clone());
            let source_text = self.source_text(fname);
            source_texts.insert(fname, source_text.clone());
            match self.move_parse(dep.clone(), source_text) {
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
        if !module_files.contains(&file_path) {
            module_files.push(file_path);
        }
        let mut source_definitions = Vec::new();
        let mut source_comments = CommentMap::new();
        for source in module_files {
            let fname = self.leak_str(source.clone());
            let source_text = self.source_text(fname);
            source_texts.insert(fname, source_text.clone());
            match self.move_parse(source.clone(), source_text) {
                Err(mut e) => {
                    errors.append(&mut e);
                }
                Ok((defs, comments)) => {
                    lib_definitions.extend(defs);
                    source_comments.insert(self.leak_str(source.clone()), comments);
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

fn move_parse(
    db: &dyn TextSource,
    file_name: PathBuf,
    source: String,
) -> Result<(Vec<ast::Definition>, MatchedFileCommentMap), Errors> {
    let fname = db.leak_str(file_name);
    let (no_comments_buffer, comment_map, _regular_comment_map) =
        move_lang::strip_comments_and_verify(fname, source.as_str())?;
    let (defs, comments) =
        move_lang::parser::syntax::parse_file_string(fname, &no_comments_buffer, comment_map)?;
    Ok((defs, comments))
}

#[allow(unused)]
fn goto_definition(
    db: &dyn TextSource,
    doc: PathBuf,
    pos: lsp_types::Position,
) -> Option<Location> {
    let text = db.source_text(db.leak_str(doc));
    let rope = Rope::from(text.as_str());
    let tree = parser().parse_with(&mut |offset, _pos| get_chunk(&rope, offset), None)?;
    let offset = position_to_offset(&rope, pos);
    let leaf = tree.root_node().descendant_for_byte_range(offset, offset)?;
    let resolved_result = NodeResolver::resolve(&leaf, &tree.root_node())?;

    None
}
