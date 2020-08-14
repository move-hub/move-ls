use move_language_server::lsp_server::MoveLanguageServer;
use tower_lsp::{LspService, Server};

pub mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[tokio::main]
async fn main() {
    env_logger::init();

    log::info!(
        "Version {}, built for {} by {} at {}.",
        built_info::PKG_VERSION,
        built_info::TARGET,
        built_info::RUSTC_VERSION,
        built_info::BUILT_TIME_UTC
    );
    if let (Some(v), Some(dirty), Some(hash)) = (
        built_info::GIT_VERSION,
        built_info::GIT_DIRTY,
        built_info::GIT_COMMIT_HASH,
    ) {
        log::info!(
            "git `{}`, commit {}({}).",
            v,
            hash,
            if dirty { "dirty" } else { "clean" }
        );
    }
    log::info!("Starting language server");

    // let mut rt = tokio::runtime::Builder::new()
    //     .threaded_scheduler()
    //     .thread_name("towe_lsp_server_rt")
    //     .build()
    //     .unwrap();
    //
    // let rt_handle = rt.handle().clone();

    // start server
    let (service, msg_stream) = LspService::new(|client| MoveLanguageServer::new(client));
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    Server::new(stdin, stdout)
        .interleave(msg_stream)
        .serve(service)
        .await;
}
