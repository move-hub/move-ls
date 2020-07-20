use serde::{Deserialize, Serialize};
use serde_json as json;

use crate::error_diagnostic::{to_diagnostics, DiagnosticInfo};
use crate::move_document::MoveDocument;
use crate::utils::find_move_file;
use bastion::prelude::*;
use codespan;
use dashmap::DashMap;
use move_core_types::account_address::AccountAddress;
use move_ir_types::location::Loc;
use move_lang::errors::{ErrorSlice, Errors, FilesSourceText, HashableError};
use move_lang::find_move_filenames;
use move_lang::shared::Address;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tower_lsp::jsonrpc;
use tower_lsp::jsonrpc::Error;
use tower_lsp::lsp_types;
use tower_lsp::lsp_types::request::{GotoDeclarationParams, GotoDeclarationResponse};
use tower_lsp::lsp_types::{
    ClientCapabilities, ConfigurationItem, Diagnostic, DiagnosticRelatedInformation,
    DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    ExecuteCommandOptions, InitializeParams, InitializeResult, InitializedParams, Location,
    MessageType, SaveOptions, ServerCapabilities, ServerInfo, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, TraceOption, Url,
    WorkDoneProgressOptions, WorkspaceCapability, WorkspaceFolderCapability,
};
use tower_lsp::{Client, LanguageServer};

pub const LANGUAGE_ID: &str = "move";

#[derive(Default)]
pub struct MoveLanguageServer {
    config: Arc<RwLock<ProjectConfig>>,
    docs: Arc<DashMap<Url, MoveDocument>>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub dialect: String,
    pub stdlib_folder: Option<PathBuf>,
    pub modules_folders: Vec<PathBuf>,
    pub sender_address: String,
}

#[tower_lsp::async_trait]
impl LanguageServer for MoveLanguageServer {
    async fn initialize(
        &self,
        client: &Client,
        params: InitializeParams,
    ) -> jsonrpc::Result<InitializeResult> {
        info!("{:?}", &params);
        let InitializeParams {
            process_id,
            root_path: _,
            root_uri,
            initialization_options,
            capabilities,
            trace,
            workspace_folders,
            client_info,
        } = params;
        if let Some(initial_config) = initialization_options {
            match serde_json::from_value(initial_config) {
                Err(e) => {
                    return Err(jsonrpc::Error::invalid_params_with_details(
                        "invalid config",
                        e,
                    ));
                }
                Ok(c) => {
                    *self.config.write() = c;
                }
            }
        }

        // let ClientCapabilities {
        //     workspace,
        //     text_document,
        //     window,
        //     experimental,
        // } = capabilities;
        //

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "move language server".to_string(),
                version: None,
            }),
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
        })
    }

    async fn initialized(&self, client: &Client, _: InitializedParams) {
        client.log_message(MessageType::Info, "move language server initialized");
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_change_configuration(
        &self,
        client: &Client,
        _params: DidChangeConfigurationParams,
    ) {
        let configuration_req = ConfigurationItem {
            scope_uri: None,
            section: Some("move".to_string()),
        };
        let config = client.configuration(vec![configuration_req]).await;

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

                            *self.config.write() = c;
                        }
                    }
                }
            },
        }
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
        if language_id != LANGUAGE_ID {
            return;
        }

        let doc = MoveDocument::new(version as u64, text.as_str());
        match doc {
            Err(e) => {
                client.log_message(MessageType::Error, e);
            }
            Ok(d) => {
                self.docs.insert(uri, d);
            }
        }
    }
    async fn did_change(&self, client: &Client, params: DidChangeTextDocumentParams) {
        let _ = client;
        let DidChangeTextDocumentParams {
            text_document,
            mut content_changes,
        } = params;
        if let Some(mut doc) = self.docs.get_mut(&text_document.uri) {
            // TODO: check version

            for c in content_changes {
                doc.reset_with(c.text).expect("should parse doc ok");
                // if let Some(r) = c.range {
                //     doc.value_mut().edit(r, c.text.as_str());
                // }
            }
            if let Some(v) = text_document.version {
                doc.incr_version(v as u64);
            }
        } else {
            client.log_message(
                MessageType::Warning,
                format!("no doc found for url {}", &text_document.uri),
            );
        }
    }

    async fn did_save(&self, client: &Client, params: DidSaveTextDocumentParams) {
        let _ = client;
        let DidSaveTextDocumentParams { text_document } = params;

        let source_path = text_document.uri.to_file_path().unwrap();
        let source_path_str = source_path.to_string_lossy().to_string();
        let config = self.config.read().clone();
        let deps = {
            let mut deps = config
                .stdlib_folder
                .as_ref()
                .map(|f| vec![f.to_string_lossy().to_string()])
                .unwrap_or_default();
            let mod_deps = config
                .modules_folders
                .iter()
                .flat_map(
                    |p| match find_move_filenames(&[p.to_string_lossy().to_string()]) {
                        Ok(t) => t,
                        Err(e) => vec![],
                    },
                )
                .filter(|p| p != &source_path_str);
            deps.extend(mod_deps);
            deps
        };

        let account_address = Address::parse_str(config.sender_address.as_str()).ok();
        let result = move_lang::move_check_no_report(&[source_path_str], &deps, account_address);

        match result {
            Err(e) => {
                client.log_message(
                    MessageType::Error,
                    format!(
                        "fail to check file {}, error: {}",
                        &source_path.display(),
                        &e
                    ),
                );
            }
            Ok((s, errs)) => {
                let opened_docs: Vec<_> = self
                    .docs
                    .iter()
                    .map(|f| (f.key().clone(), f.value().version()))
                    .collect();
                let mut diags = to_diagnostics(s, errs);

                for (doc, version) in opened_docs {
                    client.log_message(
                        MessageType::Info,
                        format!("publish diagnostic for {}", doc.path()),
                    );

                    let diag = if let Some(diag) = diags.remove(doc.path()) {
                        // let file_url = Url::from_file_path(PathBuf::from_str(fname).unwrap()).unwrap();
                        diag.into_iter()
                            .map(|d| {
                                let DiagnosticInfo {
                                    primary_label,
                                    secondary_labels,
                                } = d;
                                let related_infos: Vec<_> = secondary_labels
                                    .into_iter()
                                    .map(|l| {
                                        let url =
                                            Url::from_file_path(PathBuf::from_str(l.file).unwrap())
                                                .unwrap();
                                        DiagnosticRelatedInformation {
                                            location: Location::new(url, l.range),
                                            message: l.msg,
                                        }
                                    })
                                    .collect();
                                Diagnostic {
                                    range: primary_label.range,
                                    severity: Some(DiagnosticSeverity::Error),
                                    message: primary_label.msg,
                                    related_information: Some(related_infos),
                                    ..Default::default()
                                }
                            })
                            .collect()
                    } else {
                        vec![]
                    };
                    client.publish_diagnostics(doc, diag, Some(version as i64));
                }
            }
        }
    }

    async fn did_close(&self, client: &Client, params: DidCloseTextDocumentParams) {
        let DidCloseTextDocumentParams { text_document } = params;
        let removed_doc = self.docs.remove(&text_document.uri);
        if removed_doc.is_none() {
            client.log_message(
                MessageType::Warning,
                format!("no doc found for uri {}", &text_document.uri),
            );
        }
    }

    async fn goto_declaration(
        &self,
        params: GotoDeclarationParams,
    ) -> jsonrpc::Result<Option<GotoDeclarationResponse>> {
        let GotoDeclarationParams {
            text_document_position_params,
            work_done_progress_params,
            partial_result_params,
        } = params;
        text_document_position_params.position;
        error!("Got a textDocument/declaration request, but it is not implemented");
        Err(Error::method_not_found())
    }
}
