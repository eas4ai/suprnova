use suprnova::search::{EmbedInput, Embedder, embedder::http::HttpEmbedder};
#[tokio::test]
#[ignore = "requires SEARCH_EMBED_BASE_URL, SEARCH_EMBED_MODEL and SEARCH_EMBED_API_KEY"]
async fn configured_endpoint_embeds_equal_texts_consistently() {
    let embedder = HttpEmbedder::from_env().await.unwrap();
    let input = EmbedInput {
        text: "Rust search embeddings".into(),
        ..Default::default()
    };
    let vectors = embedder
        .embed_documents(&[input.clone(), input])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].dense.len(), embedder.profile().dimensions);
    assert_eq!(vectors[0].dense.len(), vectors[1].dense.len());
    let dot: f64 = vectors[0]
        .dense
        .iter()
        .zip(&vectors[1].dense)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    assert!((dot / (norm(&vectors[0].dense) * norm(&vectors[1].dense)) - 1.).abs() < 1e-5);
}
