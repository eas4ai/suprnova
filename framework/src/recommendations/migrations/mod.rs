//! Applications opt into the versioned graph handoff by listing these migrations.
mod m_2026_10_14_000001_recommendations;
mod m_2026_10_17_000001_recommendation_graph;
pub use m_2026_10_14_000001_recommendations::Migration as CreateRecommendationTables;
pub use m_2026_10_17_000001_recommendation_graph::Migration as CreateRecommendationGraph;
/// Register in the application's migrator so deployment owns schema changes.
pub fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    vec![
        Box::new(CreateRecommendationTables),
        Box::new(CreateRecommendationGraph),
    ]
}
