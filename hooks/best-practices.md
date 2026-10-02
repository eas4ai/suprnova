# Best-practices completion gate

Installed from `eas4ai/best-practices-agent-hook`, version 1.1.0,
commit `dd9861025fd9bd412a83964990584ff2cfa24214`.
The hook, ruleset, skills, and hook configurations are unmodified upstream copies.
The upstream MIT notice is in `best-practices.LICENSE`.

Codex loads `.codex/hooks.json`; Claude Code loads `.claude/settings.json`.
This repository-scoped gate complements the Sudus plugin; it does not replace its
hooks or authorize changes to the Sudus contract. The protected `AGENTS.md` is
unchanged; the global production-standard instructions and repository skill select
the local `BEST_PRACTICES.md`.

The gate prompts once when a visible todo list is complete. It does not independently
prove production quality. No automatic verification command is configured here.

## Installation verification

The upstream suite passed 34 tests. Ten checks through this repository's installed
Codex hook command passed: missing/incomplete todos, completed todos, repeat stops,
and failing/recovered verification. Installed upstream files were compared byte for
byte with the pinned source. The tests live upstream, not in this repository.

Ripwire's full-repository quality check exited 2. Its blocking duplication findings
compare Python `normalize_status` with Rust `escape_attribute`, and compare a
nested `walk` function with its containing `extract_todo_lists` function. Inspection
found different cross-language operations in the first pair and one source body
counted twice in the second. The imported parser also has advisory complexity flags.
No framework Rust code or upstream hook code was changed to silence these findings;
this records the non-clean analyzer result rather than claiming it passed.
