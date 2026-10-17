//! Recommend and discover over the vectors a search generation stores (REC-006).

use super::support::*;
use suprnova::{
    EngineResult, GenerationId, IndexedDocument, NullEngine, SearchEngine, Searchable,
    VectorDiscover, VectorRecommend, Viewer,
};

/// A generation of `recommendation_notes` in a fresh collection engine, holding the
/// given notes with the given three-dimensional vectors.
async fn generation(engine: &suprnova::CollectionEngine, notes: &[(&Note, [f32; 3])]) -> GenerationId {
    let descriptor = <Note as Searchable>::descriptor();
    let base = suprnova::SearchRegistry::generation_for(descriptor.name)
        .await
        .unwrap();
    let registered = suprnova::SearchRegistry::engine_for(descriptor.name).unwrap();
    let mut settings = registered.read_settings(descriptor, &base).await.unwrap();
    settings.index_time.dimensions = Some(3);
    let id = GenerationId::new("vectors").unwrap();
    engine.create_generation(descriptor, &settings, &id).await.unwrap();
    let entry = suprnova::SearchRegistry::by_index(descriptor.name).unwrap();
    let mut documents = Vec::new();
    for (note, vector) in notes {
        let document = (entry.load_document)(&note.id.to_string()).await.unwrap().unwrap();
        documents.push(IndexedDocument {
            content_hash: document.content_hash(&settings.index_time.digest()),
            document,
            dense: Some(vector.to_vec()),
            sparse: None,
        });
    }
    engine.index(descriptor, &id, &documents).await.unwrap();
    id
}

fn keys(result: &EngineResult) -> Vec<String> {
    result.candidates.iter().map(|hit| hit.document.key.clone()).collect()
}

#[tokio::test]
async fn rec_006_recommend_folds_the_stored_vectors_toward_positives_and_away_from_negatives() {
    setup(async {
        let liked = note("Liked", "x", true, "o", 1).await;
        let disliked = note("Disliked", "x", true, "o", 2).await;
        let near_liked = note("Near liked", "x", true, "o", 3).await;
        let near_disliked = note("Near disliked", "x", true, "o", 4).await;
        let between = note("Between", "x", true, "o", 5).await;
        let private = note("Private near liked", "x", false, "bob", 6).await;
        let engine = suprnova::CollectionEngine::new();
        let id = generation(
            &engine,
            &[
                (&liked, [1.0, 0.0, 0.0]),
                (&disliked, [0.0, 1.0, 0.0]),
                (&near_liked, [0.9, 0.1, 0.0]),
                (&near_disliked, [0.1, 0.9, 0.0]),
                (&between, [0.6, 0.4, 0.1]),
                (&private, [0.95, 0.05, 0.0]),
            ],
        )
        .await;
        let descriptor = <Note as Searchable>::descriptor();
        let guest = suprnova::apply_scope(descriptor, &Viewer::Guest, None).unwrap();
        let toward = engine
            .recommend(
                descriptor,
                &id,
                &VectorRecommend {
                    positive: vec![liked.id.to_string()],
                    negative: vec![],
                    filter: guest.clone(),
                    depth: 10,
                },
            )
            .await
            .unwrap();
        let toward = keys(&toward);
        assert_eq!(toward.first(), Some(&near_liked.id.to_string()));
        assert!(!toward.contains(&liked.id.to_string()), "an input is never returned");
        assert!(!toward.contains(&private.id.to_string()), "the scope filter applies");
        let away = engine
            .recommend(
                descriptor,
                &id,
                &VectorRecommend {
                    positive: vec![liked.id.to_string()],
                    negative: vec![disliked.id.to_string()],
                    filter: guest,
                    depth: 10,
                },
            )
            .await
            .unwrap();
        let away = keys(&away);
        assert!(!away.contains(&disliked.id.to_string()));
        assert_eq!(away.last(), Some(&near_disliked.id.to_string()));
        let at = |list: &[String], note: &Note| list.iter().position(|key| *key == note.id.to_string());
        // The negative pushes the in-between note down relative to the plain query.
        assert!(at(&away, &between) <= at(&toward, &between));
    })
    .await;
}

#[tokio::test]
async fn rec_006_discover_ranks_by_context_pairs_before_nearness_to_the_target() {
    setup(async {
        let target = note("Target", "x", true, "o", 1).await;
        let positive = note("Positive", "x", true, "o", 2).await;
        let negative = note("Negative", "x", true, "o", 3).await;
        let positive_side = note("Positive side", "x", true, "o", 4).await;
        let negative_side = note("Negative side", "x", true, "o", 5).await;
        let engine = suprnova::CollectionEngine::new();
        let id = generation(
            &engine,
            &[
                (&target, [1.0, 0.0, 0.0]),
                (&positive, [0.0, 0.0, 1.0]),
                (&negative, [0.0, 1.0, 0.0]),
                // Nearer the target, but on the negative side of the pair.
                (&negative_side, [0.9, 0.3, 0.0]),
                (&positive_side, [0.8, 0.0, 0.4]),
            ],
        )
        .await;
        let found = engine
            .discover(
                <Note as Searchable>::descriptor(),
                &id,
                &VectorDiscover {
                    target: target.id.to_string(),
                    context: vec![(positive.id.to_string(), negative.id.to_string())],
                    filter: None,
                    depth: 2,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            keys(&found),
            [positive_side.id.to_string(), negative_side.id.to_string()]
        );
        assert!(found.candidates[0].score >= 1.0 && found.candidates[1].score < 1.0);
    })
    .await;
}

#[tokio::test]
async fn rec_006_an_engine_that_keeps_no_vectors_cannot_recommend() {
    setup(async {
        let descriptor = <Note as Searchable>::descriptor();
        let refused = NullEngine::new()
            .recommend(
                descriptor,
                &GenerationId::new("none").unwrap(),
                &VectorRecommend {
                    positive: vec!["1".into()],
                    depth: 5,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("keeps no vectors"));
    })
    .await;
}
