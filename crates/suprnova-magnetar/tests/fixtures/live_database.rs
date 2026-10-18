//! A throwaway database on the server a live test URL names.
//!
//! Live tests that need a database of their own create one beside the
//! database the URL names and drop it at the end, so repeated runs against
//! one server never collide. A test that panics leaves its database behind.

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend};

/// A database of its own on the server a live test URL names.
pub struct LiveDatabase {
    admin: DatabaseConnection,
    name: String,
    url: String,
}

impl LiveDatabase {
    pub async fn create(server_url: &str, label: &str) -> Self {
        let admin = Database::connect(server_url)
            .await
            .expect("connect the live server");
        let name = format!("magnetar_live_{label}_{:016x}", rand::random::<u64>());
        let create = match admin.get_database_backend() {
            DbBackend::Postgres => format!("CREATE DATABASE \"{name}\""),
            _ => format!("CREATE DATABASE `{name}`"),
        };
        admin
            .execute_unprepared(&create)
            .await
            .expect("create an isolated database");
        let url = database_url(server_url, &name);
        Self { admin, name, url }
    }

    pub async fn connect(&self) -> DatabaseConnection {
        Database::connect(&self.url)
            .await
            .expect("connect the isolated database")
    }

    pub async fn drop_database(self) {
        let drop = match self.admin.get_database_backend() {
            DbBackend::Postgres => format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name),
            _ => format!("DROP DATABASE `{}`", self.name),
        };
        self.admin
            .execute_unprepared(&drop)
            .await
            .expect("drop the isolated database");
    }
}

/// The server URL with its database path replaced by `database`; any query
/// string is kept.
pub fn database_url(server_url: &str, database: &str) -> String {
    let (base, query) = server_url
        .split_once('?')
        .map_or((server_url, None), |(base, query)| (base, Some(query)));
    let authority_start = base.find("://").map_or(0, |index| index + 3);
    let authority_end = base[authority_start..]
        .find('/')
        .map_or(base.len(), |index| authority_start + index);
    let mut url = format!("{}/{database}", &base[..authority_end]);
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    url
}
