//! Applications opt into the versioned graph handoff by listing these migrations.
mod m_2026_10_14_000001_recommendations;
pub use m_2026_10_14_000001_recommendations::Migration as CreateRecommendationTables;
/// Register in the application's migrator so deployment owns schema changes.
pub fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    vec![Box::new(CreateRecommendationTables)]
}
