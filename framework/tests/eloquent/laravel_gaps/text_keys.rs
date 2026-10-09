//! The tenth parity round's seeded order on a text-keyed model, observed by
//! the `par-laravel-gaps-data` mechanism.

use crate::query_fixture::Fixture;
use suprnova::{Model, model};

#[model(
    table = "gap_text_keys",
    key_type = "String",
    auto_increment = false,
    fillable = ["id", "label"],
    timestamps = false,
)]
pub struct GapTextKey {
    pub id: String,
    pub label: String,
}

async fn seeded_ids(seed: u64) -> Vec<String> {
    GapTextKey::query()
        .in_random_order_seeded(seed)
        .get()
        .await
        .expect("seeded order on a text key")
        .iter()
        .map(|row| row.id.clone())
        .collect()
}

#[tokio::test]
async fn seeded_order_shuffles_a_text_keyed_model_differently_per_seed() {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE gap_text_keys (id TEXT PRIMARY KEY, label TEXT NOT NULL)")
        .await;
    for key in ["k_a", "k_b", "k_c", "k_d", "k_e", "k_f", "k_g", "k_h"] {
        fx.exec(&format!(
            "INSERT INTO gap_text_keys VALUES ('{key}', 'row {key}')"
        ))
        .await;
    }
    let one = seeded_ids(1).await;
    assert_eq!(one.len(), 8);
    assert_eq!(one, seeded_ids(1).await, "the same seed repeats its order");
    let two = seeded_ids(2).await;
    assert_ne!(one, two, "two seeds shuffle a text-keyed model differently");
}
