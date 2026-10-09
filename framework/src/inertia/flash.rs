//! Per-request flash data.
//!
//! Inertia v3's `page.flash` field carries one-shot data - toasts,
//! success messages, newly-created IDs - that should appear on the
//! current page but not persist across navigations.
//!
//! ## Storage model
//!
//! Flash data lives in a `tokio::task_local!` set up at the request
//! boundary by `Server::handle_request`. Within a request, anywhere
//! that can `.await` can call [`App::flash`](crate::App::flash) to
//! push values; [`InertiaResponse::resolve`](crate::InertiaResponse::resolve)
//! drains the bag at response build time and emits the contents under
//! the top-level `flash` field of the page object.
//!
//! `task_local!` (rather than `thread_local!`) is the correct primitive
//! for per-request state under Tokio: the binding follows the task
//! across `.await` points even when the runtime moves it to a different
//! worker thread. The thread-local InertiaContext bug we fixed in Tier 0
//! is exactly the kind of problem this avoids.
//!
//! ## Inertia flash data in the session
//!
//! Laravel's `Inertia::flash` keeps its data in the session under
//! `inertia.flash_data`, emits it as `page.flash` and pulls it when a page
//! renders. Suprnova does the same: [`App::flash`](crate::App::flash),
//! [`Inertia::flash`](crate::Inertia::flash) and
//! [`InertiaResponse::flash`](crate::InertiaResponse::flash) write into
//! that one session entry (as a session flash, so it is
//! `_flash.new.inertia.flash_data` until the next request ages it), and
//! [`InertiaResponse::resolve`](crate::InertiaResponse::resolve) emits and
//! removes it once the whole page is built. The data does not depend on
//! the response of the request that set it, and the Inertia middleware
//! keeps it for one more request whenever the response is a redirect, so it
//! survives any number of redirects before a page shows it. Without a
//! session in scope the values go to the task-local bag above and appear on
//! the current response only.
//!
//! Other session flash values (`Redirect::with`) still appear under
//! `page.flash` on the page after the redirect, merged below the Inertia
//! flash data.
//!
//! ### Precedence on key collision
//!
//! Same-request flash (task-local bag + builder) wins over session
//! `_flash.old.*` so a destination handler can override an inherited
//! value just by re-flashing the same key.
//!
//! ### Internal session keys are filtered
//!
//! Session flash is shared with the framework's own one-shot signals
//! (`_old_input` for form repopulation, `_inertia.*` for protocol
//! flags). Only user-visible keys are surfaced to `page.flash` - keys
//! prefixed with `_` are filtered out.
//!
//! ## History flags
//!
//! `clear_history` and `preserve_fragment` set for a later page are plain
//! session entries, `inertia.clear_history` and `inertia.preserve_fragment`,
//! as Laravel's are: they last until a page emits them, however many
//! redirects come first. A one-request flash let a logout followed by two
//! redirects render the next page without `clearHistory`, leaving private
//! pages decryptable in the history.

use crate::lock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

tokio::task_local! {
    /// Per-request flash bag. Scoped by `Server::handle_request`.
    pub(crate) static FLASH_BAG: Arc<Mutex<HashMap<String, Value>>>;
}

tokio::task_local! {
    /// Per-request history-encryption flag set by
    /// [`EncryptHistoryMiddleware`](crate::inertia::EncryptHistoryMiddleware).
    /// Read by `InertiaResponse::resolve` alongside the per-response
    /// override and the config default. See the v3 history-encryption
    /// docs for protocol details.
    pub(crate) static ENCRYPT_HISTORY: bool;
}

/// Whether the active request has been marked for history encryption
/// by [`EncryptHistoryMiddleware`]. Returns `None` when no middleware
/// has set the flag; the caller should fall back to the config default.
pub(crate) fn encrypt_history_flag() -> Option<bool> {
    ENCRYPT_HISTORY.try_with(|b| *b).ok()
}

/// A key Inertia flash data can be stored under.
///
/// Laravel's `Inertia::flash` takes a string or an enum case (a backed
/// enum's value, a unit enum's name), so a Laravel app that names its toast
/// kinds with an enum flashes with the case itself. Implement this for such
/// a type; `&str`, `String` and `&String` implement it already.
///
/// ```rust
/// use suprnova::FlashKey;
///
/// enum Toast {
///     Success,
///     Warning,
/// }
///
/// impl FlashKey for Toast {
///     fn flash_key(&self) -> String {
///         match self {
///             Toast::Success => "success".to_string(),
///             Toast::Warning => "warning".to_string(),
///         }
///     }
/// }
/// ```
pub trait FlashKey {
    /// The key the value is stored under in `page.flash`.
    fn flash_key(&self) -> String;
}

impl FlashKey for &str {
    fn flash_key(&self) -> String {
        (*self).to_string()
    }
}

impl FlashKey for String {
    fn flash_key(&self) -> String {
        self.clone()
    }
}

impl FlashKey for &String {
    fn flash_key(&self) -> String {
        (*self).clone()
    }
}

/// Session key of the Inertia flash data, Laravel's
/// `SessionKey::FLASH_DATA`.
pub(crate) const FLASH_DATA: &str = "inertia.flash_data";

/// Session key of the pending clear-history flag, Laravel's
/// `SessionKey::CLEAR_HISTORY`.
pub(crate) const CLEAR_HISTORY: &str = "inertia.clear_history";

/// Session key of the pending preserve-fragment flag, Laravel's
/// `SessionKey::PRESERVE_FRAGMENT`.
pub(crate) const PRESERVE_FRAGMENT: &str = "inertia.preserve_fragment";

/// Where a flash written in this request sits until the next one ages it.
pub(crate) fn flash_data_new_key() -> String {
    format!("_flash.new.{FLASH_DATA}")
}

/// Where a flash written by the previous request sits.
pub(crate) fn flash_data_old_key() -> String {
    format!("_flash.old.{FLASH_DATA}")
}

/// The Inertia flash data `session` holds: what the previous request left,
/// overlaid with what this one wrote.
fn flash_data_in(session: &crate::session::SessionData) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for key in [flash_data_old_key(), flash_data_new_key()] {
        if let Some(Value::Object(map)) = session.data.get(&key) {
            out.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    out
}

/// Merge `entries` into the session's Inertia flash data, kept for the
/// next request. Returns `false`, writing nothing, when no session is in
/// scope.
///
/// As Laravel's `flash` does, the whole merged map is flashed again, so
/// what an earlier request left survives with what this one adds.
pub(crate) fn put_in_session(entries: serde_json::Map<String, Value>) -> bool {
    crate::session::session_mut(|session| {
        let mut merged = flash_data_in(session);
        merged.extend(entries);
        session.data.remove(&flash_data_old_key());
        session.flash(FLASH_DATA, Value::Object(merged));
    })
    .is_some()
}

/// The Inertia flash data in the session in scope, empty without one.
pub(crate) fn get_from_session() -> serde_json::Map<String, Value> {
    crate::session::session()
        .map(|session| flash_data_in(&session))
        .unwrap_or_default()
}

/// Remove and return the Inertia flash data in the session in scope.
pub(crate) fn pull_from_session() -> serde_json::Map<String, Value> {
    crate::session::session_mut(|session| {
        let data = flash_data_in(session);
        session.forget(&flash_data_old_key());
        session.forget(&flash_data_new_key());
        data
    })
    .unwrap_or_default()
}

/// Keep the Inertia flash data the previous request left for one more
/// request - Laravel's `Middleware::reflash`, run when the response is a
/// redirect, so the data reaches the page at the end of a chain of
/// redirects instead of expiring on the way.
pub(crate) fn reflash_for_redirect() {
    crate::session::session_mut(|session| {
        if session.data.contains_key(&flash_data_old_key()) {
            let merged = flash_data_in(session);
            session.data.remove(&flash_data_old_key());
            session.flash(FLASH_DATA, Value::Object(merged));
        }
    });
}

/// Set a history flag (`CLEAR_HISTORY` or `PRESERVE_FRAGMENT`) for the next
/// page response. Returns `false`, writing nothing, without a session.
pub(crate) fn set_history_flag(key: &str) -> bool {
    crate::session::session_mut(|session| session.put(key, true)).is_some()
}

/// Add a value to the Inertia flash data.
///
/// With a session in scope it is merged into the session's
/// `inertia.flash_data`, so it reaches the next page response whatever
/// this request answers. Without one it goes into the current request's
/// flash bag and appears on this request's page only; a no-op when there
/// is no flash scope either (e.g. called outside an HTTP handler in tests
/// that don't set up the scope).
///
/// **Poison policy** (Domain 20 audit D20-A): the per-request flash
/// `Mutex` is scoped to a single request and recreated on the next
/// one, so poison only affects the request that experienced the
/// upstream panic. On poison the push is dropped silently and a
/// `tracing::error!` is emitted - the request is already failing,
/// so silent loss matches the documented "no active scope" no-op.
pub fn push(key: impl Into<String>, value: Value) {
    let key = key.into();
    let mut entry = serde_json::Map::new();
    entry.insert(key.clone(), value.clone());
    if put_in_session(entry) {
        return;
    }
    let _ = FLASH_BAG.try_with(|bag| match lock::lock(bag, "inertia flash bag") {
        Ok(mut guard) => {
            guard.insert(key, value);
        }
        Err(_) => {
            tracing::error!(
                "Inertia flash bag lock poisoned; dropping push (the upstream \
                 panic that poisoned the lock is already converted to a 500 \
                 by the panic-catch middleware)."
            );
        }
    });
}

/// Drain the current request's flash bag into a JSON map. Returns an
/// empty map when no scope is active. Called by
/// [`InertiaResponse::resolve`](crate::InertiaResponse::resolve) when
/// assembling the page object.
///
/// **Poison policy** (Domain 20 audit D20-A): on per-request Mutex
/// poison the drain returns an empty map and logs at `error` level.
/// Same per-request-scoped reasoning as [`push`].
pub fn drain() -> serde_json::Map<String, Value> {
    FLASH_BAG
        .try_with(|bag| match lock::lock(bag, "inertia flash bag") {
            Ok(mut guard) => {
                let entries = std::mem::take(&mut *guard);
                entries.into_iter().collect()
            }
            Err(_) => {
                tracing::error!("Inertia flash bag lock poisoned; returning empty drain.");
                serde_json::Map::new()
            }
        })
        .unwrap_or_default()
}

/// Create a fresh flash bag suitable for scoping into [`FLASH_BAG`].
///
/// Used by [`Server::handle_request`] when wrapping each request in
/// the flash scope.
pub(crate) fn new_bag() -> Arc<Mutex<HashMap<String, Value>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Bridge the per-request flash bag into the session's Inertia flash data
/// so the values survive an outgoing redirect.
///
/// Called by `From<Redirect> for Response` immediately before the HTTP
/// response is built. [`push`] writes to the session directly whenever one
/// is in scope, so the bag only holds values pushed before a session scope
/// existed; this moves them into `inertia.flash_data`, which the page after
/// the redirect emits under `flash`.
///
/// No-op when no session scope is active (e.g. the route is outside
/// the session middleware) - the values remain in the task-local bag
/// and still appear on the *current* response via [`drain`], but they
/// cannot persist past the redirect because there is no session to
/// persist them into.
///
/// **Move semantics**: the task-local bag is drained on transfer so a
/// redirect handler that returns a non-redirect response after calling
/// [`push`] still sees the values in [`drain`]. The double-drain risk
/// only applies on the redirect path, where the drained values are
/// transferred to the session and the current response is discarded by
/// the client following the `Location` header.
pub fn transfer_to_session() {
    if !has_pending() {
        return;
    }
    // Drain only once a session is known to be there: draining first and
    // then finding none would drop the values the doc above promises stay
    // in the bag.
    if crate::session::session_mut(|_| ()).is_some() {
        put_in_session(drain());
    }
}

/// Whether the current request's flash bag holds anything, without
/// draining it. Checked before [`transfer_to_session`] opens the session,
/// because opening it records a session read and an empty bag has nothing
/// to move.
fn has_pending() -> bool {
    FLASH_BAG
        .try_with(|bag| match lock::lock(bag, "inertia flash bag") {
            Ok(guard) => !guard.is_empty(),
            Err(_) => false,
        })
        .unwrap_or(false)
}

/// Collect the receiving request's session `_flash.old.*` entries
/// that should surface under the page object's top-level `flash`
/// field.
///
/// Filters out internal session keys (anything `_`-prefixed) so the
/// `_old_input` form-repopulation bag and the `_inertia.*` protocol
/// flags don't leak to the client. The unprefixed `_old_input` itself
/// is also filtered as belt-and-suspenders against a future move of
/// the constant. The Inertia flash data is left out too: it is emitted
/// entry by entry, not as one `inertia.flash_data` key.
///
/// Returns an empty map outside a `SessionMiddleware` scope.
pub fn drain_session_flash_for_page() -> serde_json::Map<String, Value> {
    crate::session::session()
        .map(|s| {
            let mut out = serde_json::Map::new();
            for (key, value) in &s.data {
                let Some(name) = key.strip_prefix("_flash.old.") else {
                    continue;
                };
                if name.starts_with('_') || name == FLASH_DATA {
                    continue;
                }
                out.insert(name.to_string(), value.clone());
            }
            out
        })
        .unwrap_or_default()
}
