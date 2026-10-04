//! Session-only Live component fields, read from and written to the
//! Suprnova session.
//!
//! A `#[session]` field never enters the signed snapshot. Its generated
//! code calls [`load`] when the component is mounted or reconstructed for
//! one viewer, and [`stage`] before dehydration. A staged value is not
//! written at once: the Live endpoint and the document mount hold the
//! values the request staged and write them only after the outcome is
//! accepted, so an action that fails, or a commit that fails after it,
//! leaves the session as it was.
//!
//! A value lives under one session key per component name and field. The
//! component grammar has no `#`, so `#` keeps the two apart.

use std::cell::RefCell;
use std::future::Future;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use suprnova_live::component::ComponentError;

tokio::task_local! {
    /// Values staged by the request's components, in staging order.
    static STAGED: RefCell<Vec<(String, Value)>>;
}

/// The session key one component field is stored under.
fn session_key(component: &str, field: &str) -> String {
    format!("_live_session.{component}#{field}")
}

/// The value the current session holds for `field` of `component`, or
/// `None` when there is no session, no value, or a value that no longer
/// decodes as `T` (written before the field changed its type). The field
/// then keeps the value its mount or its default gave it.
pub fn load<T: DeserializeOwned>(component: &str, field: &str) -> Option<T> {
    crate::session::session()?.get(&session_key(component, field))
}

/// Stage the value of `field` of `component` for the session.
///
/// Inside a request that accepts outcomes ([`scope`]), the value is written
/// when the outcome is accepted. Outside one, it is written at once.
///
/// # Errors
///
/// An application failure when the value does not serialize to JSON.
pub fn stage<T: Serialize>(component: &str, field: &str, value: &T) -> Result<(), ComponentError> {
    let value = serde_json::to_value(value).map_err(|_| ComponentError::application_failure())?;
    let mut entry = Some((session_key(component, field), value));
    // Outside a scope `try_with` fails and the entry is still here.
    let _ = STAGED.try_with(|staged| {
        if let Some(entry) = entry.take() {
            staged.borrow_mut().push(entry);
        }
    });
    if let Some(entry) = entry {
        write(vec![entry]);
    }
    Ok(())
}

/// Run `future` with a staging area, and return what it staged beside its
/// output. The caller passes the values to [`commit`] once the outcome is
/// accepted, and drops them otherwise.
pub(crate) async fn scope<F: Future>(future: F) -> (F::Output, Vec<(String, Value)>) {
    STAGED
        .scope(RefCell::new(Vec::new()), async move {
            let output = future.await;
            let staged = STAGED.with(|staged| staged.take());
            (output, staged)
        })
        .await
}

/// Write accepted session values. A value equal to the stored one is not
/// written again, so a request that changed nothing leaves the session
/// clean.
pub(crate) fn commit(staged: Vec<(String, Value)>) {
    if !staged.is_empty() {
        write(staged);
    }
}

fn write(values: Vec<(String, Value)>) {
    crate::session::session_mut(|session| {
        for (key, value) in values {
            if session.get::<Value>(&key).as_ref() != Some(&value) {
                session.put(&key, value);
            }
        }
    });
}
