use crate::salsa::FileId;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

pub trait SourceReader {
    fn read(&self, file_id: FileId) -> Cow<str>;
    fn did_change(&mut self, filename: &Path);
}

#[salsa::query_group(SourceStorage)]
pub trait TextSource: SourceReader {
    fn source_text(&self, filename: PathBuf) -> String;

    fn leak_str(&self, file_name: PathBuf) -> &'static str;
}

fn leak_str(_source: &dyn TextSource, file_name: PathBuf) -> &'static str {
    Box::leak(Box::new(file_name.to_string_lossy().to_string()))
}

fn source_text(db: &dyn TextSource, file_id: FileId) -> String {
    db.salsa_runtime()
        .report_synthetic_read(salsa::Durability::LOW);
    db.salsa_runtime().report_untracked_read();
    db.read(file_id).to_string()
}
