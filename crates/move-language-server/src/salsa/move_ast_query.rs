use move_lang::{
    errors::Errors,
    parser::{ast, syntax},
    strip_comments_and_verify, MatchedFileCommentMap,
};
use std::path::PathBuf;

#[salsa::query_group(AstStorage)]
pub trait Ast: super::TextSource {
    fn ast(
        &self,
        file_name: PathBuf,
    ) -> Result<(Vec<ast::Definition>, MatchedFileCommentMap), Errors>;
}

fn ast(
    db: &dyn Ast,
    file_name: PathBuf,
) -> Result<(Vec<ast::Definition>, MatchedFileCommentMap), Errors> {
    let source = db.source_text(file_name.clone());
    let fname = db.leak_str(file_name);
    let (no_comments_buffer, comment_map, _regular_comment_map) =
        strip_comments_and_verify(fname, source.as_str())?;
    let (defs, comments) = syntax::parse_file_string(fname, &no_comments_buffer, comment_map)?;
    Ok((defs, comments))
}
