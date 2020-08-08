use bastion::Bastion;
use move_language_server::lsp_server::{FrontEnd, MoveLanguageServer};

use tower_lsp::{LspService, Server};

pub mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

fn main() {
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

    let mut rt = tokio::runtime::Builder::new()
        .threaded_scheduler()
        .thread_name("towe_lsp_server_rt")
        .build()
        .unwrap();

    let rt_handle = rt.handle().clone();
    Bastion::start();
    let supervisor = Bastion::supervisor(|sp| sp).unwrap();
    let backend_ref = supervisor
        .children(|ch| {
            ch.with_exec(move |ctx| {
                let rt_handle = rt_handle.clone();
                async move {
                    let mut server = MoveLanguageServer::new(rt_handle);
                    loop {
                        let msg = ctx.recv().await?;
                        server.handle_msg(&ctx, msg).await?;
                    }
                }
            })
        })
        .unwrap();

    // start server

    let join_handle = rt.spawn(async {
        let lsp_service = FrontEnd::new(backend_ref);
        let (service, msg_stream) = LspService::new(lsp_service);
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        Server::new(stdin, stdout)
            .interleave(msg_stream)
            .serve(service)
            .await;
    });
    Bastion::block_until_stopped();
    let _ = rt.block_on(join_handle).unwrap();
}
