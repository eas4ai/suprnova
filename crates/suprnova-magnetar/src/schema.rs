//! Application-owned entity bindings for Magnetar persistence.
//!
//! Magnetar deliberately does not define tables, columns, or migrations. An
//! application supplies one [`AuthSchema`](crate::schema::AuthSchema) implementation whose descriptors
//! point at its generated SeaORM entities.

/// Linked-account field accessors.
pub mod account;
/// Ceremony field accessors.
pub mod ceremony;
/// Shared descriptor contract.
pub mod entity;
/// Lockout field accessors.
pub mod lockout;
/// Passkey field accessors.
pub mod passkey;
/// Token broker provider-token-record field accessors.
pub mod provider_token;
/// Session field accessors.
pub mod session;
/// Token and token-record field accessors.
pub mod token;
/// User field accessors.
pub mod user;

pub use account::LinkedAccountFields;
pub use ceremony::CeremonyFields;
pub use entity::EntityBinding;
pub use lockout::LockoutFields;
pub use passkey::PasskeyFields;
pub use provider_token::ProviderTokenFields;
pub use session::SessionFields;
pub use token::{TokenFields, TokenRecordFields};
pub use user::{
    NOT_NULL_PASSWORD_EMPTY_SENTINEL, SessionEpoch, UserBinding, UserFields, UserOptionalFields,
    password_hash_for_verifier,
};

/// The complete set of application-owned entities used by authentication.
///
/// The associated types are descriptors, not concrete tables owned by
/// Magnetar. Every SQL identifier therefore remains in the application's
/// generated binding code.
pub trait AuthSchema: Send + Sync + 'static {
    /// The application's user entity.
    type User: EntityBinding;
    /// The application's session entity.
    type Session: EntityBinding;
    /// The application's linked-account entity.
    type LinkedAccount: EntityBinding;
    /// The application's passkey entity.
    type Passkey: EntityBinding;
    /// The application's short-lived token entity.
    type Token: EntityBinding;
    /// The application's ceremony entity.
    type Ceremony: EntityBinding;
    /// The application's lockout entity.
    type Lockout: EntityBinding;
    /// The application's token-record entity.
    type TokenRecord: EntityBinding;
}

/// [`AuthSchema`] extended with the token broker's provider-token-record
/// entity (`docs/specs/suprnova-magnetar/11-token-broker.md`).
///
/// Kept separate from [`AuthSchema`] itself rather than adding another
/// mandatory associated type there: every existing [`AuthSchema`]
/// implementation (every plugin's generic bound) would otherwise be forced
/// to name a broker table it never uses. Only the token broker's own
/// storage seam ([`crate::storage::ProviderTokenStore`]) and its callers
/// require this trait.
pub trait BrokerSchema: AuthSchema {
    /// The application's provider-token-record entity.
    type ProviderToken: EntityBinding;
}
