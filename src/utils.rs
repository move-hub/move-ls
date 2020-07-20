use move_lang::MOVE_EXTENSION;
use std::path::{Path, PathBuf};

pub fn find_move_file(path: PathBuf) -> Vec<PathBuf> {
    let has_move_extension = |path: &Path| match path.extension().and_then(|s| s.to_str()) {
        Some(extension) => extension == MOVE_EXTENSION,
        None => false,
    };

    let mut result = vec![];

    if !path.exists() {
        return result;
    }

    if !path.is_dir() {
        // If the filename is specified directly, add it to the list, regardless
        // of whether it has a ".move" extension.
        result.push(path);
    } else {
        for entry in walkdir::WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let entry_path = entry.path();

            if !entry.file_type().is_file() || !has_move_extension(&entry_path) {
                continue;
            }
            result.push(entry.into_path());
        }
    }
    result
}
