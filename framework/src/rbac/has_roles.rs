//! Role and permission helpers for authenticatable models.

use async_trait::async_trait;
use sea_orm::{DatabaseBackend, Value};

use crate::{Authenticatable, DB, FrameworkError};

const DEFAULT_GUARD: &str = "web";

fn value(s: impl Into<String>) -> Value {
    Value::from(s.into())
}

fn int_value(i: i64) -> Value {
    Value::from(i)
}

/// Rewrite this module's `?` placeholders for the active backend.
///
/// `DB::select_one` / `DB::insert` / `DB::scalar` document that placeholders
/// are the caller's job and must already match the backend. Every statement
/// here used `?`, which Postgres rejects outright - so the whole RBAC
/// surface (roles, permissions, and every `has_role` / `has_permission`
/// check built on it) failed on Postgres, silently unexercised because the
/// suite is SQLite-only.
///
/// The rewrite is positional and deliberately naive, which is safe *here*
/// and only here: every statement in this module is a static string with
/// sequential binds, no quoted literals, and no JSONB operators. Postgres
/// spells three JSON operators `?`, `?|`, and `?&`, so the same shortcut in
/// `DB`'s public helpers would corrupt a caller's containment query - which
/// is why this stays module-local instead of moving up.
fn render(sql: &str, backend: DatabaseBackend) -> String {
    if backend != DatabaseBackend::Postgres {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0usize;
    for ch in sql.chars() {
        if ch == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(ch);
        }
    }
    out
}

/// The active backend, for [`render`].
fn backend() -> Result<DatabaseBackend, FrameworkError> {
    Ok(DB::connection()?.inner().get_database_backend())
}

/// One statement this module issues, beside the tables it names.
///
/// The list is the render-cache observation contract for the statement: a
/// read observes every table in it, and a write advances the single table
/// in it. `rbac_statements_name_every_table_they_read` parses the `FROM`
/// and `JOIN` clauses of every read below and refuses any list that does
/// not match, so a statement that grows a join without naming the joined
/// table fails the suite instead of silently observing less than it read.
#[derive(Clone, Copy)]
pub(crate) struct ObservedStatement {
    /// The statement text, with this module's `?` placeholders.
    pub(crate) sql: &'static str,
    /// Every table the statement names, in the order it names them.
    pub(crate) tables: &'static [&'static str],
}

const FIND_ROLE_ID: ObservedStatement = ObservedStatement {
    sql: "SELECT id FROM roles WHERE name = ? AND guard_name = ? LIMIT 1",
    tables: &["roles"],
};
const FIND_PERMISSION_ID: ObservedStatement = ObservedStatement {
    sql: "SELECT id FROM permissions WHERE name = ? AND guard_name = ? LIMIT 1",
    tables: &["permissions"],
};
const ROLE_HAS_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM role_permissions WHERE role_id = ? AND permission_id = ?",
    tables: &["role_permissions"],
};
const MODEL_HAS_ROLE_ID: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM model_roles WHERE model_type = ? AND model_id = ? AND role_id = ?",
    tables: &["model_roles"],
};
const MODEL_HAS_PERMISSION_ID: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM model_permissions WHERE model_type = ? AND model_id = ? AND \
          permission_id = ?",
    tables: &["model_permissions"],
};
const MODEL_HAS_ROLE_NAMED: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM model_roles \
          INNER JOIN roles ON roles.id = model_roles.role_id \
          WHERE model_roles.model_type = ? \
            AND model_roles.model_id = ? \
            AND roles.name = ? \
            AND roles.guard_name = ?",
    tables: &["model_roles", "roles"],
};
const MODEL_HAS_DIRECT_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM model_permissions \
          INNER JOIN permissions ON permissions.id = model_permissions.permission_id \
          WHERE model_permissions.model_type = ? \
            AND model_permissions.model_id = ? \
            AND permissions.name = ? \
            AND permissions.guard_name = ?",
    tables: &["model_permissions", "permissions"],
};
const MODEL_HAS_INHERITED_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "SELECT COUNT(*) FROM model_roles \
          INNER JOIN roles ON roles.id = model_roles.role_id \
          INNER JOIN role_permissions ON role_permissions.role_id = roles.id \
          INNER JOIN permissions ON permissions.id = role_permissions.permission_id \
          WHERE model_roles.model_type = ? \
            AND model_roles.model_id = ? \
            AND permissions.name = ? \
            AND permissions.guard_name = ? \
            AND roles.guard_name = ?",
    tables: &["model_roles", "roles", "role_permissions", "permissions"],
};

const INSERT_ROLE: ObservedStatement = ObservedStatement {
    sql: "INSERT INTO roles (name, display_name, guard_name) VALUES (?, ?, ?)",
    tables: &["roles"],
};
const INSERT_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "INSERT INTO permissions (name, display_name, guard_name) VALUES (?, ?, ?)",
    tables: &["permissions"],
};
const INSERT_ROLE_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "INSERT INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
    tables: &["role_permissions"],
};
const INSERT_MODEL_ROLE: ObservedStatement = ObservedStatement {
    sql: "INSERT INTO model_roles (model_type, model_id, role_id) VALUES (?, ?, ?)",
    tables: &["model_roles"],
};
const INSERT_MODEL_PERMISSION: ObservedStatement = ObservedStatement {
    sql: "INSERT INTO model_permissions (model_type, model_id, permission_id) VALUES (?, ?, ?)",
    tables: &["model_permissions"],
};

/// Every read statement this module issues, for the table-list contract.
#[cfg(any(test, feature = "testing"))]
pub(crate) const READ_STATEMENTS: &[ObservedStatement] = &[
    FIND_ROLE_ID,
    FIND_PERMISSION_ID,
    ROLE_HAS_PERMISSION,
    MODEL_HAS_ROLE_ID,
    MODEL_HAS_PERMISSION_ID,
    MODEL_HAS_ROLE_NAMED,
    MODEL_HAS_DIRECT_PERMISSION,
    MODEL_HAS_INHERITED_PERMISSION,
];

/// Every write statement this module issues, for the same contract.
#[cfg(any(test, feature = "testing"))]
pub(crate) const WRITE_STATEMENTS: &[ObservedStatement] = &[
    INSERT_ROLE,
    INSERT_PERMISSION,
    INSERT_ROLE_PERMISSION,
    INSERT_MODEL_ROLE,
    INSERT_MODEL_PERMISSION,
];

/// `DB::select_one_observing` with this module's placeholders rendered
/// first and the statement's own table list handed along.
async fn select_one(
    statement: ObservedStatement,
    values: Vec<Value>,
) -> Result<Option<crate::database::DynamicRow>, FrameworkError> {
    DB::select_one_observing(&render(statement.sql, backend()?), values, statement.tables).await
}

/// `DB::scalar_observing` for the `SELECT COUNT(*)` existence checks.
async fn exists(statement: ObservedStatement, values: Vec<Value>) -> Result<bool, FrameworkError> {
    let count: i64 =
        DB::scalar_observing(&render(statement.sql, backend()?), values, statement.tables).await?;
    Ok(count > 0)
}

/// `DB::affecting_statement_on_table` for this module's inserts: the one
/// table the statement writes advances, rather than the broad authority
/// `DB::insert` would have advanced.
async fn insert(statement: ObservedStatement, values: Vec<Value>) -> Result<bool, FrameworkError> {
    let table = statement
        .tables
        .first()
        .copied()
        .ok_or_else(|| FrameworkError::internal("rbac insert statement names no table"))?;
    let rows =
        DB::affecting_statement_on_table(&render(statement.sql, backend()?), values, table).await?;
    Ok(rows > 0)
}

async fn find_role_id(name: &str, guard_name: &str) -> Result<Option<i64>, FrameworkError> {
    let row = select_one(FIND_ROLE_ID, vec![value(name), value(guard_name)]).await?;
    row.map(|row| row.get_int("id")).transpose()
}

async fn find_permission_id(name: &str, guard_name: &str) -> Result<Option<i64>, FrameworkError> {
    let row = select_one(FIND_PERMISSION_ID, vec![value(name), value(guard_name)]).await?;
    row.map(|row| row.get_int("id")).transpose()
}

/// Create a role on the default `"web"` guard, returning its id.
///
/// The helper is idempotent: an existing `(name, guard_name)` row is returned
/// instead of inserting a duplicate.
pub async fn create_role(name: &str) -> Result<i64, FrameworkError> {
    create_role_on_guard(name, DEFAULT_GUARD).await
}

/// Create a role for a named guard, returning its id.
///
/// Use this when an app separates session and token principals with distinct
/// guards.
pub async fn create_role_on_guard(name: &str, guard_name: &str) -> Result<i64, FrameworkError> {
    if let Some(id) = find_role_id(name, guard_name).await? {
        return Ok(id);
    }
    insert(
        INSERT_ROLE,
        vec![value(name), value(name), value(guard_name)],
    )
    .await?;
    find_role_id(name, guard_name)
        .await?
        .ok_or_else(|| FrameworkError::database("rbac create_role: inserted role was not found"))
}

/// Create a permission on the default `"web"` guard, returning its id.
///
/// The helper is idempotent: an existing `(name, guard_name)` row is returned
/// instead of inserting a duplicate.
pub async fn create_permission(name: &str) -> Result<i64, FrameworkError> {
    create_permission_on_guard(name, DEFAULT_GUARD).await
}

/// Create a permission for a named guard, returning its id.
///
/// Permissions are conventionally dotted ability names such as
/// `"articles.publish"`.
pub async fn create_permission_on_guard(
    name: &str,
    guard_name: &str,
) -> Result<i64, FrameworkError> {
    if let Some(id) = find_permission_id(name, guard_name).await? {
        return Ok(id);
    }
    insert(
        INSERT_PERMISSION,
        vec![value(name), value(name), value(guard_name)],
    )
    .await?;
    find_permission_id(name, guard_name).await?.ok_or_else(|| {
        FrameworkError::database("rbac create_permission: inserted permission was not found")
    })
}

/// Give a permission to a role on the default `"web"` guard.
///
/// Missing roles or permissions are created before the assignment is stored.
pub async fn give_permission_to_role(
    role_name: &str,
    permission_name: &str,
) -> Result<(), FrameworkError> {
    give_permission_to_role_on_guard(role_name, permission_name, DEFAULT_GUARD).await
}

/// Give a permission to a role for a named guard.
///
/// The assignment is idempotent and ignores an existing join row.
pub async fn give_permission_to_role_on_guard(
    role_name: &str,
    permission_name: &str,
    guard_name: &str,
) -> Result<(), FrameworkError> {
    let role_id = create_role_on_guard(role_name, guard_name).await?;
    let permission_id = create_permission_on_guard(permission_name, guard_name).await?;
    if exists(
        ROLE_HAS_PERMISSION,
        vec![int_value(role_id), int_value(permission_id)],
    )
    .await?
    {
        return Ok(());
    }
    insert(
        INSERT_ROLE_PERMISSION,
        vec![int_value(role_id), int_value(permission_id)],
    )
    .await?;
    Ok(())
}

/// Assign a role to a model on the default `"web"` guard.
///
/// `model_type` is the model discriminator - for [`HasRoles`] implementors
/// this is [`HasRoles::rbac_model_type`], which defaults to the
/// fully-qualified Rust type path.
pub async fn assign_role_to_model(
    model_type: &str,
    model_id: &str,
    role_name: &str,
) -> Result<(), FrameworkError> {
    assign_role_to_model_on_guard(model_type, model_id, role_name, DEFAULT_GUARD).await
}

/// Assign a role to a model for a named guard.
///
/// The assignment is idempotent and stores `model_id` as a string so numeric,
/// UUID, and external-provider identifiers all work.
pub async fn assign_role_to_model_on_guard(
    model_type: &str,
    model_id: &str,
    role_name: &str,
    guard_name: &str,
) -> Result<(), FrameworkError> {
    let role_id = create_role_on_guard(role_name, guard_name).await?;
    if exists(
        MODEL_HAS_ROLE_ID,
        vec![value(model_type), value(model_id), int_value(role_id)],
    )
    .await?
    {
        return Ok(());
    }
    insert(
        INSERT_MODEL_ROLE,
        vec![value(model_type), value(model_id), int_value(role_id)],
    )
    .await?;
    Ok(())
}

/// Give a direct permission to a model on the default `"web"` guard.
///
/// Direct permissions are checked in addition to permissions inherited from
/// assigned roles.
pub async fn give_permission_to_model(
    model_type: &str,
    model_id: &str,
    permission_name: &str,
) -> Result<(), FrameworkError> {
    give_permission_to_model_on_guard(model_type, model_id, permission_name, DEFAULT_GUARD).await
}

/// Give a direct permission to a model for a named guard.
///
/// The assignment is idempotent and creates the permission row when needed.
pub async fn give_permission_to_model_on_guard(
    model_type: &str,
    model_id: &str,
    permission_name: &str,
    guard_name: &str,
) -> Result<(), FrameworkError> {
    let permission_id = create_permission_on_guard(permission_name, guard_name).await?;
    if exists(
        MODEL_HAS_PERMISSION_ID,
        vec![value(model_type), value(model_id), int_value(permission_id)],
    )
    .await?
    {
        return Ok(());
    }
    insert(
        INSERT_MODEL_PERMISSION,
        vec![value(model_type), value(model_id), int_value(permission_id)],
    )
    .await?;
    Ok(())
}

/// Check whether a model has a role on the default `"web"` guard.
pub async fn has_role_for_model(
    model_type: &str,
    model_id: &str,
    role_name: &str,
) -> Result<bool, FrameworkError> {
    has_role_for_model_on_guard(model_type, model_id, role_name, DEFAULT_GUARD).await
}

/// Check whether a model has a role for a named guard.
pub async fn has_role_for_model_on_guard(
    model_type: &str,
    model_id: &str,
    role_name: &str,
    guard_name: &str,
) -> Result<bool, FrameworkError> {
    exists(
        MODEL_HAS_ROLE_NAMED,
        vec![
            value(model_type),
            value(model_id),
            value(role_name),
            value(guard_name),
        ],
    )
    .await
}

/// Check whether a model has a permission on the default `"web"` guard.
///
/// Both direct model permissions and permissions inherited through assigned
/// roles are considered.
pub async fn has_permission_for_model(
    model_type: &str,
    model_id: &str,
    permission_name: &str,
) -> Result<bool, FrameworkError> {
    has_permission_for_model_on_guard(model_type, model_id, permission_name, DEFAULT_GUARD).await
}

/// Check whether a model has a permission for a named guard.
///
/// Both direct model permissions and permissions inherited through assigned
/// roles are considered.
pub async fn has_permission_for_model_on_guard(
    model_type: &str,
    model_id: &str,
    permission_name: &str,
    guard_name: &str,
) -> Result<bool, FrameworkError> {
    let direct = exists(
        MODEL_HAS_DIRECT_PERMISSION,
        vec![
            value(model_type),
            value(model_id),
            value(permission_name),
            value(guard_name),
        ],
    )
    .await?;
    if direct {
        return Ok(true);
    }

    exists(
        MODEL_HAS_INHERITED_PERMISSION,
        vec![
            value(model_type),
            value(model_id),
            value(permission_name),
            value(guard_name),
            value(guard_name),
        ],
    )
    .await
}

/// Trait for authenticatable models that can receive RBAC roles and
/// permissions.
///
/// The default model discriminator is the fully-qualified Rust type path
/// (`crate::models::user::User`, not the short leaf `User`). Using the full
/// path means two distinct authenticatable types that happen to share a leaf
/// name cannot silently collide on the same `(model_type, model_id)` rows -
/// which would leak one type's roles and permissions onto the other.
///
/// Override [`Self::rbac_model_type`] when an app wants a stable custom
/// discriminator. Two requirements for any override:
///
/// 1. **It must be globally unique** across every authenticatable type that
///    shares the same RBAC tables - two types returning the same string will
///    share roles and permissions for matching ids.
/// 2. **Prefer an override for any persisted discriminator you depend on.**
///    `std::any::type_name` is *not* guaranteed stable across compiler
///    versions or refactors (renaming or moving the type changes it), so apps
///    that persist `model_type` long-term and need it to stay constant should
///    return their own fixed string rather than rely on the default.
#[async_trait]
pub trait HasRoles: Authenticatable {
    /// Model discriminator stored in `model_roles.model_type` and
    /// `model_permissions.model_type`.
    fn rbac_model_type(&self) -> String {
        std::any::type_name::<Self>().to_string()
    }

    /// Model identifier stored in `model_roles.model_id` and
    /// `model_permissions.model_id`.
    fn rbac_model_id(&self) -> String {
        self.get_auth_identifier()
    }

    /// Assign this model a role on the default `"web"` guard.
    async fn assign_role(&self, role_name: &str) -> Result<(), FrameworkError> {
        assign_role_to_model(&self.rbac_model_type(), &self.rbac_model_id(), role_name).await
    }

    /// Give this model a direct permission on the default `"web"` guard.
    async fn give_permission_to(&self, permission_name: &str) -> Result<(), FrameworkError> {
        give_permission_to_model(
            &self.rbac_model_type(),
            &self.rbac_model_id(),
            permission_name,
        )
        .await
    }

    /// Check whether this model has a role on the default `"web"` guard.
    async fn has_role(&self, role_name: &str) -> Result<bool, FrameworkError> {
        has_role_for_model(&self.rbac_model_type(), &self.rbac_model_id(), role_name).await
    }

    /// Check whether this model has a permission on the default `"web"` guard.
    ///
    /// Direct permissions and role-inherited permissions are both considered.
    async fn has_permission_to(&self, permission_name: &str) -> Result<bool, FrameworkError> {
        has_permission_for_model(
            &self.rbac_model_type(),
            &self.rbac_model_id(),
            permission_name,
        )
        .await
    }
}

/// One statement's SQL beside its declared table list, in the shape
/// [`observed_rbac_statements_for_test`] returns - named so its signature
/// reads as two lists instead of a nested tuple type.
#[cfg(any(test, feature = "testing"))]
type ObservedStatementForTest = (&'static str, &'static [&'static str]);

/// The statements this module issues, each beside the tables it names, for
/// the contract test that parses the SQL back.
///
/// Test seam: the lists are the render-cache observation contract, and the
/// only way to check them against the SQL they belong to is to hand both to
/// a test. Returns reads first, writes second.
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
#[must_use]
pub fn observed_rbac_statements_for_test()
-> (Vec<ObservedStatementForTest>, Vec<ObservedStatementForTest>) {
    let map = |statements: &'static [ObservedStatement]| {
        statements
            .iter()
            .map(|statement| (statement.sql, statement.tables))
            .collect()
    };
    (map(READ_STATEMENTS), map(WRITE_STATEMENTS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Arc;

    // Two distinct authenticatable types that share the leaf name `Account`
    // but live in separate modules, so their fully-qualified type paths
    // differ. With the short-leaf-name default they collided; the
    // fully-qualified default must keep them apart.
    mod first {
        use super::*;

        pub struct Account {
            pub id: i64,
        }

        impl Authenticatable for Account {
            fn get_auth_identifier(&self) -> String {
                self.id.to_string()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
                self
            }
        }

        impl HasRoles for Account {}
    }

    mod second {
        use super::*;

        pub struct Account {
            pub id: i64,
        }

        impl Authenticatable for Account {
            fn get_auth_identifier(&self) -> String {
                self.id.to_string()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
                self
            }
        }

        impl HasRoles for Account {}
    }

    #[test]
    fn distinct_types_with_same_leaf_name_get_distinct_discriminators() {
        let a = first::Account { id: 7 };
        let b = second::Account { id: 7 };

        // Same leaf name, same id - but the discriminators must differ so
        // the two types cannot share roles/permissions rows.
        assert_ne!(
            a.rbac_model_type(),
            b.rbac_model_type(),
            "distinct types sharing a leaf name must not share an RBAC discriminator"
        );
        // The default is the fully-qualified type path, not the leaf.
        assert!(a.rbac_model_type().contains("::"));
        assert!(a.rbac_model_type().ends_with("Account"));
        assert!(b.rbac_model_type().ends_with("Account"));
    }

    #[test]
    fn render_numbers_placeholders_for_postgres_only() {
        let sql = "SELECT id FROM roles WHERE name = ? AND guard_name = ? LIMIT 1";
        assert_eq!(
            render(sql, DatabaseBackend::Postgres),
            "SELECT id FROM roles WHERE name = $1 AND guard_name = $2 LIMIT 1",
            "Postgres rejects `?` outright - this is the whole bug"
        );
        for backend in [DatabaseBackend::Sqlite, DatabaseBackend::MySql] {
            assert_eq!(
                render(sql, backend),
                sql,
                "{backend:?} takes `?` natively and must be left alone"
            );
        }
    }

    /// The widest statement in the module: five binds across four joins.
    /// Ordinals must run 1..=5 in source order, or the binds land on the
    /// wrong columns and the check silently answers the wrong question.
    #[test]
    fn render_numbers_every_bind_in_order() {
        let sql = "SELECT COUNT(*) FROM model_roles \
         WHERE model_roles.model_type = ? \
           AND model_roles.model_id = ? \
           AND permissions.name = ? \
           AND permissions.guard_name = ? \
           AND roles.guard_name = ?";
        let out = render(sql, DatabaseBackend::Postgres);
        for n in 1..=5 {
            assert!(out.contains(&format!("${n}")), "missing ${n} in: {out}");
        }
        assert!(!out.contains('?'), "no `?` may survive: {out}");
        assert!(
            out.find("$1") < out.find("$2")
                && out.find("$2") < out.find("$3")
                && out.find("$3") < out.find("$4")
                && out.find("$4") < out.find("$5"),
            "ordinals must follow source order or binds bind the wrong columns: {out}"
        );
    }

    #[test]
    fn render_leaves_a_placeholder_free_statement_untouched() {
        let sql = "SELECT COUNT(*) FROM roles";
        assert_eq!(render(sql, DatabaseBackend::Postgres), sql);
    }

    #[test]
    fn rbac_model_id_uses_auth_identifier() {
        let a = first::Account { id: 42 };
        assert_eq!(a.rbac_model_id(), "42");
    }
}
