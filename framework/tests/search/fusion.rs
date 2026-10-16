use suprnova::search::fusion::{federated_rrf, normalize, paginate, weighted_rrf};

#[test]
fn reciprocal_rank_fusion_rewards_agreement_with_requested_weights() {
    let ranked = weighted_rrf(&[
        (0.6, vec!["a".into(), "b".into()]),
        (0.4, vec!["b".into(), "c".into()]),
    ]);
    assert_eq!(ranked[0].0, "b");
    assert!((ranked[0].1 - (0.6 / 62.0 + 0.4 / 61.0)).abs() < 1e-12);
}

#[test]
fn one_leg_preserves_order_and_ignores_duplicates_and_invalid_weights() {
    let ranked = weighted_rrf(&[
        (1.0, vec!["z".into(), "a".into(), "z".into()]),
        (0.0, vec!["x".into()]),
        (f64::NAN, vec!["y".into()]),
    ]);
    assert_eq!(
        ranked
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    assert_eq!(ranked[0].1, 1.0 / 61.0);
}

#[test]
fn normalization_is_local_and_rejects_non_finite_native_scores() {
    assert_eq!(
        normalize(&[
            ("a".into(), 4.0),
            ("b".into(), 2.0),
            ("c".into(), -1.0),
            ("d".into(), f64::INFINITY)
        ]),
        vec![
            ("a".into(), 1.0),
            ("b".into(), 0.5),
            ("c".into(), 0.0),
            ("d".into(), 0.0)
        ]
    );
    assert_eq!(normalize(&[("a".into(), 0.0)])[0].1, 0.0);
}

#[test]
fn equal_rank_ties_use_keys_and_federated_ties_use_index_order() {
    let ranked = weighted_rrf(&[(1.0, vec!["b".into()]), (1.0, vec!["a".into()])]);
    assert_eq!(ranked[0].0, "a");
    let federated = federated_rrf(&[
        ("second".into(), vec!["same".into(), "x".into()]),
        ("first".into(), vec!["same".into()]),
    ]);
    assert_eq!(
        federated
            .iter()
            .map(|hit| hit.index.as_str())
            .collect::<Vec<_>>(),
        ["second", "first", "second"]
    );
}

#[test]
fn pagination_slices_without_overlap_and_saturates_overflow() {
    assert_eq!(paginate(&[1, 2, 3, 4], 0, 2), [1, 2]);
    assert_eq!(paginate(&[1, 2, 3, 4], 2, 2), [3, 4]);
    assert!(paginate(&[1, 2], usize::MAX, usize::MAX).is_empty());
    assert_eq!(paginate(&[1, 2], 1, usize::MAX), [2]);
}
