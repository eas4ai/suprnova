//! Applications opt into the versioned graph handoff by listing these migrations.
mod m_2026_10_14_000001_recommendations;
mod m_2026_10_17_000001_recommendation_graph;
mod m_2026_10_20_000001_recommendation_ranking_digest;
pub use m_2026_10_14_000001_recommendations::Migration as CreateRecommendationTables;
pub use m_2026_10_17_000001_recommendation_graph::Migration as CreateRecommendationGraph;
pub use m_2026_10_20_000001_recommendation_ranking_digest::Migration as AddRecommendationRankingDigest;
/// Register in the application's migrator so deployment owns schema changes.
pub fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    vec![
        Box::new(CreateRecommendationTables),
        Box::new(CreateRecommendationGraph),
        Box::new(AddRecommendationRankingDigest),
    ]
}
