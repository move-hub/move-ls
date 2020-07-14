use codespan::{FileId, Files};
use move_ir_types::location::Loc;
use move_lang::errors::{Error, ErrorSlice, Errors, FilesSourceText, HashableError};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticRelatedInformation};

fn to_diagnostics(sources: FilesSourceText, errs: Errors) {
    let mut files = codespan9::Files::new();
    let mut file_map = HashMap::with_capacity(sources.len());

    for (name, content) in sources {
        let file_id = files.add(name, content);
        file_map.insert(name, file_id);
    }

    for err in errs {
        for (loc, msg) in err {
            let file_name = loc.file();
            let span = loc.span();
            if let Some(file_id) = file_map.get(file_name) {
                let err_span = files
                    .location(*file_id, span.start())
                    .and_then(|start_location| {
                        files
                            .location(*file_id, span.end())
                            .map(|end| (start_location, end))
                    });
                if let Ok((s, e)) = err_span {
                    let s = lsp_types::Position::new(
                        s.line.to_usize() as u64,
                        s.column.to_usize() as u64,
                    );
                    let e = lsp_types::Position::new(
                        e.line.to_usize() as u64,
                        e.column.to_usize() as u64,
                    );
                    let range = lsp_types::Range::new(s, e);
                    let diagnostic = lsp_types::Diagnostic::new_simple(range, msg);
                }
            }
        }
    }
}

fn render_errors(files: &codespan9::Files<String>, file_mapping: &FileMapping, mut errors: Errors) {
    errors.sort_by(|e1, e2| {
        let loc1: &Loc = &e1[0].0;
        let loc2: &Loc = &e2[0].0;
        loc1.cmp(loc2)
    });
    let mut seen: HashSet<HashableError> = HashSet::new();
    for error in errors.into_iter() {
        let hashable_error = hashable_error(&error);
        if seen.contains(&hashable_error) {
            continue;
        }
        seen.insert(hashable_error);

        // let err = render_error(files, file_mapping, error);
        // emit(writer, &Config::default(), &files, &err).unwrap()
    }
}

fn render_error(files: &Files<String>, file_mapping: &FileMapping, mut error: Error) {
    let primary_err = error.remove(0);
    let secondary_errs: Vec<_> = error
        .into_iter()
        .map(|e| DiagnosticRelatedInformation {
            location: convert_loc(files, file_mapping, e.0).1.start,
            message: e.1,
        })
        .collect();

    Diagnostic {
        range: convert_loc(files, file_mapping, primary_err.0).1,
        severity: Some(lsp_types::DiagnosticSeverity::Error),
        code: None,
        source: None,
        message: primary_err.1,
        related_information: Some(secondary_errs),
        tags: None,
    };
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
