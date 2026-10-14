//! The scaffold's models, at the paths its templates name each other by
//! (`crate::models::user`, `crate::models::note`): `scaffold_tables` drives
//! the scaffold's `User`, and that model has many notes.

// `#[rustfmt::skip]`: the templates are scaffold output, not workspace
// source, so `cargo fmt` must not rewrite them.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/note.rs.tpl"]
pub mod note;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/user.rs.tpl"]
pub mod user;

// The scaffold's handlers read notes only through `Note::owned_by`, and none
// of them is in this binary. Naming it here checks its signature against the
// template.
const _: fn(u64) -> suprnova::Builder<note::Note> = note::Note::owned_by;

#[tokio::test]
async fn scaffold_notes_initialize_one_generation_without_replacing_it() {
    struct SearchMigrator;

    impl sea_orm_migration::MigratorTrait for SearchMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            suprnova::search::migrations()
        }
    }

    let _db = suprnova::testing::TestDatabase::fresh::<SearchMigrator>()
        .await
        .expect("fresh search migrations");
    suprnova::SearchRegistry::isolated(async {
        let engine = suprnova::SearchRegistry::engine_for("notes").expect("notes engine");
        assert_eq!(engine.current_generation("notes").await.unwrap(), None);
        note::Note::initialize_search()
            .await
            .expect("initial notes generation");
        let generation = engine.current_generation("notes").await.unwrap();
        assert!(generation.is_some());
        note::Note::initialize_search()
            .await
            .expect("existing notes generation");
        assert_eq!(
            engine.current_generation("notes").await.unwrap(),
            generation
        );
    })
    .await;
}
