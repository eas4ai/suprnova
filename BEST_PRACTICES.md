# BEST_PRACTICES.md

Ruleset version 1.1.0 (2026-09-01).

This ruleset is the minimum bar for production coding work by any agent in this repository. The rules are mandatory unless the user or maintainers explicitly override them.

## Production coding rules

### Before you change anything

1. **Understand before you edit.** Know the requested outcome — restate it for non-trivial work — and map the affected files, runtime paths, data flows, and user-visible behavior. Find the existing patterns, conventions, flags, contracts, and tests first, and preserve current behavior unless the task explicitly changes it.

### As you build

2. **Make the smallest coherent change.** Solve the real problem with the least durable change. No speculative rewrites, needless dependencies, unrelated cleanup, or cosmetic churn.

3. **Write for the next maintainer.** Keep code cohesive and named for intent, with logic in the right layer. Avoid hidden coupling, global state, copy-paste, and cleverness that is hard to reason about.

4. **Honor contracts at boundaries.** Validate inputs where trust changes. Keep public interfaces backward-compatible unless a break is explicitly requested, and change types, schemas, callers, mocks, and docs together.

5. **Handle errors and secrets deliberately.** Fail safely with actionable, context-preserving errors; never swallow exceptions. Never expose secrets or sensitive data in logs, messages, traces, or fixtures.

6. **Treat security as part of the task.** Authn/authz, injection, deserialization, path traversal, secrets, dependency provenance, and data exposure are requirements, not afterthoughts.

7. **Make state changes survivable.** For persistence, migrations, queues, caches, and jobs, account for retries, idempotency, partial failure, rollback, concurrency, and old/new version coexistence.

8. **Guard performance and reliability.** Avoid N+1 queries, blocking work on hot paths, unbounded memory or retries, races, resource leaks, and missing timeouts.

### Process and delivery

9. **Track multi-step work.** Keep a todo list with exactly one item in progress; never mark an item done until its code and its verification are both complete.

10. **Test and verify what matters.** Cover the success path, key edge cases, and failure modes at the level the risk warrants — not snapshots alone. Run the repository's documented checks (targeted tests, lint, type, build); if one cannot run, say so and state the risk. Keep docs, examples, and generated artifacts in sync with the behavior you changed.

11. **Report honestly; never fake completion.** State what changed, what verification actually ran, and what remains risky or unverified. Never claim a check passed unless it ran and passed, and never declare work done while known defects, failing checks, missing tests, or incomplete items remain.

12. **Work as a technical partner.** Use the user's experience as a resource: recommend a concrete path when you have one, and ask for guidance when their judgment would materially improve the result.

13. **Self-audit before delivery (the release gate).** You are delivering production software. Do not deliver work you know to be deficient — incomplete, unverified, internally inconsistent, or in violation of this ruleset. Before delivering, review the implementation against every rule and answer honestly: are you satisfied it follows this ruleset, or would you make revisions? If revisions are needed, make them before you deliver. (This ruleset is itself subject to this rule.)

14. **Use simple technical English.** In interactions, commit messages, comments, and documentation, write to be understood on first read: short sentences, one idea per sentence, active voice, concrete verbs, and common words over rare ones ("use" over "utilize" or "leverage"). Use the project's own vocabulary, and define any term of art at first use. Cut filler, hype, hedging, and buzzwords; text is done when nothing more can be removed without losing meaning. (This is a plain-writing rule, not a controlled vocabulary; no restricted dictionary applies.)
