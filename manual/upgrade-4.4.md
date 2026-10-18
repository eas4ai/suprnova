# Upgrading to 4.4

This chapter moves an application from Suprnova 4.3.0 to 4.4.0. Each section
matches an entry in the 4.4.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.3 to 4.4.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**. On
MySQL and MariaDB, the first boot of 4.4 widens five of Magnetar's tables,
and on a large table that takes time. Read that entry before you deploy.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.4.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.4.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.4.0 suprnova-cli
   cargo check --all-targets
   ```

2. On MySQL or MariaDB, plan the first boot as the next section describes.
3. Run your tests.

## Fixed entries that change behaviour

### Magnetar's tables hold long values on MySQL and MariaDB.

**Before:** on MySQL and MariaDB, Magnetar created its byte columns as
`varbinary(255)`, and the session user agent, the passkey credential id and
public key and the provider scope columns as `varchar(255)`. A longer value
failed to save.

**Now:** new tables get `blob`, `longblob` and `text` columns. The first time
`migrate` runs against an existing database, it widens these columns in five
tables: `auth_sessions`, `auth_methods`, `auth_ceremonies`,
`auth_two_factor` and `auth_provider_tokens`. MySQL and MariaDB change a
column's type by copying the table, and they block writes to that table
while they copy. The framework runs `migrate` when the application boots,
unless you built `MagnetarConfig` with `.apply_migrations(false)`.

**Do:** if `auth_sessions` is large, deploy in a quiet window. Start one
instance of 4.4 first and let it finish booting before you start the rest,
so only one process widens the tables. Sign-ins and new sessions wait while
`auth_sessions` is copied. If a health check stops that first instance
before it finishes, the copy is rolled back and the next boot starts it
again, so give the first boot enough time. If you added your own index on
one of these columns, `migrate` leaves that column narrow and logs a
warning naming the index. Drop the index, or give it a key length, and boot
again. Postgres and SQLite need nothing.

### Registration answers a validation failure with 422.

**Before:** `Auth::password().register` answered a weak password or an
unusable address with a 401 error.

**Now:** it answers the framework's validation error: 422, with the message
under the field (`password` or `email`) in `errors`. Sign-in still answers
invalid credentials as before.

**Do:** if your code or your front end matched the 401 from registration,
match the 422 validation error instead.

### Email addresses longer than 254 characters never reach the database.

**Before:** Magnetar accepted an address of any length. On MySQL a long one
failed to save, and sign-in with one answered 503.

**Now:** sign-in with an address longer than 254 characters answers as an
unknown address does. Sign-up, the account import and OAuth email completion
refuse one as a validation error. An account that already holds such an
address can no longer sign in by email. The first `migrate` on a database
logs how many such addresses `app_users` holds, without naming them.

**Do:** if that warning appears, contact those users and change their
address. Nothing else changes.

## Fixed entries with no upgrade step

- **Two servers migrating Magnetar's tables at once no longer collide.** Nothing to change.
- **Session metadata stays within its columns.** Nothing to change.

## Next

- [Authentication](authentication.md) documents Magnetar's schema and
  `apply_migrations`.
- [Upgrading to 4.3](upgrade-4.3.md) covers the preceding release.
