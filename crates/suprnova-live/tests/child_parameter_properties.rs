//! Property coverage for canonical child-parameter capabilities.

mod child_parameter_support;

use child_parameter_support::{NOW, issued_child};
use proptest::prelude::*;
use suprnova_live::child::verify_child_parameters;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn canonical_child_capabilities_round_trip_deterministically(value in "[a-z0-9]{1,16}") {
        let query = format!("q-{value}");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let first = runtime.block_on(issued_child(&query));
        let verified = verify_child_parameters(
            &first.encoded,
            &first.expected,
            &first.keys,
            NOW,
            &first.limits,
        )
        .expect("generated capability verifies");
        let second = runtime.block_on(issued_child(&query));

        prop_assert_eq!(verified.parameters(), &first.parameters);
        prop_assert_eq!(&first.encoded, &second.encoded);

        let mut tampered = first.encoded.clone();
        let index = value.len() % tampered.len();
        tampered[index] ^= 1;
        let result = std::panic::catch_unwind(|| {
            verify_child_parameters(
                &tampered,
                &first.expected,
                &first.keys,
                NOW,
                &first.limits,
            )
        });
        prop_assert!(result.is_ok());
        prop_assert!(result.expect("verification did not panic").is_err());
    }

    #[test]
    fn arbitrary_bounded_bytes_never_panic_or_expose_parameters(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let fixture = runtime.block_on(issued_child("property"));
        let result = std::panic::catch_unwind(|| {
            verify_child_parameters(
                &bytes,
                &fixture.expected,
                &fixture.keys,
                NOW,
                &fixture.limits,
            )
        });
        prop_assert!(result.is_ok());
    }
}
