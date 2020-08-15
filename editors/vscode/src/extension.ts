import * as fs from 'fs';
import * as path from 'path';

import * as vscode from 'vscode';
import * as lsp from 'vscode-languageclient';
import { ExecuteCommandRequest } from 'vscode-languageclient';

interface ExtensionConfig {
	languageServerPath: string | undefined,
	modulesPath: string,
	stdlibPath: string | undefined,
	targetDir: string,
	sender: string | undefined
}

interface LanguageServerConfig {
	dialect: string,
	modules_folders: string[],
	stdlib_folder: string | undefined,
	sender_address: string | undefined
}

const workspaceClients: Map<vscode.WorkspaceFolder, lsp.LanguageClient> = new Map();
/**
 * Activate extension: register commands, attach handlers
 * @param {vscode.ExtensionContext} context
 */
export async function activate(context: vscode.ExtensionContext) {

	context.subscriptions.push(vscode.commands.registerCommand('move.compile', () => compileCommand().catch(console.error)));
	const extensionPath = context.extensionPath;
	const compilationOutputChannel = vscode.window.createOutputChannel('Move Compilation Log');

	/**
	 * Try to load local config. If non existent - use VSCode settings for this
	 * extension.
	 * @param workspaceFolder current workspace folder
	 * @returns extension config
	 */
	function loadConfig(workspaceFolder?: vscode.WorkspaceFolder): ExtensionConfig {
		const moveConfig = vscode.workspace.getConfiguration('move', workspaceFolder);
		const workDir = workspaceFolder;
		const folder = (workDir && workDir.uri.fsPath) || extensionPath;

		const cfg = {
			sender: moveConfig.get<string>('sender') || undefined,
			targetDir: moveConfig.get<string>('targetDir') || 'target',
			modulesPath: moveConfig.get<string>('modulesPath') || 'modules',
			stdlibPath: moveConfig.get<string>('stdlibPath') || undefined,
			languageServerPath: moveConfig.get<string>('languageServerPath') || undefined
		};

		if (cfg.stdlibPath && !path.isAbsolute(cfg.stdlibPath)) {
			cfg.stdlibPath = path.join(folder, cfg.stdlibPath);
		}

		if (cfg.modulesPath && !path.isAbsolute(cfg.modulesPath)) {
			cfg.modulesPath = path.join(folder, cfg.modulesPath);
		}

		return cfg;
		// return {
		// 	sender: cfg.sender,
		// 	targetDir: cfg.targetDir,
		// 	// @ts-ignore
		// 	modulesPath: cfg.modulesPath,
		// 	// @ts-ignore
		// 	stdlibPath: cfg.stdlibPath
		// };
	}

	/**
	 * Command: Move: Compile file
	 * Logic:
	 * - get active editor document, check if it's move
	 * - check network
	 * - run compillation
	 */
	async function compileCommand(): Promise<any> {

		// @ts-ignore
		const document = vscode.window.activeTextEditor.document;

		if (!checkDocumentLanguage(document, 'move')) {
			return vscode.window.showWarningMessage('Only .move files are supported by compiler');
		}

		const config = loadConfig(vscode.workspace.getWorkspaceFolder(document.uri));
		let sender = config.sender || null;

		// check if account has been preset
		if (!sender) {
			const prompt = 'Enter account from which you\'re going to deploy this script (or set it in config)';
			const placeHolder = '0x...';

			await vscode.window
				.showInputBox({ prompt, placeHolder })
				.then((value) => (value) && (sender = value));
		}

		const workdir = vscode.workspace.getWorkspaceFolder(document.uri) || { uri: { fsPath: '' } };
		const outdir = path.join(workdir.uri.fsPath, config.targetDir);



		checkCreateOutDir(outdir);

		if (!sender) {
			return vscode.window.showErrorMessage('sender is not specified');
		}
		return compileUsingLSP(sender, document, outdir);
	}

	async function compileUsingLSP(sender: string, document: vscode.TextDocument, outdir: string) {
		let workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
		if (!workspaceFolder) {
			return;
		}

		let client = workspaceClients.get(workspaceFolder);
		if (!client) {
			vscode.window.showWarningMessage(`no move language server running for file ${document.uri.fsPath}`);
			return;
		}
		let params: lsp.ExecuteCommandParams = {
			command: "compile",
			arguments: [sender, {
				file: document.uri.toString(),
				out_dir: outdir,
			}],
			workDoneToken: 1000,
		};


		compilationOutputChannel.show(true);

		let response = await client.sendRequest(ExecuteCommandRequest.type, params);

		if (!!response) {
			compilationOutputChannel.append(`${response}`);
		} else {
			compilationOutputChannel.appendLine("Compile Successful");
		}
	}

	function didOpenTextDocument(document: vscode.TextDocument) {

		if (!checkDocumentLanguage(document, 'move')) {
			return;
		}

		const folder = vscode.workspace.getWorkspaceFolder(document.uri);
		if (folder === undefined || workspaceClients.has(folder)) {
			console.log('LANGUAGE SERVER ALREADY STARTED');
			return;
		}

		const extensionConfig = loadConfig(folder);
		const executable = (process.platform === 'win32') ? 'move-ls.exe' : 'move-ls';
		let binaryPath = extensionConfig.languageServerPath || path.join(extensionPath, 'bin', executable);

		const lspExecutable: lsp.Executable = {
			command: binaryPath,
			options: { env: { RUST_LOG: 'move_language_server=info' } },
		};

		const serverOptions: lsp.ServerOptions = {
			run: lspExecutable,
			debug: lspExecutable,
		};

		const clientOptions: lsp.LanguageClientOptions = {
			// synchronize: {
			// 	configurationSection: 'move'
			// },
			outputChannel: vscode.window.createOutputChannel('Move Language Server'),
			traceOutputChannel: vscode.window.createOutputChannel("Move Language Server Trace"),
			workspaceFolder: folder,
			documentSelector: [{ scheme: 'file', language: 'move', pattern: folder.uri.fsPath + '/**/*' }],
			initializationOptions: toLanguageServerConfig(extensionConfig)
		};

		const client = new lsp.LanguageClient('move', 'Move Language Server', serverOptions, clientOptions);

		context.subscriptions.push(client.start());

		// TODO: should expose other configurations?
		client.onReady().then(() => client.onRequest('workspace/configuration', (params: lsp.ConfigurationParams) => {
			return params.items.map(_item => {
				let appConfig = loadConfig(folder);
				return toLanguageServerConfig(appConfig);
			});
		}));

		workspaceClients.set(folder, client);
	}

	vscode.workspace.onDidChangeConfiguration(evt => {
		for (let [folder, client] of workspaceClients) {
			if (evt.affectsConfiguration("move", folder)) {
				const moveConfig = vscode.workspace.getConfiguration('move', folder);
				client.sendNotification('workspace/didChangeConfiguration', { settings: moveConfig });
			}
		}
	});
	vscode.workspace.onDidOpenTextDocument(didOpenTextDocument);
	vscode.workspace.textDocuments.forEach(didOpenTextDocument);
	vscode.workspace.onDidChangeWorkspaceFolders((event) => {
		for (const folder of event.removed) {
			const client = workspaceClients.get(folder);
			if (client) {
				workspaceClients.delete(folder);
				client.stop();
			}
		}
	});

}

// this method is called when your extension is deactivated
export function deactivate() {
	return Array.from(workspaceClients.entries())
		.map(([, client]) => client.stop())
		.reduce((chain, prom) => chain.then(() => prom), Promise.resolve());
}


function toLanguageServerConfig(cfg: ExtensionConfig): LanguageServerConfig {
	const modules_folders = [];

	if (cfg.modulesPath) {
		modules_folders.push(cfg.modulesPath);
	}

	return {
		dialect: 'libra',
		modules_folders,
		stdlib_folder: cfg.stdlibPath,
		sender_address: cfg.sender,
	};
}

function checkDocumentLanguage(document: vscode.TextDocument, languageId: string) {
	if (document.languageId !== languageId || (document.uri.scheme !== 'file' && document.uri.scheme !== 'untitled')) {
		return false;
	}

	return true;
}

/**
 * Check whether compiler output directory exists: create if not, error when it's a
 *
 * @param   {String}  outDir  Output directory as set in config
 * @throws  {Error} 		  Throw Error when ourDir path exists and is not directory
 */
function checkCreateOutDir(outDir: string): void {
	const outDirPath = path.resolve(outDir);

	if (fs.existsSync(outDirPath)) {
		if (!fs.statSync(outDirPath).isDirectory()) {
			throw new Error('Can\'t create dir under move.targetDir path - file exists');
		}
	} else {
		fs.mkdirSync(outDirPath);
	}
}

