use crate::{
    config::ProjectConfig,
    error_diagnostic::{to_diagnostics, DiagnosticInfo},
    move_document::MoveDocument,
    salsa::{Config, RootDatabase, TextSource},
    utils::find_move_file,
};
use anyhow::{bail, Result};
use bastion::prelude::*;
use codespan;
use dashmap::DashMap;
use futures::{channel::mpsc, StreamExt};
use move_core_types::account_address::AccountAddress;
use move_ir_types::location::Loc;
use move_lang::{
    errors::{ErrorSlice, Errors, FilesSourceText, HashableError},
    find_move_filenames,
    shared::Address,
};
use parking_lot::RwLock;
use salsa::Database;
use serde::{export::fmt::Display, Deserialize, Serialize};
use serde_json as json;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use tower_lsp::{
    jsonrpc,
    jsonrpc::Error,
    lsp_types,
    lsp_types::{
        notification::Progress,
        request::{GotoDeclarationParams, GotoDeclarationResponse},
        ClientCapabilities, ConfigurationItem, Diagnostic, DiagnosticRelatedInformation,
        DiagnosticSeverity, DidChangeConfigurationParams, DidChangeTextDocumentParams,
        DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
        ExecuteCommandOptions, ExecuteCommandParams, ExecuteCommandRegistrationOptions,
        InitializeParams, InitializeResult, InitializedParams, Location, MessageType,
        ProgressParams, ProgressParamsValue, ProgressToken, SaveOptions, ServerCapabilities,
        ServerInfo, TextDocumentItem, TextDocumentSyncCapability, TextDocumentSyncKind,
        TextDocumentSyncOptions, TraceOption, Url, WorkDoneProgress, WorkDoneProgressBegin,
        WorkDoneProgressEnd, WorkDoneProgressOptions, WorkDoneProgressParams, WorkspaceCapability,
        WorkspaceFolderCapability,
    },
    Client, LanguageServer,
};

pub const LANGUAGE_ID: &str = "move";

pub struct MoveLanguageServer {
    db: RootDatabase,
    config: ProjectConfig,
    docs: DashMap<Url, i64>,
    client: Option<Client>,
    handle: tokio::runtime::Handle,
}

impl MoveLanguageServer {
    pub fn new(rt_handle: tokio::runtime::Handle) -> Self {
        Self {
            handle: rt_handle,
            db: RootDatabase::default(),
            config: ProjectConfig::default(),
            docs: Default::default(),
            client: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConfChange(pub ProjectConfig);

impl MoveLanguageServer {
    pub async fn handle_msg(&mut self, ctx: &BastionContext, msg: SignedMessage) -> Result<(), ()> {
        bastion::msg! { msg,
            m: (Client, InitializeParams) =!> {
                let (c, p) = m;
                let result = self.initialize(c, p).await;
                let _ = answer!(ctx, result);
            };
            m: ConfChange => {
                self.handle_config_change(m.0);
            };
            m: DidOpenTextDocumentParams => {
                self.handle_file_open(m);
            };
            m: DidChangeTextDocumentParams => {
                self.handle_file_change(m);
            };
            m: DidCloseTextDocumentParams => {
                self.handle_file_close(m);
            };
            m: DidSaveTextDocumentParams => {
                self.handle_file_save(m);
            };
            m: CompilationAction =!> {
                let result = self.do_compilation(m);
                let _ = answer!(ctx, result);
            };
            _:_ => ();
        }
        Ok(())
    }

    async fn initialize(
        &mut self,
        client: Client,
        params: InitializeParams,
    ) -> Result<InitializeResult> {
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

        self.client = Some(client);

        if let Some(initial_config) = initialization_options {
            let conf = serde_json::from_value(initial_config)
                .map_err(|e| jsonrpc::Error::invalid_params_with_details("invalid config", e))?;
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
            .map(|p| find_move_file(p))
            .unwrap_or_default();
        let module_files: Vec<_> = new_config
            .modules_folders
            .into_iter()
            .flat_map(|p| find_move_file(p))
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
                    language_id,
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

        let (sources, result) = self.db.check_file(source_path);

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

            self.handle.enter(|| {
                self.client.as_ref().unwrap().publish_diagnostics(
                    doc.clone(),
                    diag,
                    Some(version as i64),
                )
            });
        }
    }

    fn do_compilation(&self, action: CompilationAction) -> Result<(), String> {
        let CompilationAction { file, out_dir } = action;

        if let Ok(p) = file.to_file_path() {
            match self.db.compile_file(p) {
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
pub struct CompilationAction {
    file: Url,
    out_dir: PathBuf,
}

pub struct FrontEnd {
    backend: ChildrenRef,
}

impl FrontEnd {
    pub fn new(backend: ChildrenRef) -> Self {
        Self { backend }
    }

    async fn ask<T, R>(&self, msg: T) -> Result<R, ()>
    where
        T: bastion::message::Message,
        R: bastion::message::Message,
    {
        let answer = self.backend.elems()[0]
            .ask_anonymously(msg)
            .map_err(|_| ())?;

        let response = answer.await?;
        let (var, _) = response.extract();
        if var.is::<R>() {
            Ok(var.downcast::<R>().unwrap())
        } else {
            Err(())
        }
    }

    fn try_tell<T>(&self, msg: T)
    where
        T: bastion::message::Message,
    {
        if let Err(p) = self.tell(msg) {
            error!("move language server fail to be notified about {:?}", p);
        }
    }

    fn tell<T>(&self, msg: T) -> Result<(), T>
    where
        T: bastion::message::Message,
    {
        self.backend.elems()[0].tell_anonymously(msg)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for FrontEnd {
    async fn initialize(
        &self,
        client: &Client,
        params: InitializeParams,
    ) -> jsonrpc::Result<InitializeResult> {
        info!("{:#?}", &params);
        self.ask::<_, Result<InitializeResult>>((client.clone(), params))
            .await
            .map_err(|_| jsonrpc::Error::internal_error())?
            .map_err(|e| jsonrpc::Error::invalid_params_with_details("fail to initialize", e))
    }

    async fn initialized(&self, client: &Client, _: InitializedParams) {
        info!("move language server initialized");
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        // stop bastion supervisor tree.
        Bastion::stop();
        Ok(())
    }

    async fn did_change_configuration(
        &self,
        client: &Client,
        _params: DidChangeConfigurationParams,
    ) {
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
                self.try_tell(ConfChange(c));
            }
        }
    }

    async fn execute_command(
        &self,
        client: &Client,
        params: ExecuteCommandParams,
    ) -> jsonrpc::Result<Option<Value>> {
        let ExecuteCommandParams {
            command,
            mut arguments,
            work_done_progress_params: WorkDoneProgressParams { work_done_token },
        } = params;

        match command.as_str() {
            "compile" => {
                let arg = arguments.pop().ok_or_else(|| {
                    jsonrpc::Error::invalid_params("no arguments found for compile command")
                })?;

                let arg: CompilationAction = serde_json::from_value(arg).map_err(|e| {
                    jsonrpc::Error::invalid_params_with_details(
                        "fail to parse compile arguments",
                        e,
                    )
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

                let result = self
                    .ask::<_, Result<(), String>>(arg)
                    .await
                    .map_err(|_| jsonrpc::Error::internal_error())?;

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
    async fn did_open(&self, client: &Client, params: DidOpenTextDocumentParams) {
        if params.text_document.language_id.as_str() != LANGUAGE_ID {
            return;
        }

        self.try_tell(params);
    }

    async fn did_change(&self, client: &Client, params: DidChangeTextDocumentParams) {
        self.try_tell(params);
    }

    async fn did_save(&self, client: &Client, params: DidSaveTextDocumentParams) {
        self.try_tell(params);
    }

    async fn did_close(&self, client: &Client, params: DidCloseTextDocumentParams) {
        self.try_tell(params);
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
