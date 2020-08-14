use crate::{
    config::ProjectConfig,
    error_diagnostic::{to_diagnostics, DiagnosticInfo},
    salsa::{Config, RootDatabase, TextSource},
    utils::find_move_file,
};
use anyhow::{bail, Result};
use dashmap::DashMap;
use futures::lock::Mutex;
use move_core_types::account_address::AccountAddress;
use move_lang::{
    errors::{Errors, FilesSourceText},
    shared::Address,
};
use serde::{Deserialize, Serialize};
use serde_json as json;
use serde_json::Value;
use std::{convert::TryFrom, path::PathBuf, str::FromStr};
use tower_lsp::{
    jsonrpc, lsp_types,
    lsp_types::{
        notification::Progress, ConfigurationItem, Diagnostic, DiagnosticRelatedInformation,
        DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
        DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
        ExecuteCommandOptions, ExecuteCommandParams, InitializeParams, InitializeResult,
        InitializedParams, Location, ProgressParams, ProgressParamsValue, SaveOptions,
        ServerCapabilities, ServerInfo, TextDocumentItem, TextDocumentSyncCapability,
        TextDocumentSyncKind, TextDocumentSyncOptions, Url, WorkDoneProgress,
        WorkDoneProgressBegin, WorkDoneProgressEnd, WorkDoneProgressOptions,
        WorkDoneProgressParams, WorkspaceCapability, WorkspaceFolderCapability,
    },
    Client, LanguageServer,
};

pub const LANGUAGE_ID: &str = "move";
pub struct MoveLanguageServer {
    inner: Mutex<Inner>,
}

impl MoveLanguageServer {
    pub fn new(client: Client) -> Self {
        let inner = Inner {
            db: RootDatabase::default(),
            config: ProjectConfig::default(),
            docs: Default::default(),
            client,
        };
        Self {
            inner: Mutex::new(inner),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for MoveLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        info!("{:#?}", &params);
        let mut guard = self.inner.lock().await;
        guard
            .initialize(params)
            .await
            .map_err(|e| jsonrpc::Error::invalid_params(format!("fail to initialize, {}", e)))
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("move language server initialized");
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, _params: DidChangeConfigurationParams) {
        let mut guard = self.inner.lock().await;
        let client = guard.client.clone();
        let config = async {
            let configuration_req = ConfigurationItem {
                scope_uri: None,
                section: None,
            };
            let config = client.configuration(vec![configuration_req]).await;
            match config?.pop() {
                None => bail!("client respond empty config data"),
                Some(config) => match json::from_value::<ProjectConfig>(config) {
                    Err(e) => bail!("cannot deserialize config data, {:?}", e),
                    Ok(c) => Ok(c),
                },
            }
        }
        .await;

        match config {
            Err(e) => {
                info!("Fetch client configuration failure: {:?}", e);
            }
            Ok(c) => {
                guard.handle_config_change(c);
            }
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> jsonrpc::Result<Option<Value>> {
        let ExecuteCommandParams {
            command,
            mut arguments,
            work_done_progress_params: WorkDoneProgressParams { work_done_token },
        } = params;

        let mut guard = self.inner.lock().await;
        let client = guard.client.clone();
        match command.as_str() {
            "compile" => {
                let arg = arguments.pop().ok_or_else(|| {
                    jsonrpc::Error::invalid_params("no arguments found for compile command")
                })?;

                let sender_opt = match arguments
                    .pop()
                    .as_ref()
                    .and_then(|s| s.as_str())
                    .map(|s| AccountAddress::from_hex_literal(s))
                    .transpose()
                {
                    Err(e) => {
                        let err_msg = format!("invalid sender address, {}", e);
                        return Ok(Some(Value::String(err_msg)));
                    }
                    Ok(sender) => sender.map(|s| Address::try_from(s.as_ref()).unwrap()),
                };

                let args: CompilationArgs = serde_json::from_value(arg).map_err(|e| {
                    jsonrpc::Error::invalid_params(format!(
                        "fail to parse compile arguments, {}",
                        e
                    ))
                })?;
                if work_done_token.is_some() {
                    client.send_custom_notification::<Progress>(ProgressParams {
                        token: work_done_token.clone().unwrap(),
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                            WorkDoneProgressBegin {
                                title: "Compiling".to_string(),
                                cancellable: None,
                                message: None,
                                percentage: None,
                            },
                        )),
                    })
                }

                let result = guard.do_compilation(sender_opt, args);

                if work_done_token.is_some() {
                    client.send_custom_notification::<Progress>(ProgressParams {
                        token: work_done_token.unwrap(),
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                            WorkDoneProgressEnd {
                                message: Some("Compile Done".to_string()),
                            },
                        )),
                    })
                }

                match result {
                    Ok(_) => Ok(None),
                    Err(e) => Ok(Some(Value::String(e))),
                }
            }
            _ => Ok(None),
        }
    }
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if params.text_document.language_id.as_str() != LANGUAGE_ID {
            return;
        }
        let mut guard = self.inner.lock().await;
        guard.handle_file_open(params);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut guard = self.inner.lock().await;
        guard.handle_file_change(params);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let mut guard = self.inner.lock().await;
        guard.handle_file_save(params);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut guard = self.inner.lock().await;
        guard.handle_file_close(params);
    }

    // async fn goto_declaration(
    //     &self,
    //     params: GotoDeclarationParams,
    // ) -> jsonrpc::Result<Option<GotoDeclarationResponse>> {
    //     let GotoDeclarationParams {
    //         text_document_position_params,
    //         work_done_progress_params,
    //         partial_result_params,
    //     } = params;
    //     text_document_position_params.position;
    //     error!("Got a textDocument/declaration request, but it is not implemented");
    //     Err(Error::method_not_found())
    // }
}

pub struct Inner {
    db: RootDatabase,
    config: ProjectConfig,
    docs: DashMap<Url, i64>,
    client: Client,
}

#[derive(Clone, Debug, Default)]
pub struct ConfChange(pub ProjectConfig);

impl Inner {
    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        let InitializeParams {
            initialization_options,
            ..
        } = params;

        if let Some(initial_config) = initialization_options {
            let conf = serde_json::from_value(initial_config)
                .map_err(|e| jsonrpc::Error::invalid_params(format!("invalid config, {}", e)))?;
            self.handle_config_change(conf);
        }

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
                        save: Some(lsp_types::TextDocumentSyncSaveOptions::SaveOptions(
                            SaveOptions {
                                include_text: Some(false),
                            },
                        )),
                        ..Default::default()
                    },
                )),
                workspace: Some(WorkspaceCapability {
                    workspace_folders: Some(WorkspaceFolderCapability {
                        supported: Some(false),
                        change_notifications: None,
                    }),
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["compile".to_string()],
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(true),
                    },
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    fn handle_config_change(&mut self, new_config: ProjectConfig) {
        debug!("server config change to: {:?}", &new_config);

        self.config = new_config.clone();

        let stdlib_files = new_config
            .stdlib_folder
            .map(find_move_file)
            .unwrap_or_default();
        let module_files: Vec<_> = new_config
            .modules_folders
            .into_iter()
            .flat_map(find_move_file)
            .collect();

        self.db.set_stdlib_files(stdlib_files.clone());
        self.db.set_module_files(module_files.clone());
        self.db.set_sender(new_config.sender_address);

        for f in stdlib_files {
            match std::fs::read_to_string(f.as_path()) {
                Ok(content) => {
                    self.db.set_source_text(self.db.leak_str(f), content);
                }
                Err(e) => {
                    error!("fail to read stdlib path: {}, {}", f.as_path().display(), e);
                }
            }
        }

        for f in module_files {
            match std::fs::read_to_string(f.as_path()) {
                Ok(content) => {
                    self.db.set_source_text(self.db.leak_str(f), content);
                }
                Err(e) => {
                    error!("fail to read module path: {}, {}", f.as_path().display(), e);
                }
            }
        }
    }

    fn handle_file_open(&mut self, param: DidOpenTextDocumentParams) {
        let DidOpenTextDocumentParams {
            text_document:
                TextDocumentItem {
                    language_id: _,
                    text,
                    version,
                    uri,
                },
        } = param;
        self.docs.insert(uri.clone(), version);

        if let Ok(p) = uri.to_file_path() {
            self.db.set_source_text(self.db.leak_str(p), text);
        }
    }

    fn handle_file_change(&mut self, param: DidChangeTextDocumentParams) {
        let DidChangeTextDocumentParams {
            text_document,
            mut content_changes,
        } = param;

        if let Some(v) = text_document.version {
            self.docs.insert(text_document.uri.clone(), v);
        }

        if let Ok(p) = text_document.uri.to_file_path() {
            if let Some(s) = content_changes.pop() {
                self.db.set_source_text(self.db.leak_str(p), s.text);
            }
        }
    }

    fn handle_file_close(&mut self, param: DidCloseTextDocumentParams) {
        let DidCloseTextDocumentParams { text_document } = param;
        self.docs.remove(&text_document.uri);
    }

    fn handle_file_save(&mut self, param: DidSaveTextDocumentParams) {
        let DidSaveTextDocumentParams { text_document } = param;
        let source_path = text_document.uri.to_file_path().unwrap();

        let (sources, result) = self.db.check_file(None, source_path);

        let errors = result.err().unwrap_or_default();
        self.publish_diagnostics(sources, errors);
    }

    fn publish_diagnostics(&self, sources: FilesSourceText, errs: Errors) {
        let mut diags = to_diagnostics(sources, errs);

        for f in self.docs.iter() {
            let (doc, version) = (f.key(), *f.value());

            debug!("publish diagnostic for {}", doc.path());

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
                                let url = Url::from_file_path(PathBuf::from_str(l.file).unwrap())
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

            self.client
                .publish_diagnostics(doc.clone(), diag, Some(version as i64));
        }
    }

    fn do_compilation(
        &mut self,
        sender: Option<Address>,
        arg: CompilationArgs,
    ) -> Result<(), String> {
        let CompilationArgs { file, out_dir } = arg;

        if let Ok(p) = file.to_file_path() {
            match self.db.compile_file(sender, p) {
                (s, Ok(u)) => move_lang::output_compiled_units(
                    true,
                    s,
                    u,
                    out_dir.as_path().to_string_lossy().as_ref(),
                )
                .map_err(|e| format!("{}", e)),
                (s, Err(e)) => Err(String::from_utf8_lossy(
                    move_lang::errors::report_errors_to_buffer(s, e).as_slice(),
                )
                .to_string()),
            }
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompilationArgs {
    file: Url,
    out_dir: PathBuf,
}
