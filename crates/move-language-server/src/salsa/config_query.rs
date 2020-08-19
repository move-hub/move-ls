use move_lang::shared::Address;
use std::path::PathBuf;

#[salsa::query_group(ConfigStorage)]
pub trait Config {
    #[salsa::input]
    fn stdlib_files(&self) -> Vec<PathBuf>;

    #[salsa::input]
    fn module_files(&self) -> Vec<PathBuf>;

    #[salsa::input]
    fn sender(&self) -> Option<Address>;
}
