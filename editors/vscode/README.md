# Mover

Mover implements a language server protocol for Libra Move language.
It contains two parts:

- A move language server implemente in Rust.
- A vscode extenstion to develope move contract.

## Features

- Semantic Hightlighting for Move Lang.
- Instant diagnistics.
- integrated Move compiling command.


## Requirements


I recommand you to install **move-syntax-highlight** plugin.

## Extension Settings


This extension contributes the following settings:

* `move.targetDir`: Relative path inside working directory to put compiled files into. Default to: `target`.
* `move.sender`: Default account to use when compiling. Required.
* `move.languageServerPath`: Custom path to Move Language Server binary.
* `move.stdlibPath`: Path of stdlib.
* `move.modulesPath`: Path of modules. Default: `modules`.


## Known Issues

None.

## Release Notes

See [CHANGELOG](./CHANGELOG.md)

**Enjoy!**
