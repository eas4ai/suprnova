//! Property contracts for typed model codec round-trips and mutation atomicity.

use proptest::prelude::*;
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::snapshot::state::FieldCategory;
use suprnova_live::state::{
    ModelBindingSchema, ModelCodec, ModelFieldBinding, ModelPath, ProposalApplication,
    ProposalBatch, ProposalLimits, RawModelProposal,
};

fn integer_schema() -> ModelBindingSchema {
    ModelBindingSchema::new(vec![
        ModelFieldBinding::new("value", FieldCategory::Model, ModelCodec::I64)
            .expect("integer binding"),
    ])
    .expect("integer schema")
}

proptest! {
    #[test]
    fn signed_integer_codec_round_trips_losslessly(value in any::<i64>()) {
        let codec = ModelCodec::I64;
        let encoded = codec.encode(&value, &ProposalLimits::default().input_limits())
            .expect("integer encodes");
        let decoded: i64 = codec.decode(&encoded, &ProposalLimits::default().input_limits())
            .expect("integer decodes");
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn unsigned_string_and_list_codecs_round_trip_losslessly(
        unsigned in any::<u64>(),
        text in ".{0,128}",
        flags in proptest::collection::vec(any::<bool>(), 0..64),
    ) {
        let limits = ProposalLimits::default().input_limits();

        let encoded = ModelCodec::U64.encode(&unsigned, &limits).expect("u64 encodes");
        prop_assert_eq!(
            ModelCodec::U64.decode::<u64>(&encoded, &limits).expect("u64 decodes"),
            unsigned,
        );

        let encoded = ModelCodec::String.encode(&text, &limits).expect("string encodes");
        prop_assert_eq!(
            ModelCodec::String.decode::<String>(&encoded, &limits).expect("string decodes"),
            text,
        );

        let codec = ModelCodec::list(ModelCodec::Boolean);
        let encoded = codec.encode(&flags, &limits).expect("list encodes");
        prop_assert_eq!(
            codec.decode::<Vec<bool>>(&encoded, &limits).expect("list decodes"),
            flags,
        );
    }

    #[test]
    fn every_failed_integer_proposal_leaves_state_unchanged(
        original in any::<i64>(),
        invalid in "[^0-9-]{1,32}",
    ) {
        let path = ModelPath::parse("value").expect("model path");
        let batch = ProposalBatch::prepare(
            &integer_schema(),
            vec![RawModelProposal::new("value", CanonicalValue::String(invalid))],
            &ProposalLimits::default(),
        )
        .expect("invalid conversion is a field issue");
        let mut state = original;
        let outcome = batch.apply_required(&path, &mut state, |state, value: i64| *state = value);

        prop_assert!(matches!(outcome, ProposalApplication::Invalid(_)));
        prop_assert_eq!(state, original);
    }
}
