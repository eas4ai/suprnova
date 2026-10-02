# CLAUDE.md

## Repository production standard

Before planning, coding, reviewing, or completing implementation work, read and apply `BEST_PRACTICES.md`. It is the authoritative production coding ruleset for this repository.

## Required workflow

1. Use a todo list for multi-step work and keep exactly one item in progress.
2. Do not mark a todo complete until implementation and relevant verification are complete.
3. Use the project skill at `.claude/skills/best-practices/SKILL.md` whenever writing, modifying, reviewing, or completing code.
4. Before final delivery, answer rule 13 in `BEST_PRACTICES.md` (the release gate) honestly and revise until the answer is satisfactory.
5. Write every response, commit message, comment, and doc in simple technical English (rule 14 in `BEST_PRACTICES.md`).

## Completion hook

This repository includes a hook at `hooks/todo_complete_gate.py` and project hook configuration in `.claude/settings.json`. When the current todo list appears complete, the hook asks Claude to run one final production-quality pass against `BEST_PRACTICES.md` before stopping.
