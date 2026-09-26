# Suprnova feature map: command line

Source: binaries built from the repository at d03b4f1; every entry is read from the binary's own `--help` output, and each command's declaration is cited.

A checked box means the documentation for that item has been remediated against the source.

## Counts

- `suprnova`: 33 commands
- `app`: 15 commands
- `console`: 5 commands

## `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova new` · suprnova-cli/src/main.rs:33
  - Create a new Suprnova project
  - argument `[NAME]`: The name of the project to create
- [ ] command `suprnova serve` · suprnova-cli/src/main.rs:60
  - Start the development servers (backend + frontend)
  - option `-p, --port <PORT>`: Backend port. Overrides SERVER_PORT/.env and pins the port exactly (no free-port scan). Defaults to SERVER_PORT, else 8765, scanning upward if that port is busy
- [ ] command `suprnova dev:tls` · suprnova-cli/src/main.rs:115
  - Register an HTTPS dev URL (https://<name>.localhost) and trust portless's CA in your browsers' certificate stores
  - option `-p, --port <PORT>`: Backend port to route to. Defaults to SERVER_PORT, else 8765
- [ ] command `suprnova web:run` · suprnova-cli/src/main.rs:137
  - Run the web server (app runtime)
- [ ] command `suprnova generate-types` · suprnova-cli/src/main.rs:140
  - Generate TypeScript types from Rust InertiaProps structs
  - option `-o, --output <OUTPUT>`: Output file path (default: frontend/src/types/inertia-props.ts)
  - option `-w, --watch`: Watch for changes and regenerate
- [ ] command `suprnova make:middleware` · suprnova-cli/src/main.rs:154
  - Generate a new middleware
  - argument `<NAME>`: Name of the middleware (e.g., Auth, RateLimit)
- [ ] command `suprnova make:controller` · suprnova-cli/src/main.rs:160
  - Generate a new controller
  - argument `<NAME>`: Name of the controller (e.g., users, user_profile)
- [ ] command `suprnova make:action` · suprnova-cli/src/main.rs:166
  - Generate a new action
  - argument `<NAME>`: Name of the action (e.g., AddTodo, CreateUser)
- [ ] command `suprnova live:make` · suprnova-cli/src/main.rs:172
  - Scaffold a Live component with its view and registration
  - argument `<NAME>`: Name of the component (e.g., Counter, TodoList, todo-list)
- [ ] command `suprnova live:add` · suprnova-cli/src/main.rs:181
  - Install a Live component library component from its manifest
  - argument `[NAME]`: Shipped component to install (e.g., field, password-input)
- [ ] command `suprnova live:check` · suprnova-cli/src/main.rs:196
  - Check every registered Live view with the integrated checker
- [ ] command `suprnova live:inspect` · suprnova-cli/src/main.rs:210
  - Report safe Live runtime, registry, provider, and artifact state
- [ ] command `suprnova live:assets` · suprnova-cli/src/main.rs:220
  - Publish the reviewed Live runtime artifacts into a directory
- [ ] command `suprnova make:command` · suprnova-cli/src/main.rs:234
  - Generate a new console command
  - argument `<NAME>`: Name of the command (e.g., `clean-cache`, `mail:send`, `CleanCache`)
- [ ] command `suprnova make:error` · suprnova-cli/src/main.rs:240
  - Generate a new domain error
  - argument `<NAME>`: Name of the error (e.g., UserNotFound, InvalidInput)
- [ ] command `suprnova make:inertia` · suprnova-cli/src/main.rs:246
  - Generate a new Inertia page or Data struct
  - argument `<NAME>`: Name of the page or struct (e.g., About, UserProps)
- [ ] command `suprnova make:migration` · suprnova-cli/src/main.rs:255
  - Generate a new database migration
  - argument `<NAME>`: Name of the migration (e.g., create_users_table, add_email_to_users)
- [ ] command `suprnova make:task` · suprnova-cli/src/main.rs:261
  - Generate a new scheduled task
  - argument `<NAME>`: Name of the task (e.g., CleanupLogs, SendReminders)
- [ ] command `suprnova migrate` · suprnova-cli/src/main.rs:267
  - Run all pending database migrations
- [ ] command `suprnova migrate:rollback` · suprnova-cli/src/main.rs:269
  - Rollback the last database migration(s)
- [ ] command `suprnova migrate:status` · suprnova-cli/src/main.rs:276
  - Show the status of all migrations
- [ ] command `suprnova migrate:fresh` · suprnova-cli/src/main.rs:279
  - Drop all tables and re-run all migrations
- [ ] command `suprnova db:sync` · suprnova-cli/src/main.rs:288
  - Sync database schema to entity files (runs migrations + generates entities)
- [ ] command `suprnova docker:init` · suprnova-cli/src/main.rs:298
  - Generate a production-ready Dockerfile
- [ ] command `suprnova docker:compose` · suprnova-cli/src/main.rs:301
  - Generate docker-compose.yml for local development
- [ ] command `suprnova schedule:run` · suprnova-cli/src/main.rs:311
  - Run all due scheduled tasks once (typically called by cron every minute)
- [ ] command `suprnova schedule:work` · suprnova-cli/src/main.rs:314
  - Start the scheduler daemon (runs continuously, checks every minute)
- [ ] command `suprnova schedule:list` · suprnova-cli/src/main.rs:317
  - List all registered scheduled tasks
- [ ] command `suprnova workflow:work` · suprnova-cli/src/main.rs:320
  - Start the workflow worker daemon
- [ ] command `suprnova workflow:install` · suprnova-cli/src/main.rs:323
  - Install workflow migrations
- [ ] command `suprnova ssr:start` · suprnova-cli/src/main.rs:326
  - Launch the Inertia SSR worker in the foreground
- [ ] command `suprnova ssr:check` · suprnova-cli/src/main.rs:338
  - Verify the Inertia SSR worker is reachable
- [ ] command `suprnova key:generate` · suprnova-cli/src/main.rs:349
  - Generate a new APP_KEY (32-byte AES-256, base64 URL-safe, no padding)

## App runner (`suprnova::Application`; the project binary)

- [ ] command `app serve` · framework/src/app/mod.rs:118
  - Run the web server (default command)
- [ ] command `app web:run` · framework/src/app/mod.rs:124
  - Run the web server (alias for serve)
- [ ] command `app migrate` · framework/src/app/mod.rs:131
  - Run pending database migrations
- [ ] command `app migrate:status` · framework/src/app/mod.rs:133
  - Show migration status
- [ ] command `app migrate:rollback` · framework/src/app/mod.rs:136
  - Rollback the last migration(s)
  - argument `[STEPS]`: Number of migrations to rollback [default: 1]
- [ ] command `app migrate:fresh` · framework/src/app/mod.rs:143
  - Drop all tables and re-run all migrations
- [ ] command `app schedule:work` · framework/src/app/mod.rs:150
  - Run the scheduler daemon (checks every minute)
- [ ] command `app schedule:run` · framework/src/app/mod.rs:153
  - Run all due scheduled tasks once
- [ ] command `app schedule:list` · framework/src/app/mod.rs:156
  - List all registered scheduled tasks
- [ ] command `app workflow:work` · framework/src/app/mod.rs:163
  - Run the workflow worker daemon
- [ ] command `app queue:work` · framework/src/app/mod.rs:166
  - Run the queue worker daemon (drains the configured queue driver)
- [ ] command `app queue:pause` · framework/src/app/mod.rs:186
  - Pause job processing for a queue (or every queue with `--all`). Mirrors `php artisan queue:pause`
  - argument `[QUEUE]`: Queue to pause. Required unless `--all` is given
- [ ] command `app queue:resume` · framework/src/app/mod.rs:197
  - Resume job processing for a paused queue (or every queue with `--all`). Mirrors `php artisan queue:resume` (alias `queue:continue`)
  - argument `[QUEUE]`: Queue to resume. Required unless `--all` is given
- [ ] command `app down` · framework/src/app/mod.rs:207
  - Put the application into maintenance mode
- [ ] command `app up` · framework/src/app/mod.rs:234
  - Bring the application out of maintenance mode

## Console binary (framework-registered commands)

- [ ] command `console db:seed` · framework/src/console/builtins/db_seed.rs:36
  - Run seeders (all by default, or one via --class=<Name>)
  - argument `[__suprnova_trailing_args]...`
- [ ] command `console model:prune` · framework/src/eloquent/console/prune.rs:25
  - Prune stale rows for every Prunable / MassPrunable model.
- [ ] command `console __suprnova:live-tool` · framework/src/live/tooling.rs:639 (hidden from `help`; dispatchable)
  - Answers the suprnova CLI's Live tooling protocol (not for interactive use)
- [ ] command `console render-cache:epoch-advance` · framework/src/render_cache/console.rs:98 (hidden from `help`; dispatchable)
  - Advances the RenderCache authority epoch, making every stored entry unreachable at its next freshness check (emergency invalidation)
- [ ] command `console render-cache:inspect` · framework/src/render_cache/console.rs:168 (hidden from `help`; dispatchable)
  - Body-free inspection of one RenderCache entry by its rk1. key
  - argument `<key>`: The entry's rk1. lookup key, as printed by application logging

Excluded as demo-app commands (`app/src/commands`): `bench:enqueue-abort`, `bench:enqueue-records`, `bench:enqueue-sleep`, `bench:password-hash`, `bench:verify-records`, `bench:verify-ticks`, `greet`
