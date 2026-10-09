//! Migrations an application runs besides its own migrator's list.
//!
//! [`Application::migrations::<M>()`](crate::Application::migrations) names
//! one migrator, and a migration a crate or the framework ships had to be
//! copied into that migrator's list by hand: a release that added one ran
//! nowhere until every application listed it. Laravel's service providers
//! hand the migrator their paths with `loadMigrationsFrom`; here an
//! application hands over a list with
//! [`Application::load_migrations_from`](crate::Application::load_migrations_from),
//! and a crate registers its own with [`register_migrations!`](crate::register_migrations).
//!
//! Every migrate command of the application binary (`migrate`,
//! `migrate:status`, `migrate:rollback`, `migrate:fresh`, `schema:dump` and
//! the migration `serve` runs on boot) runs the migrator's list, then the
//! loaded lists in the order they were loaded, then the registered lists in
//! the order of their owners' names. A migration whose name an earlier list
//! holds is left out: SeaORM runs the list it is given and would run it
//! twice.

use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::RwLock;

use sea_orm_migration::prelude::{DynIden, MigrationTrait, MigratorTrait};

/// A function that returns migrations, in the order they run. The shape of
/// `payments::migrations::migrations` and
/// `auth_flows::two_factor::migrations`.
pub type MigrationList = fn() -> Vec<Box<dyn MigrationTrait>>;

/// One crate's migrations, submitted by [`register_migrations!`](crate::register_migrations).
///
/// An application picks a registration up by linking the crate that makes
/// it: the `inventory` collection only sees code the binary links.
pub struct RegisteredMigrations {
    /// Who registered the list, usually the crate's name. The registered
    /// lists run in the order of these names, so the order does not depend
    /// on the order the linker placed them in.
    pub owner: &'static str,
    /// The function that returns the migrations.
    pub list: MigrationList,
}

inventory::collect!(RegisteredMigrations);

/// Register `list`, a [`MigrationList`], under `owner` for every
/// application that links this crate, as a Laravel package's service
/// provider calls `loadMigrationsFrom`.
///
/// ```rust,no_run
/// // In the crate's lib.rs; list the crate's migrations in order.
/// fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
///     Vec::new()
/// }
///
/// suprnova::register_migrations!("acme-billing", migrations);
/// # fn main() {}
/// ```
///
/// The registration holds in every binary that links the crate, so keep it
/// in a crate whose tables every such application wants. The framework
/// registers none of its own: an application loads those it uses with
/// [`Application::load_migrations_from`](crate::Application::load_migrations_from).
#[macro_export]
macro_rules! register_migrations {
    ($owner:expr, $list:expr $(,)?) => {
        $crate::inventory::submit! {
            $crate::database::migration_registry::RegisteredMigrations {
                owner: $owner,
                list: $list,
            }
        }
    };
}

/// The lists the running application loaded. Set by its run before any
/// migrate command, because SeaORM reads a migrator's list through a
/// function with no receiver.
static LOADED: RwLock<Vec<MigrationList>> = RwLock::new(Vec::new());

/// Install the lists `Application::load_migrations_from` collected, in
/// place of any an earlier run installed.
pub(crate) fn install_loaded(lists: Vec<MigrationList>) {
    *LOADED.write().unwrap_or_else(|p| p.into_inner()) = lists;
}

/// Every migration to run after `own`: `own` first, then the loaded lists,
/// then the registered ones by owner, each name once.
fn combined(own: Vec<Box<dyn MigrationTrait>>) -> Vec<Box<dyn MigrationTrait>> {
    let loaded = LOADED.read().unwrap_or_else(|p| p.into_inner()).clone();
    let mut registered: Vec<&RegisteredMigrations> = inventory::iter::<RegisteredMigrations>
        .into_iter()
        .collect();
    registered.sort_by_key(|registration| registration.owner);

    let mut seen = HashSet::new();
    let mut all = Vec::new();
    let lists = std::iter::once(own)
        .chain(loaded.iter().map(|list| list()))
        .chain(registered.iter().map(|registration| (registration.list)()));
    for list in lists {
        for migration in list {
            if seen.insert(migration.name().to_owned()) {
                all.push(migration);
            }
        }
    }
    all
}

/// `M` with the loaded and registered migrations after its own list. Every
/// migrate path of `Application` runs through it; the ledger table stays
/// `M`'s.
pub(crate) struct WithRegistered<M>(PhantomData<M>);

#[async_trait::async_trait]
impl<M: MigratorTrait> MigratorTrait for WithRegistered<M> {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        combined(M::migrations())
    }

    fn migration_table_name() -> DynIden {
        M::migration_table_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm_migration::prelude::*;

    struct Named(&'static str);

    impl MigrationName for Named {
        fn name(&self) -> &str {
            self.0
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Named {
        async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Ok(())
        }
    }

    fn names(list: &[Box<dyn MigrationTrait>]) -> Vec<&str> {
        list.iter().map(|migration| migration.name()).collect()
    }

    #[test]
    fn a_name_an_earlier_list_holds_is_left_out() {
        let own: Vec<Box<dyn MigrationTrait>> = vec![Box::new(Named("a")), Box::new(Named("b"))];
        // No list is loaded in this unit test binary, and it registers none.
        let all = combined(own);
        assert_eq!(names(&all), ["a", "b"]);

        let twice: Vec<Box<dyn MigrationTrait>> = vec![
            Box::new(Named("a")),
            Box::new(Named("b")),
            Box::new(Named("a")),
        ];
        assert_eq!(names(&combined(twice)), ["a", "b"]);
    }
}
