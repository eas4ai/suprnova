//! The `{name}` contract.

/// What a `{name}` must provide. Every method is required: the contract
/// names the work, and each implementor does it. Bind an implementor in
/// the container with `App::bind::<dyn {name}>(Arc::new(implementor))`.
pub trait {name}: Send + Sync {
    /// The implementor's name, for logs and messages.
    fn name(&self) -> &str;
}
