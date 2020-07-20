extern crate log;

use move_lsp::lsp_server::MoveLanguageServer;
use tokio;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    serve_lsp().await;
}

async fn serve_lsp() {
    let move_server = MoveLanguageServer::default();
    let (service, msg_stream) = LspService::new(move_server);
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    Server::new(stdin, stdout)
        .interleave(msg_stream)
        .serve(service)
        .await;
}
