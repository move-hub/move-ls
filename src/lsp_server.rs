#[macro_use]
extern crate log;

use serde::{Deserialize, Serialize};
use serde_json as json;

use bastion::prelude::*;
use codespan;
use dashmap::DashMap;
use move_ir_types::location::Loc;
use move_lang::errors::{ErrorSlice, Errors, FilesSourceText, HashableError};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tower_lsp::jsonrpc;
use tower_lsp::lsp_types;
use tower_lsp::lsp_types::{
    ConfigurationItem, Diagnostic, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    InitializeParams, InitializeResult, InitializedParams, MessageType, SaveOptions,
    ServerCapabilities, ServerInfo, TextDocumentItem, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, Url, WorkspaceCapability,
    WorkspaceFolderCapability,
};
use tower_lsp::{Client, LanguageServer};

pub struct MoveLanguageServer {
    ws: WorkspaceManager,
}

struct WorkspaceManager {
    config: Arc<RwLock<ProjectConfig>>,
    docs: DashMap<Url, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    stdlib_folder: String,
    modules_folder: String,
}

#[tower_lsp::async_trait]
impl LanguageServer for MoveLanguageServer {
    async fn initialize(
        &self,
        client: &Client,
        params: InitializeParams,
    ) -> jsonrpc::Result<InitializeResult> {
        info!("{:?}", params);
        if let Some(root_uri) = &params.root_uri {}
        // if let Some(ws_folders) = params.workspace_folders.as_ref() {
        //     for ws in ws_folders {
        //         ws.name
        //     }
        // }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::Full),
                        save: Some(SaveOptions {
                            include_text: Some(false),
                        }),
                        ..Default::default()
                    },
                )),
                workspace: Some(WorkspaceCapability {
                    workspace_folders: Some(WorkspaceFolderCapability {
                        supported: Some(false),
                        change_notifications: None,
                    }),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "move language server".to_string(),
                version: None,
            }),
        })
    }

    async fn initialized(&self, client: &Client, _: InitializedParams) {
        client.log_message(MessageType::Info, "move language server initialized");
        let configration_req = ConfigurationItem {
            scope_uri: None,
            section: Some("move".to_string()),
        };
        let config = client.configuration(vec![configration_req]).await;

        match config {
            Err(e) => {
                error!("Fetch client configuration failure: {:?}", e);
            }
            Ok(mut configs) => match configs.pop() {
                None => error!("client respond empty config data"),
                Some(config) => {
                    match json::from_value::<ProjectConfig>(config) {
                        Err(e) => error!("cannot deserialize config data, {:?}", e),
                        Ok(c) => {
                            // TODO: save config
                            info!("project config: {:?}", c);

                            *self.ws.config.write() = c;
                        }
                    }
                }
            },
        }
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_change_configuration(
        &self,
        client: &Client,
        params: DidChangeConfigurationParams,
    ) {
        // TODO: refetch config.
    }

    async fn did_open(&self, client: &Client, params: DidOpenTextDocumentParams) {
        let DidOpenTextDocumentParams {
            text_document:
                TextDocumentItem {
                    language_id,
                    text,
                    version,
                    uri,
                },
        } = params;

        // self.ws.docs.insert(uri, );
        let p = uri.to_file_path().unwrap();
    }
    async fn did_change(&self, client: &Client, params: DidChangeTextDocumentParams) {
        let _ = client;
        let DidChangeTextDocumentParams {
            text_document,
            content_changes,
        } = params;
        warn!("Got a textDocument/didChange notification, but it is not implemented");
    }
    async fn did_save(&self, client: &Client, params: DidSaveTextDocumentParams) {
        let _ = client;
        let DidSaveTextDocumentParams { text_document } = params;
        let source_path = text_document.uri.to_file_path().unwrap();
        let source_path = source_path.to_string_lossy().to_string();
        let result = move_lang::move_check_no_report(&[source_path], &[], None);
        // client.publish_diagnostics(text_document.uri, )
        match result {
            Err(e) => {
                error!("fail to check");
            }
            Ok((s, errs)) => {
                for es in errs {
                    for (loc, msg) in es {}
                }
            }
        }
        warn!("Got a textDocument/didSave notification, but it is not implemented");
    }

    async fn did_close(&self, client: &Client, params: DidCloseTextDocumentParams) {
        let _ = client;
        let _ = params;
        warn!("Got a textDocument/didClose notification, but it is not implemented");
    }
}
