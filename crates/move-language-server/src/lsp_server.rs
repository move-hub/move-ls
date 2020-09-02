use crate::{
    config::ProjectConfig,
    error_diagnostic::{to_diagnostics, DiagnosticInfo},
    move_document::MoveDocument,
    salsa::{config_query::Config, text_source_query::SourceReader, RootDatabase},
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
        notification::{Notification, Progress},
        ConfigurationItem, Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
        DidChangeConfigurationParams, DidChangeTextDocumentParams,
        DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
        DidOpenTextDocumentParams, DidSaveTextDocumentParams, ExecuteCommandOptions,
        ExecuteCommandParams, GotoDefinitionParams, GotoDefinitionResponse, InitializeParams,
        InitializeResult, InitializedParams, Location, ProgressParams, ProgressParamsValue,
        Registration, SaveOptions, ServerCapabilities, ServerInfo, TextDocumentItem,
        TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind,
        TextDocumentSyncOptions, Unregistration, Url, WorkDoneProgress, WorkDoneProgressBegin,
        WorkDoneProgressEnd, WorkDoneProgressOptions, WorkDoneProgressParams, WorkspaceCapability,
        WorkspaceFolderCapability,
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
            file_watch_registration: Default::default(),
            client_capabilities: Default::default(),
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
        let mut guard = self.inner.lock().await;
        guard.register_file_watch().await;
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
                panic!("Fetch client configuration failure: {:?}", e);
            }
            Ok(c) => {
                guard.handle_config_change(c);
                guard.register_file_watch().await;
            }
        }
    }

    async fn did_change_watched_files(&self, params: lsp_types::DidChangeWatchedFilesParams) {
        let lsp_types::DidChangeWatchedFilesParams { changes } = params;
        let mut inner = self.inner.lock().await;
        inner.handle_watched_files_change(changes);
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
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let GotoDefinitionParams {
            text_document_position_params:
                TextDocumentPositionParams {
                    text_document: _,
                    position: _,
                },
            work_done_progress_params: _,
            partial_result_params: _,
        } = params;

        error!("Got a textDocument/definition request, but it is not implemented");
        Err(jsonrpc::Error::method_not_found())
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
    docs: DashMap<Url, MoveDocument>,
    client: Client,
    file_watch_registration: uuid::Uuid,
    client_capabilities: lsp_types::ClientCapabilities,
}

fn _assert_object_safe() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Inner>();
    assert_send::<RootDatabase>();
    assert_send::<MoveLanguageServer>();
    assert_sync::<MoveLanguageServer>();
}

#[derive(Clone, Debug, Default)]
pub struct ConfChange(pub ProjectConfig);

impl Inner {
    async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        let InitializeParams {
            initialization_options,
            capabilities,
            ..
        } = params;
        self.client_capabilities = capabilities;

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
                        change: Some(TextDocumentSyncKind::Incremental),
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

    async fn register_file_watch(&mut self) {
        let inner = self;

        // unregister old
        if !inner.file_watch_registration.is_nil() {
            inner
                .client
                .unregister_capability(vec![Unregistration {
                    id: inner.file_watch_registration.to_string(),
                    method: lsp_types::notification::DidChangeWatchedFiles::METHOD.to_string(),
                }])
                .await
                .expect("should unregister didChangeWatchedFiles");
        }

        let mut file_watchers = vec![];
        if let Some(stdlib) = inner.config.stdlib_folder.as_ref() {
            let w = lsp_types::FileSystemWatcher {
                glob_pattern: stdlib.join("**/*.move").display().to_string(),
                kind: Some(
                    lsp_types::WatchKind::Create
                        | lsp_types::WatchKind::Delete
                        | lsp_types::WatchKind::Change,
                ),
            };
            file_watchers.push(w);
        }
        for module_folder in inner.config.modules_folders.as_slice() {
            let w = lsp_types::FileSystemWatcher {
                glob_pattern: module_folder.join("**/*.move").display().to_string(),
                kind: Some(lsp_types::WatchKind::Create | lsp_types::WatchKind::Delete),
            };
            file_watchers.push(w);
        }

        let registration_options = DidChangeWatchedFilesRegistrationOptions {
            watchers: file_watchers,
        };

        let new_registration_id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            lsp_types::notification::DidChangeWatchedFiles::METHOD.as_bytes(),
        );
        inner.file_watch_registration = new_registration_id;
        inner
            .client
            .register_capability(vec![Registration {
                id: new_registration_id.to_string(),
                method: lsp_types::notification::DidChangeWatchedFiles::METHOD.to_string(),
                register_options: serde_json::to_value(registration_options).ok(),
            }])
            .await
            .expect("should register didChangeWatchedFiles");
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

        self.db
            .set_stdlib_files_with_durability(stdlib_files.clone(), salsa::Durability::HIGH);
        self.db
            .set_module_files_with_durability(module_files.clone(), salsa::Durability::HIGH);
        self.db
            .set_sender_with_durability(new_config.sender_address, salsa::Durability::HIGH);
    }

    fn handle_watched_files_change(&mut self, changes: Vec<lsp_types::FileEvent>) {
        let inner = self;
        for lsp_types::FileEvent { uri, typ } in changes {
            let fp = uri.to_file_path().expect("uri to be a file path");

            let is_stdlib_file = inner
                .config
                .stdlib_folder
                .as_ref()
                .filter(|stdlib_folder| fp.starts_with(stdlib_folder))
                .is_some();

            if matches!(typ, lsp_types::FileChangeType::Changed) {
                inner.db.did_change(fp.as_path());
                return;
            }

            let mut files = if is_stdlib_file {
                inner.db.stdlib_files()
            } else {
                inner.db.module_files()
            };
            let mut files_updated = false;
            match typ {
                lsp_types::FileChangeType::Created => {
                    if !files.contains(&fp) {
                        files.push(fp);
                        files_updated = true;
                    }
                }
                lsp_types::FileChangeType::Deleted => {
                    if let Some(idx) = files.iter().position(|x| x == &fp) {
                        files.remove(idx);
                        files_updated = true;
                    }
                }
                lsp_types::FileChangeType::Changed => {}
            }

            if !files_updated {
                return;
            }

            if is_stdlib_file {
                inner
                    .db
                    .set_stdlib_files_with_durability(files, salsa::Durability::HIGH);
            } else {
                inner
                    .db
                    .set_module_files_with_durability(files, salsa::Durability::HIGH);
            }
            inner.diagnose_with_optional_file(None);
        }
    }

    fn handle_file_open(&mut self, param: DidOpenTextDocumentParams) {
        debug!("file opened: {:?}", &param);
        let DidOpenTextDocumentParams {
            text_document:
                TextDocumentItem {
                    language_id: _,
                    text,
                    version,
                    uri,
                },
        } = param;
        let doc = MoveDocument::new(version as u64, text.as_str());
        if let Ok(p) = uri.to_file_path() {
            self.db.update_source(p.clone(), doc.doc().rope().clone());
            self.diagnose_with_optional_file(Some(p));
        }
        self.docs.insert(uri.clone(), doc);
    }

    fn handle_file_change(&mut self, param: DidChangeTextDocumentParams) {
        debug!("file changed: {:?}", &param);
        let DidChangeTextDocumentParams {
            text_document,
            content_changes,
        } = param;

        if let Some(mut doc) = self.docs.get_mut(&text_document.uri) {
            // incremental edit
            let changes = content_changes
                .into_iter()
                .map(|change| (change.range.unwrap(), change.text));
            doc.edit_many(text_document.version.unwrap() as u64, changes);
        }
        if let Some(rope) = self
            .docs
            .get(&text_document.uri)
            .map(|d| d.doc().rope().clone())
        {
            if let Ok(p) = text_document.uri.to_file_path() {
                self.db.update_source(p.clone(), rope);
                // recheck diagnostics
                self.diagnose_with_optional_file(Some(p));
            }
        }
    }

    fn handle_file_close(&mut self, param: DidCloseTextDocumentParams) {
        debug!("file closed: {:?}", &param);
        let DidCloseTextDocumentParams { text_document } = param;
        self.docs.remove(&text_document.uri);
        if let Ok(p) = text_document.uri.to_file_path() {
            self.db.close_source(p);
        }
    }

    fn handle_file_save(&mut self, param: DidSaveTextDocumentParams) {
        debug!("file saved: {:?}", &param);
        let DidSaveTextDocumentParams { text_document } = param;
        let source_path = text_document.uri.to_file_path().unwrap();

        self.diagnose_with_optional_file(Some(source_path));
    }

    fn diagnose_with_optional_file(&self, additional: Option<PathBuf>) {
        let (sources, result) = match additional {
            None => self.db.check_all(None),
            Some(fp) => self.db.check_file(None, fp),
        };
        let errors = result.err().unwrap_or_default();
        self.publish_diagnostics(sources, errors);
    }

    fn publish_diagnostics(&self, sources: FilesSourceText, errs: Errors) {
        let mut diags = to_diagnostics(sources, errs);

        for f in self.docs.iter() {
            let (doc, version) = (f.key(), f.doc().version());

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
