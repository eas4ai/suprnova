# Feature map: `manual/cli-generators.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 8 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova make:middleware` · suprnova-cli/src/main.rs:154
  - Generate a new middleware
  - argument `<NAME>`: Name of the middleware (e.g., Auth, RateLimit)
- [ ] command `suprnova make:controller` · suprnova-cli/src/main.rs:160
  - Generate a new controller
  - argument `<NAME>`: Name of the controller (e.g., users, user_profile)
- [ ] command `suprnova make:action` · suprnova-cli/src/main.rs:166
  - Generate a new action
  - argument `<NAME>`: Name of the action (e.g., AddTodo, CreateUser)
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
