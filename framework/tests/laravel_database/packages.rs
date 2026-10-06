//! LDB-008: the framework's feature flags read and write the flags
//! laravel/pennant stores in `features`, and the RBAC reads and writes
//! spatie/laravel-permission's tables, honoring the assignments spatie
//! recorded and writing `model_type` as the model's `morph_type`.

use std::sync::Arc;

use suprnova::features::{Context, DatabaseEvaluator, Evaluator};
use suprnova::rbac::entity::{Permission, Role};
use suprnova::{HasRoles, Model};

use crate::models::LdbUser;
use crate::on_every_engine;
use crate::support::{self, Engine};

/// The flag `name` for the global scope and for user `user`, as the
/// framework answers them.
fn answers(
    evaluator: &Arc<DatabaseEvaluator>,
    name: &str,
    user: i64,
) -> (Option<bool>, Option<bool>) {
    featureflag::evaluator::with_default(Arc::clone(evaluator), || {
        let global = evaluator.is_enabled(name, &Context::root());
        let scoped = featureflag::context! { user_id = user };
        (global, evaluator.is_enabled(name, &scoped))
    })
}

/// The `(scope, value)` of every stored row for `name`, in id order.
async fn stored_flag(db: &support::Db, name: &str) -> Vec<(String, String)> {
    support::rows(
        &db.conn,
        &format!("SELECT scope, value FROM features WHERE name = '{name}' ORDER BY id"),
    )
    .await
    .iter()
    .map(|row| (support::text(row, "scope"), support::text(row, "value")))
    .collect()
}

/// Pennant's stored flags read the same through the framework, any value
/// other than `false` is enabled, and the rows the framework writes are
/// the rows Pennant writes.
async fn pennant_flags(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    let evaluator = Arc::new(DatabaseEvaluator::new().await.expect("the evaluator"));

    assert_eq!(
        answers(&evaluator, "new-api", 1),
        (Some(true), Some(true)),
        "{engine:?}"
    );
    assert_eq!(
        answers(&evaluator, "maintenance", 1),
        (Some(false), Some(false)),
        "{engine:?}"
    );
    assert_eq!(
        answers(&evaluator, "purple-theme", 1),
        (None, Some(true)),
        "{engine:?}: Pennant's rich value \"blue\" is active"
    );
    assert_eq!(answers(&evaluator, "purple-theme", 2), (None, None));
    assert_eq!(
        answers(&evaluator, "beta", 1),
        (None, Some(false)),
        "{engine:?}"
    );
    assert_eq!(
        answers(&evaluator, "beta", 2),
        (None, Some(true)),
        "{engine:?}"
    );

    // Writes take Pennant's form: the `__laravel_null` and
    // `App\Models\User|{id}` scopes, the values `true` and `false`, one
    // row per flag and scope.
    let pennant_rows = stored_flag(&db, "beta").await;
    evaluator
        .set_flag("beta", "user:3", false)
        .await
        .expect("store a user's flag");
    evaluator
        .set_flag("beta", "user:2", false)
        .await
        .expect("change a user's flag");
    let mut expected = pennant_rows.clone();
    expected[1].1 = "false".to_owned();
    expected.push(("App\\Models\\User|3".to_owned(), "false".to_owned()));
    assert_eq!(stored_flag(&db, "beta").await, expected, "{engine:?}");
    assert_eq!(
        pennant_rows[0],
        ("App\\Models\\User|1".to_owned(), "false".to_owned())
    );

    evaluator
        .set_flag("dark-mode", "", true)
        .await
        .expect("store a global flag");
    evaluator
        .set_flag("maintenance", "", true)
        .await
        .expect("change a global flag");
    assert_eq!(
        stored_flag(&db, "dark-mode").await,
        [("__laravel_null".to_owned(), "true".to_owned())],
        "{engine:?}: the row Pennant stores for an activated global flag"
    );
    assert_eq!(
        stored_flag(&db, "maintenance").await,
        [("__laravel_null".to_owned(), "true".to_owned())],
        "{engine:?}"
    );
    evaluator.reload().await.expect("reload");
    assert_eq!(answers(&evaluator, "beta", 3), (None, Some(false)));
    assert_eq!(
        answers(&evaluator, "dark-mode", 3),
        (Some(true), Some(true))
    );
    let written = support::rows(
        &db.conn,
        "SELECT created_at, updated_at FROM features WHERE name = 'dark-mode'",
    )
    .await;
    assert!(!written[0]["created_at"].is_null() && !written[0]["updated_at"].is_null());
}

on_every_engine!(pennant_flags =>
    ldb_008_pennant_flags_read_and_write_sqlite,
    ldb_008_pennant_flags_read_and_write_postgres,
    ldb_008_pennant_flags_read_and_write_mysql);

async fn user(id: u64) -> LdbUser {
    LdbUser::find(id)
        .await
        .expect("find")
        .expect("a fixture user")
}

/// The assignments spatie recorded hold, roles and permissions read
/// without `display_name`, and the framework's writes land in spatie's
/// tables with `model_type` the model's `morph_type`.
async fn spatie_permissions(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);

    let roles: Vec<String> = Role::query()
        .order_by_asc("id")
        .get()
        .await
        .expect("roles without display_name")
        .iter()
        .map(|role| role.name.clone())
        .collect();
    assert_eq!(roles, ["writer", "admin"], "{engine:?}");
    assert_eq!(
        Permission::query().get().await.expect("permissions").len(),
        3
    );

    let taylor = user(1).await;
    let abigail = user(2).await;
    let jeffrey = user(4).await;
    assert!(
        taylor.has_role("writer").await.expect("check"),
        "{engine:?}"
    );
    assert!(!taylor.has_role("admin").await.expect("check"));
    assert!(
        taylor
            .has_permission_to("edit articles")
            .await
            .expect("check"),
        "{engine:?}: a permission through a role"
    );
    assert!(
        !taylor
            .has_permission_to("delete articles")
            .await
            .expect("check")
    );
    assert!(
        abigail
            .has_permission_to("delete articles")
            .await
            .expect("check"),
        "{engine:?}: a direct permission"
    );
    assert!(!abigail.has_role("writer").await.expect("check"));
    for permission in ["edit articles", "delete articles", "publish articles"] {
        assert!(
            jeffrey.has_permission_to(permission).await.expect("check"),
            "{engine:?}: admin grants {permission}"
        );
    }

    // Writes.
    let dayle = user(3).await;
    dayle.assign_role("admin").await.expect("assign a role");
    dayle
        .give_permission_to("edit articles")
        .await
        .expect("give a permission");
    suprnova::rbac::give_permission_to_role("writer", "publish articles")
        .await
        .expect("give a role a permission");
    suprnova::rbac::create_role("editor")
        .await
        .expect("create a role");
    assert!(dayle.has_role("admin").await.expect("check"));
    assert!(
        taylor
            .has_permission_to("publish articles")
            .await
            .expect("check")
    );

    let assigned = support::rows(
        &db.conn,
        "SELECT r.name AS name, m.model_type AS model_type FROM model_has_roles m \
         JOIN roles r ON r.id = m.role_id WHERE m.model_id = 3",
    )
    .await;
    assert_eq!(assigned.len(), 1, "{engine:?}");
    assert_eq!(support::text(&assigned[0], "name"), "admin");
    assert_eq!(
        support::text(&assigned[0], "model_type"),
        "App\\Models\\User"
    );
    let direct = support::rows(
        &db.conn,
        "SELECT p.name AS name, m.model_type AS model_type FROM model_has_permissions m \
         JOIN permissions p ON p.id = m.permission_id WHERE m.model_id = 3",
    )
    .await;
    assert_eq!(direct.len(), 1, "{engine:?}");
    assert_eq!(support::text(&direct[0], "name"), "edit articles");
    assert_eq!(support::text(&direct[0], "model_type"), "App\\Models\\User");
    assert_eq!(
        support::count(
            &db.conn,
            "role_has_permissions",
            "role_id = 1 AND permission_id = 3"
        )
        .await,
        1,
        "{engine:?}"
    );
    let editor = support::rows(
        &db.conn,
        "SELECT guard_name, created_at FROM roles WHERE name = 'editor'",
    )
    .await;
    assert_eq!(support::text(&editor[0], "guard_name"), "web");
    assert!(
        !editor[0]["created_at"].is_null(),
        "{engine:?}: spatie's timestamps"
    );
}

on_every_engine!(spatie_permissions =>
    ldb_008_spatie_roles_and_permissions_sqlite,
    ldb_008_spatie_roles_and_permissions_postgres,
    ldb_008_spatie_roles_and_permissions_mysql);
