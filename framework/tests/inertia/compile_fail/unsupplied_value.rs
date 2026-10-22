//! A root template that names a value the framework does not supply
//! (`author`) must not compile (RDOC-001).

#[suprnova::inertia_root(path = "inertia/unsupplied_value.html")]
pub struct AppDocument;

fn main() {}
