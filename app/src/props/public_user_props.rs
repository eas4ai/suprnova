//! Props for the two **unauthenticated** user routes — `GET /users` and
//! `GET /users/{id}` — plus the public cursor listing at `GET /api/users`.
//!
//! These exist as separate types from [`UserProps`](super::UserProps) for
//! one reason: `UserProps` serialises `email`, and none of these three
//! routes requires a session. Whatever they emit is world-readable.
//!
//! That distinction is invisible while the routes serve fixtures — a
//! hardcoded `user-001@example.com` leaks nothing. It stops being
//! invisible the moment they read the real `users` table, which is
//! exactly what the benchmark work does to them. Against a seeded table
//! that is every address in the database behind one unauthenticated
//! cursor, paged out 20 at a time by anyone who asks.
//!
//! So the public projection is id + name. Email stays on `UserProps` and
//! on the JSON:API resources under `/api/users/{id}` and `/api/v3/users`,
//! which are session-gated precisely because `UserResource` carries it.
//!
//! Both types are outbound-only, so they derive `InertiaProps` (which
//! emits `Serialize` and feeds `suprnova generate-types`) rather than
//! `Data` — there is no inbound body to deserialise or validate.

use suprnova::InertiaProps;

/// One row of the public user directory.
#[derive(Debug, Clone, InertiaProps)]
pub struct PublicUserProps {
    pub id: i64,
    pub name: String,
}

/// A single user's public detail page.
///
/// `bio` is `Option` because the relation is a `HasOne`: a user may have
/// no profile row, and the eager loader reports that as `None` rather
/// than borrowing a neighbour's. Serving `null` here is the honest answer
/// — the alternative, substituting an empty string, would make "no
/// profile" and "empty profile" indistinguishable to the frontend.
#[derive(Debug, Clone, InertiaProps)]
pub struct UserDetailProps {
    pub id: i64,
    pub name: String,
    pub bio: Option<String>,
}
