//! `suprnova::fake` in a crate that has no `fake` among its own
//! dependencies, which this application is. The tests of the framework
//! cannot show it: the framework has the dependency, so `::fake` is a
//! name there.

use suprnova::fake::faker::internet::en::SafeEmail;
use suprnova::fake::faker::name::en::Name;
use suprnova::fake::rand::SeedableRng;
use suprnova::fake::rand::rngs::StdRng;
use suprnova::{Dummy, Fake, Faker};

/// The derive is told where the crate is. Without the attribute this
/// does not compile here.
#[derive(Debug, Dummy, PartialEq)]
#[dummy(crate_name = "suprnova::fake")]
struct Row {
    id: u32,
    note: String,
    #[dummy(faker = "SafeEmail()")]
    email: String,
}

#[test]
fn a_struct_derives_dummy_with_the_crate_of_the_framework() {
    let row: Row = Faker.fake();

    assert!(row.email.contains('@'), "{row:?}");
}

#[test]
fn the_fakers_are_the_fakers_of_the_traits_at_the_crate_root() {
    // `Fake` is the trait of the crate root and the fakers are the ones
    // of `suprnova::fake`. Were they of two versions of the crate, the
    // fakers would not implement the trait.
    let email: String = SafeEmail().fake();
    let name: String = Name().fake();

    assert!(email.contains('@'), "{email}");
    assert!(!name.is_empty());
}

#[test]
fn a_seeded_generator_gives_the_same_values() {
    let values = || {
        let mut rng = StdRng::seed_from_u64(7);
        let email: String = SafeEmail().fake_with_rng(&mut rng);
        let row: Row = Faker.fake_with_rng(&mut rng);
        (email, row)
    };

    assert_eq!(values(), values());
}
