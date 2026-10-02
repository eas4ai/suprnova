---
name: best-practices
description: Apply the repository production software best practices ruleset before planning, coding, reviewing, refactoring, debugging, testing, or marking implementation work complete.
---

# Best Practices Skill

Use this skill for every coding task, especially implementation, refactoring, debugging, code review, test creation, and completion-gate work.

## Instructions

1. Read `BEST_PRACTICES.md` from the repository root before making or approving implementation changes.
2. Convert the request into a short implementation plan when the task has multiple steps or meaningful risk.
3. Keep a todo list current. Mark one item in progress at a time. Do not mark an item complete until code and relevant verification for that item are complete.
4. Follow the repository's existing architecture, naming, dependency, testing, and error-handling patterns unless the requested change requires otherwise.
5. Run targeted verification after changes. Run broader lint, type, build, or test checks when the risk warrants it or the repository expects it.
6. Before final delivery, perform a self-review using every rule in `BEST_PRACTICES.md`.
7. Treat rule 13 in `BEST_PRACTICES.md` as the release gate. If the answer is that revisions are needed, make those revisions before delivering.
8. Write every response, commit message, comment, and doc in simple technical English (rule 14): short sentences, one idea each, common words, no filler or buzzwords.

## Expected final response

Report:

- What changed.
- What verification ran and passed.
- Any material limitation or command that could not be run, with the reason.

Never claim verification passed unless it actually ran and passed.
