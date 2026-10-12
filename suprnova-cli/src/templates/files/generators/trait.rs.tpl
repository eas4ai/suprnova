//! The `{name}` trait.

/// Behaviour the types that implement `{name}` share. A default method
/// gives every implementor the same body, and an implementor can override
/// it.
pub trait {name} {
    /// The implementing type's name, for logs and messages.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
