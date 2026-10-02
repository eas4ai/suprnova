---
name: best-practices
description: Apply the repository production software best practices ruleset before planning, coding, reviewing, refactoring, debugging, testing, or marking implementation work complete.
---

# Best Practices Skill

Use this skill for every coding task, especially implementation, refactoring, debugging, code review, test creation, and completion-gate work.

## Current ruleset

!`if [ -n "$CLAUDE_PROJECT_DIR" ] && [ -f "$CLAUDE_PROJECT_DIR/BEST_PRACTICES.md" ]; then cat "$CLAUDE_PROJECT_DIR/BEST_PRACTICES.md"; elif [ -f BEST_PRACTICES.md ]; then cat BEST_PRACTICES.md; elif [ -f "$HOME/.claude/BEST_PRACTICES.md" ]; then cat "$HOME/.claude/BEST_PRACTICES.md"; else echo "BEST_PRACTICES.md was not found. Ask the user or inspect the repository before proceeding."; fi`

The ruleset above was loaded from the nearest available source: this repository's own `BEST_PRACTICES.md` if it ships one, otherwise the global `~/.claude/BEST_PRACTICES.md`. A repository copy always wins, so a project may tighten or tailor the rules.

## Instructions

1. Apply the loaded ruleset before making or approving implementation changes.
2. Convert the request into a short implementation plan when the task has multiple steps or meaningful risk.
3. Keep a todo list current. Mark one item in progress at a time. Do not mark an item complete until code and relevant verification for that item are complete.
4. Follow the repository's existing architecture, naming, dependency, testing, and error-handling patterns unless the requested change requires otherwise.
5. Run targeted verification after changes. Run broader lint, type, build, or test checks when the risk warrants it or the repository expects it.
6. Before final delivery, perform a self-review using every rule in `BEST_PRACTICES.md`.
7. Treat rule 13 in `BEST_PRACTICES.md` as the release gate. If the answer is that revisions are needed, make those revisions before delivering.
8. Write every response, commit message, comment, and doc in simple technical English (rule 14): short sentences, one idea each, common words, no filler or buzzwords.

## Expected final response

Report what changed, what verification ran and passed, and any material limitation or command that could not be run with the reason. Never claim verification passed unless it actually ran and passed. Write the report in simple technical English.
