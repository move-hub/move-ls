use move_lang::{
    errors::Errors,
    parser::{ast, syntax},
    strip_comments_and_verify, FileCommentMap, MatchedFileCommentMap,
};
use std::path::PathBuf;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AstInfo {
    pub defs: Vec<ast::Definition>,
    pub doc_comments: MatchedFileCommentMap,
    pub comment_map: FileCommentMap,
    pub regular_comment_map: FileCommentMap,
}

#[salsa::query_group(AstStorage)]
pub trait Ast: super::TextSource {
    fn ast(&self, file_name: PathBuf) -> Result<AstInfo, Errors>;
}

fn ast(db: &dyn Ast, file_name: PathBuf) -> Result<AstInfo, Errors> {
    let source = db.source_text(file_name.clone());
    let fname = db.leak_str(file_name);
    let (no_comments_buffer, comment_map, regular_comment_map) =
        strip_comments_and_verify(fname, source.as_str())?;
    let (defs, comments) =
        syntax::parse_file_string(fname, &no_comments_buffer, comment_map.clone())?;
    Ok(AstInfo {
        defs,
        doc_comments: comments,
        comment_map,
        regular_comment_map,
    })
}
