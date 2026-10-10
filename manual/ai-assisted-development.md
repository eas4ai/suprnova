# AI-Assisted Development

A coding assistant such as Claude Code, Codex, Cursor or GitHub Copilot
writes better Suprnova code when it reads the Suprnova manual instead of
guessing from Laravel or from older Rust web frameworks. This chapter
shows you how to give an assistant the manual as plain text, how to
install the Suprnova language server, and how to point an assistant at
your project.

## Read the manual as Markdown

[suprnova.app](https://suprnova.app) serves every manual page twice: as
HTML for you, and as its Markdown source for an assistant. Add `.md` to
a page's address to get the source, with no page layout and no scripts:

```bash
curl https://suprnova.app/docs/routing.md
```

The HTML page names its Markdown form in a
`<link rel="alternate" type="text/markdown">` tag, so an assistant that
fetches the HTML can find it.

Two more files follow the [llms.txt](https://llmstxt.org) convention:

- `https://suprnova.app/llms.txt` is an index: a short summary of
  Suprnova, then a link to the Markdown form of every chapter, grouped as
  the manual's table of contents groups them.
- `https://suprnova.app/llms-full.txt` is the whole manual in one
  document, every chapter inlined in table-of-contents order. It is large
  (about 2 MB), so give it to an assistant that can hold it, or let the
  assistant fetch the chapters it needs from `llms.txt`.

Tell your assistant where they are. For example, in a conversation:

```text
Read https://suprnova.app/llms.txt, then fetch the chapters you need as
Markdown before you change routing or models in this project.
```

The manual is also in the framework's repository as plain Markdown, under
`manual/`. An assistant that has the repository checked out can read the
files directly.

## Install the language server

The [Suprnova LSP](https://github.com/eas4ai/suprnova-lsp) is a language
server for Suprnova applications. It reads the API that `#[model]`
generates from rustdoc JSON, so hover, completion and type inference work
on generated methods such as `Post::query()`, and it finds your
application's models on its own. It runs without rust-analyzer. An
assistant that drives your editor, such as Copilot or Cursor, sees the
same completions you do.

To install it in VS Code:

1. Open the [releases page](https://github.com/eas4ai/suprnova-lsp/releases)
   and download the `.vsix` file for your platform, such as
   `suprnova-lsp-0.3.0-linux-x64.vsix`. Each one bundles its server.
2. In VS Code, run the **Extensions: Install from VSIX...** command and
   choose the file, or run
   `code --install-extension suprnova-lsp-0.3.0-linux-x64.vsix`.
3. Reload VS Code.

The extension ID is `eas4ai.suprnova-lsp`, and its settings start with
`suprnova-lsp.`. It exports the model API again when your code changes,
which needs the nightly Rust toolchain its settings name. The releases
are pre-releases. Each release also carries the server alone, as a
`.tar.gz` file for each platform, for editors other than VS Code.

## Point an assistant at your project

An assistant works best when it knows how your project is built and how
you check a change. Most assistants read an instruction file at the root
of the project: `AGENTS.md` for Codex and several others, `CLAUDE.md` for
Claude Code. Suprnova does not generate one; write it yourself and keep it
short. For an application made with `suprnova new`, it can say:

```markdown
# Working on this project

This is a Suprnova application: a Laravel-shaped Rust web framework.
Read https://suprnova.app/llms.txt and fetch the chapters you need as
Markdown before you change a subsystem. Do not assume Laravel or Axum APIs.

- Models are `#[suprnova::model]` structs in `src/models/`; migrations are
  hand-written SeaORM migrations in `src/migrations/`.
- Routes are in `src/routes.rs`; handlers use `#[handler]`.
- After changing an `InertiaProps` struct, run `suprnova generate-types`.
- Check a change with `cargo check`, then `cargo test`.
- Create files with `suprnova make:controller`, `make:migration` and the
  other `make:*` commands rather than by hand.
```

Name the commands your project uses. The [CLI reference](cli.md) lists
every `suprnova` command, and [Testing](testing.md) shows how the tests of
an application run.

### Why Suprnova diverges

Laravel's AI chapter is built around **Laravel Boost**, an MCP server
installed with Composer that gives an assistant tools to inspect a running
Laravel application, a documentation search API, and generated guideline
files. Boost is Laravel tooling, for PHP applications, and Suprnova does
not ship a port of it. In its place, Suprnova offers the manual as
Markdown at `/docs/<chapter>.md`, the `llms.txt` and `llms-full.txt`
files, and the Suprnova LSP for editor-level knowledge of your models.
The instruction file that Boost generates is one you write yourself.

## Next

- [Development](development.md) - the local workflow an assistant runs
  commands in
- [CLI reference](cli.md) - every `suprnova` command, for your
  instruction file
- [Testing](testing.md) - how to check a change before you accept it
