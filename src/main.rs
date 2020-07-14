#[macro_use]
extern crate log;
use bastion::supervisor::SupervisionStrategy;
use bastion::{Bastion, Callbacks};
use move_lsp::lsp_server::MoveLanguageServer;
use tower_lsp::{LspService, Server};

fn main() {
    Bastion::init();
    Bastion::start();
    Bastion::children(|ch| {
        let callbacks = Callbacks::new()
            .with_before_start(|| info!("Prepare start move language server"))
            .with_after_stop(|| info!("move language server stopped"))
            .with_before_restart(|| warn!("Prepare to restart move language server"))
            .with_after_restart(|| warn!("move language server restarted"));

        ch.with_callbacks(callbacks).with_exec(|ctx| async move {
            serve_lsp().await;
            Ok(())
        })
    });

    Bastion::block_until_stopped();
}

async fn serve_lsp() {
    let move_server = MoveLanguageServer {};
    let (service, msg_stream) = LspService::new(move_server);
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    Server::new(stdin, stdout)
        .interleave(msg_stream)
        .serve(service)
        .await;
}
