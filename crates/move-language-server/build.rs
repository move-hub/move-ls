use std::path::PathBuf;

fn main() {
    let dir: PathBuf = ["..", "..", "tree-sitter-move", "src"].iter().collect();

    cc::Build::new()
        .include(&dir)
        .file(dir.join("parser.c"))
        .compile("tree-sitter-move");

    built::write_built_file().expect("Failed to acquire build-time information");
}
