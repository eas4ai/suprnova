# Adversary brief: schema-dump

## Your role
Your task is READ-ONLY in the repo. You may use a scratchpad to record your finding but will clean up after yourself once you have created a report. Your report is your only output: deliver it to the builder and stop.

Rules:
- Do not build, run tests or run any project code. The receipts below say what already ran.
- Do not start subagents. Do the whole review inline, yourself.
- Keep your scratchpad and your report file outside the repository. A file written inside it changes the workspace, and sudus report refuses a report once the workspace differs from the reviewed snapshot.
- Read the whole specification under docs/spec, with its glossary and roadmap, and docs/decisions.jsonl before you judge anything. Judge this commitment as part of the whole system, not as a feature alone.
- Treat every claim in the builder's account as something to disprove, not as settled.
- Do not read the paths listed under Paths not to read.

You have no stake in Done. The builder wants this commitment finished; you do not inherit that goal. Your role is very important as it is to prove that the implementation for this commitment is not production ready. The builder will record your report and it will be committed to the repo as evidence. You are accountable for the contents of your report to the builder. Everything you report will be validated. Ensure that you are satisfied with the report before you give it to the builder. The user will use these reports to score the builders performance as well as yours. Accuracy and candidness is imperative.

If your report includes a bug report about Sudus, that you have discovered - stop the review, provide the bug report to the builder and stop. Sudus will likely require remediation before continuing the review.

Scrutinize the commitment against the requirement, not against the builder's account of it. Put the frozen text of each requirement beside what was built, and judge the work through five lenses:

1. The falsifier. Is the chosen failure the right observable failure for this obligation, or a weaker neighbour? Does the mechanism detect it, and could the code satisfy the check without meeting the obligation? Is there an input that reaches the falsifier which the check never exercises?
2. The decisions. Every one the builder made, recorded or not: the defaults, the assumptions, the convenient reading of an ambiguous line. Were the premises true? Was the option not taken closer to the text? Did any decision belong to the developer?
3. Security. The trust boundaries the change touched: input crossing them, paths and symlinks, secrets in code, records or output, new dependencies, new egress, new defaults.
4. Logic. Every new branch, catch, default and early return, and the input that takes the other side of it. Ordering, idempotency, partial failure.
5. Complexity and spec adherence. Behavior no requirement names. Behavior a requirement names that holds only on the happy path. Changed paths no mechanism covers. Documentation the code no longer matches. The same falsifier unreachable with less.

If you know there is a likely issue, start there.

A finding is a specific claim against the work: where (path and line, record, or requirement id), what is wrong, and why, quoting the requirement text or the decision it contradicts. Mark it Critical (a requirement is unmet, the falsifier is reachable, or a security exposure ships), Major (an uncovered defect in a touched path, or a decision that was the developer's) or Minor (an edge the commitment did not promise, or complexity the next commitment pays for). No fixes: a remedy is one line of advice, and optional. A note that is not a defect goes with the attempt that raised it, not in the numbered findings.

The attempts the brief requires are made by reading: for each pair, say what you looked for, what you found, and whether it held. Do not pad. A clean report earned by scrutiny is a good outcome. Developer rulings are settled; the implementation of them is not.

The user requires your best effort.

## Roadmap section
## schema-dump

Requirements: PAR-038, PAR-039, PAR-040

25. schema-dump - `schema:dump` writes the schema and the migration ledger
   with each engine's own tool, `migrate`, `migrate:fresh`, `serve` and
   `TestDatabase::fresh` load it into a database with no applied
   migration before newer migrations run, and `--prune` replaces the
   dumped migrations with `PrunedMigration` names
   (`laravel-parity.md` PAR-038 to PAR-040, next-feature item
   `schema-dump`). Done when the mechanism passes on a committed tree,
   against SQLite, Postgres, MySQL and MariaDB.

Out of every commitment above, by the developer's 2026-09-12 ruling:
complete blocks (auth flows, account settings, CTAs, pricing, dashboard
layouts), tag input, nested menus, command palette, drag-reorder,
auto-infinite-scroll, carousel, image cropper, rich text editor, rating
input. They are a separate project and enter this roadmap only if the
developer writes them in.

Every commitment inherits the Live gate and the repository gate; a
commitment is Done only when its mechanisms pass on a committed tree and
its review is recorded (`sudus review`).

## Frozen requirements and falsifiers
[PAR-038] `schema:dump` on the app binary MUST write the schema of the `DATABASE_URL` database to `database/schema/<engine>-schema.sql`, where the engine is `sqlite`, `postgres`, `mysql` or `mariadb`, or to the file `--path` names: the statements that create every table, index, view and constraint the database holds, without any table's rows, followed by one `INSERT` for each row of the Migrator's ledger table (`seaql_migrations` unless the Migrator names another). Postgres MUST be dumped with `pg_dump`, MySQL with `mysqldump` and MariaDB with `mariadb-dump`, each given the password through its environment or a file only the current user can read, never as an argument; SQLite MUST be read through its own connection. When the tool is missing or fails, the command MUST exit with an error that names the tool, and an earlier dump file MUST stay as it was. The developer CLI's `suprnova schema:dump` MUST run the app binary's.
Falsifier: on any of SQLite, Postgres, MySQL and MariaDB, after `migrate` the dump file is absent, holds a row of a table other than the ledger, lacks a table or index the migrations created, or lacks an applied migration's ledger row; the password appears in the dump tool's arguments; or with the tool missing the command exits zero or changes an earlier dump file.

[PAR-039] When the migration ledger records no migration and a dump file exists for the engine, or `--schema-path` names one, `migrate`, `migrate:fresh` and the migration `serve` runs MUST load the file into the database before any migration runs, and then run only the migrations the loaded ledger does not record. `TestDatabase::fresh` MUST load `database/schema/sqlite-schema.sql` the same way. Postgres MUST be loaded with `psql`, MySQL with `mysql`, MariaDB with `mariadb`, and SQLite through its own connection. A failed load MUST stop the command with an error before any migration runs. A database whose ledger records a migration MUST NOT be loaded.
Falsifier: on any of SQLite, Postgres, MySQL and MariaDB, an empty database migrated with a dump of the first migrations and one newer migration lacks a table from the dump, runs one of the dumped migrations again, or lacks the newer migration's table; a database with an applied migration is loaded; or a failing load lets a migration run.

[PAR-040] `schema:dump --prune`, on the app binary and through the developer CLI, MUST, after the dump succeeds, delete the file of each migration in `src/migrations/` whose name the dump's ledger records, remove its `mod` line from `mod.rs`, and replace its entry in the Migrator's list with `PrunedMigration::new("<name>")`, which keeps the name without the code. A migration the ledger does not record MUST stay. Without `--prune`, and when the dump fails, no file changes. A `PrunedMigration` that has to run, up or down, MUST fail with an error that names it and `database/schema`, so a database never skips a pruned migration's schema; a database whose ledger records it migrates as before.
Falsifier: after `--prune`, an applied migration's file remains, a pending migration's file is gone, or `mod.rs` still declares a deleted module; `migrate` fails against a database that applied the pruned migrations; `migrate` succeeds on an empty database with no dump file; or a failed dump pruned a file.

## Mechanism definitions
par-schema-dump: {"command":"RUST_MIN_STACK=33554432 CARGO_INCREMENTAL=0 scripts/check-schema-dump.sh","cwd":null,"documents":["CHANGELOG.md","crates/suprnova-live/docs","feature-map/exclusions.json","feature-map/laravel/parity.jsonl","feature-map/manual-triage-verdicts.json","feature-map/manual-triage.jsonl","feature-map/meta.json","feature-map/surface.jsonl","feature-map/tools","manual"],"identity":{"env":[],"image":null,"tools":{}},"inputs":["CHANGELOG.md","Cargo.lock","Cargo.toml","app","crates/suprnova-live","crates/suprnova-live/docs","feature-map/exclusions.json","feature-map/laravel/parity.jsonl","feature-map/manual-triage-verdicts.json","feature-map/manual-triage.jsonl","feature-map/meta.json","feature-map/surface.jsonl","feature-map/tools","framework","manual","suprnova-cli","suprnova-macros"],"requirements":["PAR-038","PAR-039","PAR-040"],"results":null}

## Receipts (what already ran; do not run it again)
PAR-038: current receipt 99a8880d90bc40265fa232a63d8e3ff1e3dbf43e from mechanism par-schema-dump: pass
PAR-038: fail receipt 126673d3ee85931b2de9020cb226634d58ccf1bf from mechanism par-schema-dump, the violating example its review binds
PAR-039: current receipt 99a8880d90bc40265fa232a63d8e3ff1e3dbf43e from mechanism par-schema-dump: pass
PAR-039: fail receipt 126673d3ee85931b2de9020cb226634d58ccf1bf from mechanism par-schema-dump, the violating example its review binds
PAR-040: current receipt 99a8880d90bc40265fa232a63d8e3ff1e3dbf43e from mechanism par-schema-dump: pass
PAR-040: fail receipt 126673d3ee85931b2de9020cb226634d58ccf1bf from mechanism par-schema-dump, the violating example its review binds

## Builder claims
mechanism par-schema-dump Q1 observed: With framework/tests/schema_dump and suprnova-cli/tests/schema_dump_cli.rs committed before the implementation (5e0040c9), sudus check recorded fail receipt 126673d3 for PAR-038 to PAR-040: the schema_dump test binary did not compile because suprnova::SchemaDump and suprnova::PrunedMigration do not exist; the throwaway databases on the standing Postgres, MariaDB and MySQL 8.4 were created first, so the script reached the build. Bound to all three.
mechanism par-schema-dump Q2 observed: The only errors were the missing API. The same command passes on the committed tree (receipt 99a8880d): 13 of 13 framework tests, among them postgres_dumps_loads_and_prunes, mysql_dumps_loads_and_prunes and mariadb_dumps_loads_and_prunes run with --run-ignored all, and 2 of 2 CLI tests. The two schema_dump unit tests pass.
requirement PAR-038 Q3 observed: cases::dump_holds_the_schema_and_the_ledger on all four engines: tables, the index, no row of sd_users, exactly two INSERTs both into seaql_migrations with each applied version, and default_path is database/schema/<engine>-schema.sql; engines.rs wraps pg_dump, mysqldump and mariadb-dump in a script that logs its arguments and asserts the URL's password is not among them; sqlite::a_missing_dump_tool_keeps_the_earlier_dump empties PATH and asserts the error names pg_dump, not the password, and the earlier file is unchanged; the CLI test asserts suprnova schema:dump [--path, --prune] reaches the app binary's schema:dump. Pass receipt 99a8880d.
requirement PAR-038 Q4 observed: framework/src/database/schema_dump.rs: SchemaDump::dump looks the tool up on PATH before connecting for a scheme that names the engine, asks VERSION() for a mysql:// URL, runs pg_dump --schema-only --no-owner --no-acl with PGPASSWORD and PGSSLMODE, or mysqldump/mariadb-dump --no-data --routines with a 0600 --defaults-extra-file (and --set-gtid-purged=OFF for MySQL, retried without when the tool rejects it), strips AUTO_INCREMENT counters, reads SQLite's sqlite_master (tables, indexes, views, triggers, shadow tables left out), appends the Migrator's ledger rows as quoted INSERTs after a marker (schema-qualified on Postgres, whose dump empties the search path), and writes through a .partial file renamed into place. The app binary's schema:dump and the CLI's call it.
requirement PAR-039 Q3 observed: cases::the_dump_loads_before_newer_migrations (migrate and fresh), only_an_empty_ledger_loads (a migrated database ignores a dump that adds a table; a broken dump fails before any migration runs) on all four engines; sqlite::migrate_finds_the_dump_at_its_default_path; sqlite::test_database_fresh_loads_the_sqlite_dump; the CLI test asserts migrate --schema-path reaches the app binary. The dumped migrations create their tables without IF NOT EXISTS, so a rerun would fail. Pass receipt 99a8880d.
requirement PAR-039 Q4 observed: load_when_empty: the ledger table missing or holding no row, the --schema-path file or the engine's default dump present: the empty ledger table is dropped, then the file is loaded with psql --single-transaction -v ON_ERROR_STOP=1, mysql/mariadb reading it on stdin with the option file, or SQLite's connection in one transaction; then M::up. SchemaDump::fresh drops every table through an empty Migrator that shares M's ledger name first. The app binary's migrate, migrate:fresh and serve auto-migration call SchemaDump::migrate/fresh; TestDatabase::fresh loads database/schema/sqlite-schema.sql before M::up.
requirement PAR-040 Q3 observed: prune::prune_replaces_the_dumped_migrations_and_keeps_the_rest (files deleted, mod lines gone, PrunedMigration entries, the unrecorded migration kept), a_dump_with_no_ledger_prunes_nothing, a_failed_dump_prunes_nothing; cases::pruned_migrations_keep_their_names on all four engines: a database that applied them migrates on, an empty one loads the dump, rollback into a pruned migration fails naming it, and migrate with no dump fails naming the migration and database/schema. Pass receipt 99a8880d.
requirement PAR-040 Q4 observed: SchemaDump::prune reads the version of each ledger INSERT after the marker, keeps those that are identifiers with a file in src/migrations, removes their mod lines and swaps Box::new(name::Migration) for Box::new(suprnova::PrunedMigration::new("name")), refusing and changing nothing when a module is not declared and listed as make:migration writes it; mod.rs is written before the files are deleted. dump_and_prune prunes only after dump succeeds. PrunedMigration::up and down return DbErr::Custom naming the migration and database/schema.
commitment schema-dump Q5 observed: Test edits after the red commit: cargo fmt only. The mechanism runs a gitignored script, scripts/check-schema-dump.sh, which reads the gate passwords from the gate scripts and puts /usr/lib/postgresql/18/bin first on PATH because a 17.2 pg_dump from miniconda shadows the server's 18.6 client. MySQL 8.4 is dumped with the mysqldump on this machine, which is MariaDB 12.3's. schema_dump and both schema:dump commands needed chapter rules in feature-map/tools/build_surface.py, a declared mechanism input. Parity: 46 rows judged, the three events by_design_no by the confirmed spec.
commitment schema-dump Q6 observed: Not tested: Oracle's own mysqldump and mysql clients (only MariaDB's are installed here); a Postgres server newer than its pg_dump (pg_dump refuses); sslmode and Unix-socket URLs; databases with views, triggers, routines or Postgres types and extensions beyond the test's tables and index; the serve auto-migration path end to end (it calls the same SchemaDump::migrate); the doctests on SchemaDump and PrunedMigration and the rustdoc JSON link check, which run in the release gate; a Windows build (the option file falls back to a plain write).

## Builder findings

## Agent decisions (each is a decision pair below)
none recorded

## Interface obligations

## Changed paths (start snapshot to reviewed snapshot; A added, M modified, D deleted)
M CHANGELOG.md
M feature-map/exclusions.json
M feature-map/laravel/parity.jsonl
M feature-map/manual-triage.jsonl
M feature-map/meta.json
M feature-map/surface.jsonl
M feature-map/tools/build_surface.py
M framework/src/app/mod.rs
M framework/src/database/mod.rs
A framework/src/database/schema_dump.rs
M framework/src/database/testing.rs
M framework/src/lib.rs
M manual/cli-migrations.md
M manual/database-testing.md
M manual/migrations.md
M suprnova-cli/src/commands/migrate.rs
M suprnova-cli/src/commands/migrate_fresh.rs
M suprnova-cli/src/commands/mod.rs
A suprnova-cli/src/commands/schema_dump.rs
M suprnova-cli/src/main.rs

## Paths not to read
These hold credentials or are excluded from every model by the settings. Do not open them; any file matching one of the patterns is excluded too, tracked or not.
pattern **/.env
pattern **/.env.*
pattern **/*.pem
pattern **/*.p12
pattern **/*.pfx
pattern **/*.key
pattern **/id_rsa
pattern **/id_dsa
pattern **/id_ecdsa
pattern **/id_ed25519
pattern **/id_ecdsa_sk
pattern **/id_ed25519_sk
credential app/.env
credential suprnova-cli/src/templates/files/api/.env.tpl

## Report
Write your report as one JSON file outside the repository and give back its path; it goes to `sudus report` unchanged. It is an object with exactly these fields:
- "attempts": one {"question", "target", "looked_for", "found", "held"} for each pair below, spelled exactly as written. "looked_for" and "found" are text; "held" is true when the work held against what you looked for and false when it did not.
  falsifier PAR-038
  falsifier PAR-039
  falsifier PAR-040
  security schema-dump
  logic schema-dump
  complexity schema-dump
- "interface_attempts": one {"path", "looked_for", "found", "held"} for each path below, each path a JSON string to copy as it is (none: write [])
- "findings": [{"n": 1, "severity": "Critical", "where": "...", "text": "...", "remedy": null}, ...], numbered from 1 in order, or [] when you found nothing. "severity" is Critical, Major or Minor; "where" is the path and line, record or requirement id; "text" is what is wrong and why; "remedy" is one line of advice or null.
- "sudus_bug": null, or the bug report when you stopped on a Sudus bug; then "attempts", "interface_attempts" and "findings" are [].
