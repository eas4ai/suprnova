//! `suprnova::fake`: what a factory that is written by hand needs of the
//! `fake` crate. That an application reaches it with no `fake` among its
//! own dependencies is shown in `app/tests/fake_without_a_dependency.rs`:
//! this crate has the dependency.

use suprnova::fake::faker::internet::en::SafeEmail;
use suprnova::fake::faker::lorem::en::Sentence;
use suprnova::fake::faker::name::en::Name;
use suprnova::fake::rand::SeedableRng;
use suprnova::fake::rand::rngs::StdRng;
use suprnova::{Dummy, Fake, Faker};

#[derive(Debug, Dummy, PartialEq)]
struct Row {
    id: u32,
    note: String,
}

#[test]
fn the_fakers_are_the_fakers_of_the_traits_at_the_crate_root() {
    // `Fake` is the trait of the crate root, and the fakers are the ones
    // of `suprnova::fake`. Were they of two versions of the crate, the
    // fakers would not implement the trait.
    let email: String = SafeEmail().fake();
    let name: String = Name().fake();
    let sentence: String = Sentence(3..6).fake();

    assert!(email.contains('@'), "{email}");
    assert!(!name.is_empty());
    assert!(sentence.split_whitespace().count() >= 3, "{sentence}");
}

#[test]
fn a_seeded_generator_gives_the_same_values() {
    let first: (String, Row) = {
        let mut rng = StdRng::seed_from_u64(7);
        (
            SafeEmail().fake_with_rng(&mut rng),
            Faker.fake_with_rng(&mut rng),
        )
    };
    let second: (String, Row) = {
        let mut rng = StdRng::seed_from_u64(7);
        (
            SafeEmail().fake_with_rng(&mut rng),
            Faker.fake_with_rng(&mut rng),
        )
    };

    assert_eq!(first, second);
}
