//! User model.
//!
//! Defined with the `#[suprnova::model]` macro: the struct below *is* the
//! Eloquent model. The macro emits the SeaORM `Entity` / `Column` /
//! `ActiveModel` in an inner `user` module and gives `User` the query surface
//! (`User::query()`, `User::find()`, the `Model::create` mass-assignment entry
//! point, `save`, timestamps). `Authenticatable` is implemented on the struct
//! so the auth stack (session middleware, user providers, `Auth::user()`)
//! resolves users without touching SeaORM directly.

use std::any::Any;

use chrono::{DateTime, Utc};
use suprnova::{
    attrs, hashing, model, Authenticatable, CanResetPassword, FrameworkError, MustVerifyEmail,
};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
    // The users migration creates these as DATETIME on MySQL and `timestamp`
    // on Postgres, nullable columns without a time zone that hold the UTC
    // wall clock, as the Laravel 13 skeleton's are. A `DateTime<Utc>` field
    // defaults to the text cast `AsDateTime`, which those columns refuse, so
    // each names the cast for its column. The naive casts also read the
    // `TIMESTAMP` columns Laravel's own migration creates on MySQL.
    casts = {
        email_verified_at = suprnova::AsOptionalNaiveDateTime,
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
    // `notes()` reaches the notes this user wrote through `notes.user_id`,
    // the column the convention names for a `HasMany<Note>` on `User`.
    relations = {
        notes: HasMany<crate::models::note::Note>,
    },
)]
pub struct User {
    // `BIGINT UNSIGNED` on MySQL, as Laravel's `id()` creates it.
    pub id: u64,
    pub name: String,
    pub email: String,
    pub password: String,
    // Laravel's remember-me token. This application never writes it, so a
    // Laravel application on the same database keeps its own remember-me.
    pub remember_token: Option<String>,
    // Nullable verification timestamp. `NULL` means unverified.
    pub email_verified_at: Option<DateTime<Utc>>,
    // Nullable, as Laravel's are: a row Laravel wrote may hold neither.
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// Re-export the SeaORM types the macro emits in the inner `user` module so
// call sites referencing `crate::models::user::{Entity, Column, ActiveModel}`
// keep resolving.
pub use user::{ActiveModel, Column, Entity};

impl User {
    /// Find a user by their email address.
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    /// Verify a plaintext password against this user's stored hash.
    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    /// Create a new user, hashing the password before insert. Values are
    /// mass-assigned through the model's `fillable` set.
    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let name: String = name.into();
        let email: String = email.into();
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name,
            email: email,
            password: hashed,
        })
        .await
    }
}

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn auth_identifier_name(&self) -> &'static str {
        "id"
    }

    // Returning the stored hash is what lets the registered
    // `EloquentUserProvider<User>` verify credentials. The trait default
    // returns `None` (for models that authenticate by other means -
    // OAuth, passkeys), which makes `Auth::attempt` reject every
    // password.
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(
        self: std::sync::Arc<Self>,
    ) -> std::sync::Arc<dyn Any + Send + Sync> {
        self
    }
}

// The email-verification flow reads the address + verification timestamp
// through this trait, and writes the timestamp back when a verification link
// is consumed. Implementing it on `User` is what lets the configured
// `EloquentUserProvider<User>` (registered in `bootstrap.rs`) drive
// `EmailVerification::resend` / `verify`.
impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
        self.email_verified_at = v;
    }

    fn name(&self) -> Option<&str> {
        Some(&self.name)
    }
}

// The password-reset flow addresses the reset / password-changed mail through
// `email_for_reset()` and persists the rotated (already-hashed) password
// through `set_password_hash()`. Implementing it on `User` lets the configured
// `EloquentUserProvider<User>` drive `PasswordReset::send_link` / `complete`.
impl CanResetPassword for User {
    fn email_for_reset(&self) -> &str {
        &self.email
    }

    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}
