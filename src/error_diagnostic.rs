use codespan::{FileId, Files};
use move_ir_types::location::Loc;
use move_lang::errors::{Error, ErrorSlice, Errors, FilesSourceText, HashableError};
use std::collections::{hash_map::RandomState, HashMap, HashSet};
use tower_lsp::{
    lsp_types,
    lsp_types::{Diagnostic, DiagnosticRelatedInformation, Range},
};

#[derive(Clone, Debug)]
pub struct DiagnosticInfo {
    pub primary_label: Label,
    pub secondary_labels: Vec<Label>,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub file: &'static str,
    pub range: Range,
    pub msg: String,
}

pub fn to_diagnostics(
    sources: FilesSourceText,
    errs: Errors,
) -> HashMap<&'static str, Vec<DiagnosticInfo>> {
    let mut files = Files::new();
    let mut file_map = HashMap::with_capacity(sources.len());

    for (name, content) in sources {
        let file_id = files.add(name, content);
        file_map.insert(name, file_id);
    }

    render_errors(&files, &file_map, errs)
}

type FileName = &'static str;
fn render_errors(
    files: &Files<String>,
    file_mapping: &FileMapping,
    mut errors: Errors,
) -> HashMap<FileName, Vec<DiagnosticInfo>> {
    errors.sort_by(|e1, e2| {
        let loc1: &Loc = &e1[0].0;
        let loc2: &Loc = &e2[0].0;
        loc1.cmp(loc2)
    });
    let mut seen: HashSet<HashableError> = HashSet::new();
    let mut diagnostics = HashMap::new();
    for error in errors.into_iter() {
        let hashable_error = hashable_error(&error);
        if seen.contains(&hashable_error) {
            continue;
        }
        seen.insert(hashable_error);

        let diagnostic = render_error(files, file_mapping, error);
        diagnostics
            .entry(diagnostic.primary_label.file)
            .or_insert_with(Vec::new)
            .push(diagnostic);
    }

    diagnostics
}

fn render_error(files: &Files<String>, file_mapping: &FileMapping, error: Error) -> DiagnosticInfo {
    let mut spans: Vec<_> = error
        .into_iter()
        .map(|e| Label {
            file: e.0.file(),
            range: convert_loc(files, file_mapping, e.0).1,
            msg: e.1,
        })
        .collect();
    let primary_label = spans.remove(0);
    DiagnosticInfo {
        primary_label,
        secondary_labels: spans,
    }
}
type FileMapping = HashMap<&'static str, FileId>;

fn convert_loc(
    files: &Files<String>,
    file_mapping: &FileMapping,
    loc: Loc,
) -> (FileId, lsp_types::Range) {
    let file_name = loc.file();
    let span = loc.span();
    let file_id = file_mapping.get(file_name).unwrap();

    let err_span = files
        .location(*file_id, span.start())
        .and_then(|start_location| {
            files
                .location(*file_id, span.end())
                .map(|end| (start_location, end))
        });
    let (s, e) = err_span.unwrap();

    let s = lsp_types::Position::new(s.line.to_usize() as u64, s.column.to_usize() as u64);
    let e = lsp_types::Position::new(e.line.to_usize() as u64, e.column.to_usize() as u64);
    let range = lsp_types::Range::new(s, e);

    (*file_id, range)
}

// TODO: public in libra
fn hashable_error(error: &ErrorSlice) -> HashableError {
    error
        .iter()
        .map(|(loc, e)| {
            (
                loc.file(),
                loc.span().start().to_usize(),
                loc.span().end().to_usize(),
                e.clone(),
            )
        })
        .collect()
}
