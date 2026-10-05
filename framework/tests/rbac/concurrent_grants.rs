//! IDENTITY-021: every RBAC create and grant converges under concurrency.
//!
//! Each helper checks for an existing row and inserts one when there is
//! none. Two requests that both see none both insert, and the second used to
//! report a duplicate-key error although the state it asked for existed.
//!
//! These tests land the competing row exactly between the check and the
//! insert. On SQLite a trigger inserts it as the helper's own insert begins.
//! On Postgres and MySQL a second connection holds it uncommitted: the
//! helper's check cannot see it, the helper's insert waits on it, and the
//! holder commits only once the server reports that wait. The helper must
//! then succeed, and exactly one row must hold the key.

use std::time::Duration;

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement, TransactionTrait};
use suprnova::FrameworkError;
use suprnova::rbac::migrations::CreateRbacTables;
use suprnova::rbac::{
    assign_role_to_model, create_permission, create_role, give_permission_to_model,
    give_permission_to_role,
};
use suprnova::testing::TestDatabase;

/// One RBAC write whose last statement is an insert that can race.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Grant {
    Role,
    Permission,
    RolePermission,
    ModelRole,
    ModelPermission,
}

/// Every racing insert the helpers issue.
pub(crate) const GRANTS: [Grant; 5] = [
    Grant::Role,
    Grant::Permission,
    Grant::RolePermission,
    Grant::ModelRole,
    Grant::ModelPermission,
];

const ROLE: &str = "racer";
const PERMISSION: &str = "racer.act";
const MODEL_TYPE: &str = "racer-model";
const MODEL_ID: &str = "7";

impl Grant {
    /// The table the racing insert writes.
    pub(crate) fn table(self) -> &'static str {
        match self {
            Self::Role => "roles",
            Self::Permission => "permissions",
            Self::RolePermission => "role_permissions",
            Self::ModelRole => "model_roles",
            Self::ModelPermission => "model_permissions",
        }
    }

    /// Creates what the helper reads before its check, so only its last
    /// insert races.
    pub(crate) async fn seed(self) {
        if matches!(self, Self::RolePermission | Self::ModelRole) {
            create_role(ROLE).await.expect("seed the role");
        }
        if matches!(self, Self::RolePermission | Self::ModelPermission) {
            create_permission(PERMISSION)
                .await
                .expect("seed the permission");
        }
    }

    /// The row another request inserts, as SQL with literal values. Runs
    /// after [`Self::seed`], so the role and permission ids exist.
    pub(crate) async fn competing_insert(self) -> String {
        match self {
            Self::Role => format!(
                "INSERT INTO roles (name, display_name, guard_name) \
                 VALUES ('{ROLE}', '{ROLE}', 'web')"
            ),
            Self::Permission => format!(
                "INSERT INTO permissions (name, display_name, guard_name) \
                 VALUES ('{PERMISSION}', '{PERMISSION}', 'web')"
            ),
            Self::RolePermission => {
                let role = create_role(ROLE).await.expect("the seeded role");
                let permission = create_permission(PERMISSION)
                    .await
                    .expect("the seeded permission");
                format!(
                    "INSERT INTO role_permissions (role_id, permission_id) \
                     VALUES ({role}, {permission})"
                )
            }
            Self::ModelRole => {
                let role = create_role(ROLE).await.expect("the seeded role");
                format!(
                    "INSERT INTO model_roles (model_type, model_id, role_id) \
                     VALUES ('{MODEL_TYPE}', '{MODEL_ID}', {role})"
                )
            }
            Self::ModelPermission => {
                let permission = create_permission(PERMISSION)
                    .await
                    .expect("the seeded permission");
                format!(
                    "INSERT INTO model_permissions (model_type, model_id, permission_id) \
                     VALUES ('{MODEL_TYPE}', '{MODEL_ID}', {permission})"
                )
            }
        }
    }

    /// The helper under test.
    pub(crate) async fn run(self) -> Result<(), FrameworkError> {
        match self {
            Self::Role => create_role(ROLE).await.map(drop),
            Self::Permission => create_permission(PERMISSION).await.map(drop),
            Self::RolePermission => give_permission_to_role(ROLE, PERMISSION).await,
            Self::ModelRole => assign_role_to_model(MODEL_TYPE, MODEL_ID, ROLE).await,
            Self::ModelPermission => {
                give_permission_to_model(MODEL_TYPE, MODEL_ID, PERMISSION).await
            }
        }
    }

    /// Counts the rows that hold the raced key.
    pub(crate) fn count_sql(self) -> String {
        let filter = match self {
            Self::Role => format!("name = '{ROLE}' AND guard_name = 'web'"),
            Self::Permission => format!("name = '{PERMISSION}' AND guard_name = 'web'"),
            Self::RolePermission => "1 = 1".to_owned(),
            Self::ModelRole | Self::ModelPermission => {
                format!("model_type = '{MODEL_TYPE}' AND model_id = '{MODEL_ID}'")
            }
        };
        format!("SELECT COUNT(*) FROM {} WHERE {filter}", self.table())
    }
}

/// `SELECT COUNT(*)` on `connection`, as an `i64` on every backend.
pub(crate) async fn count(connection: &impl ConnectionTrait, sql: &str) -> i64 {
    let row = connection
        .query_one_raw(Statement::from_string(
            connection.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .expect("run a count")
        .expect("a count returns a row");
    row.try_get_by_index::<i64>(0).expect("read the count")
}

struct RbacMigrator;

impl sea_orm_migration::MigratorTrait for RbacMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(CreateRbacTables)]
    }
}

/// On SQLite, a trigger lands the competing row as the helper's insert
/// begins: after its check saw no row, before its insert writes one.
#[tokio::test]
async fn sqlite_every_grant_converges_when_a_concurrent_request_inserts_first() {
    for grant in GRANTS {
        let db = TestDatabase::fresh::<RbacMigrator>().await.unwrap();
        grant.seed().await;
        let competing = grant.competing_insert().await;
        db.conn()
            .execute_unprepared(&format!(
                "CREATE TRIGGER concurrent_grant BEFORE INSERT ON {} BEGIN {competing}; END",
                grant.table()
            ))
            .await
            .expect("create the racing trigger");

        let outcome = grant.run().await;
        db.conn()
            .execute_unprepared("DROP TRIGGER concurrent_grant")
            .await
            .expect("drop the racing trigger");

        assert!(
            outcome.is_ok(),
            "{grant:?}: the state the helper asked for exists, so it must succeed - got {outcome:?}"
        );
        assert_eq!(
            count(db.conn(), &grant.count_sql()).await,
            1,
            "{grant:?}: exactly one row holds the key"
        );
        grant.run().await.expect("a repeat converges too");
    }
}

/// How the server reports a session waiting on a row lock, for
/// [`race_against_a_held_row`].
pub(crate) enum LockProbe {
    Postgres,
    MySql,
}

impl LockProbe {
    /// Counts the sessions of this database whose insert waits on a lock.
    /// The probe's own text starts with `SELECT`, so it never counts itself;
    /// the holder's finished insert is idle, so it never counts either.
    fn sql(&self) -> &'static str {
        match self {
            Self::Postgres => {
                "SELECT COUNT(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' \
                 AND query LIKE 'INSERT INTO %'"
            }
            Self::MySql => {
                "SELECT COUNT(*) FROM information_schema.PROCESSLIST \
                 WHERE DB = DATABASE() AND INFO LIKE 'INSERT INTO %'"
            }
        }
    }
}

/// Waits until `watcher` sees an insert waiting on a lock. The insert under
/// test can only wait on the held row, so this is the moment the race is
/// set: its check is behind it and its insert is blocked. Fails the test if
/// that never happens, which means the helper never reached its insert.
async fn until_an_insert_waits(watcher: &DatabaseConnection, probe: &LockProbe) {
    for _ in 0..1_000 {
        if count(watcher, probe.sql()).await > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the helper's insert never waited on the held row");
}

/// Runs every grant against a competing row that another connection holds
/// uncommitted, on the engine `url` names. `install` connects the framework
/// to a fresh RBAC schema on that engine for one grant.
pub(crate) async fn race_against_a_held_row<F, Fut, G>(url: &str, probe: LockProbe, install: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = G>,
{
    for grant in GRANTS {
        let installed = install().await;
        grant.seed().await;
        let competing = grant.competing_insert().await;
        let holder = Database::connect(url).await.expect("connect the holder");
        let watcher = Database::connect(url).await.expect("connect the watcher");
        let held = holder.begin().await.expect("begin the holder");
        held.execute_unprepared(&competing)
            .await
            .expect("hold the competing row");

        let (outcome, ()) = tokio::join!(grant.run(), async {
            until_an_insert_waits(&watcher, &probe).await;
            held.commit().await.expect("commit the competing row");
        });

        assert!(
            outcome.is_ok(),
            "{grant:?}: the state the helper asked for exists, so it must succeed - got {outcome:?}"
        );
        assert_eq!(
            count(&watcher, &grant.count_sql()).await,
            1,
            "{grant:?}: exactly one row holds the key"
        );
        grant.run().await.expect("a repeat converges too");
        holder.close().await.expect("close the holder");
        watcher.close().await.expect("close the watcher");
        drop(installed);
    }
}
