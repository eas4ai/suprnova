//! IMG-008: `ImageConfig` is `#[non_exhaustive]`, so a struct literal
//! outside the crate does not compile, even one that fills the rest from
//! the default.

fn main() {
    let _ = suprnova::ImageConfig {
        max_dimension: 1024,
        ..suprnova::ImageConfig::default()
    };
}
