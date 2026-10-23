//! The lifecycle events of document models, through the framework's
//! observer shape.
//!
//! Each event is one generic type here, `Created<User>` for a `User`
//! document, so a listener names the model in the type: register it with
//! [`EventFacade::listen`](crate::events::EventFacade::listen), or with
//! [`listen_cancellable`] for
//! the cancellable ones (`Saving`, `Creating`, `Updating`, `Deleting` and
//! `Restoring`), as for SQL models. A cancellable listener that answers
//! [`EventResult::cancel`] stops the write, and the call returns
//! [`FrameworkError::bad_request`] with the reason.
//!
//! [`DocumentObserver`] collects a model's callbacks in one impl. The
//! `#[suprnova::observer(User)]` attribute registers one at boot, as it
//! does for an SQL `Observer`, because `#[suprnova::document]` names the
//! event types in a `user::events` module; [`observe`] registers one by
//! hand.
//!
//! The cancellable events carry the attributes about to be written as a
//! [`Document`] behind a mutex, keyed by field name, the key under its
//! field name. A listener may change them; the write uses what it leaves.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use ::bson::Document;
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::document::DocumentModel;
use crate::eloquent::events::{
    CancellableListener, EventResult, dispatch_after_with, dispatch_cancellable_with,
    listen_cancellable,
};
use crate::error::FrameworkError;
use crate::events::{Event, EventFacade, Listener};

/// The attributes a cancellable event carries, shared with its listeners.
pub type SharedAttributes = Arc<Mutex<Document>>;

macro_rules! model_event {
    ($(#[$doc:meta])* $name:ident { $($(#[$field_doc:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$doc])*
        pub struct $name<D> {
            $($(#[$field_doc])* pub $field: $ty,)*
        }

        impl<D: DocumentModel> Clone for $name<D> {
            fn clone(&self) -> Self {
                Self { $($field: self.$field.clone(),)* }
            }
        }

        impl<D: DocumentModel> fmt::Debug for $name<D> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    $(.field(stringify!($field), &self.$field))*
                    .finish()
            }
        }

        impl<D: DocumentModel> Event for $name<D> {
            fn event_name() -> &'static str {
                std::any::type_name::<Self>()
            }
        }
    };
}

model_event! {
    /// Fires once before a query reads documents of `D`.
    Retrieving {
        /// The model type the query reads.
        document_type: PhantomData<fn() -> D>,
    }
}

model_event! {
    /// Fires for each document of `D` a query read.
    Retrieved {
        /// The document as read.
        model: D,
    }
}

model_event! {
    /// Fires before a document of `D` is inserted or updated. Cancellable.
    Saving {
        /// The attributes about to be written.
        attrs: SharedAttributes,
        /// `true` on an insert, `false` on an update.
        is_creating: bool,
        /// The model type being saved.
        document_type: PhantomData<fn() -> D>,
    }
}

model_event! {
    /// Fires before a document of `D` is inserted. Cancellable.
    Creating {
        /// The attributes about to be inserted.
        attrs: SharedAttributes,
        /// The model type being created.
        document_type: PhantomData<fn() -> D>,
    }
}

model_event! {
    /// Fires after a document of `D` was inserted.
    Created {
        /// The document as inserted.
        model: D,
    }
}

model_event! {
    /// Fires before a document of `D` is updated. Cancellable.
    Updating {
        /// The document before the update.
        previous: D,
        /// The attributes about to be written.
        attrs: SharedAttributes,
    }
}

model_event! {
    /// Fires after a document of `D` was updated.
    Updated {
        /// The document before the update.
        previous: D,
        /// The document as the server now holds it.
        current: D,
    }
}

model_event! {
    /// Fires after a document of `D` was inserted or updated.
    Saved {
        /// The document as written.
        model: D,
    }
}

model_event! {
    /// Fires before a document of `D` is deleted, or trashed when the
    /// model soft deletes. Cancellable.
    Deleting {
        /// The document about to be deleted.
        model: D,
        /// `true` for `force_delete`.
        is_force: bool,
    }
}

model_event! {
    /// Fires after a document of `D` was deleted or trashed.
    Deleted {
        /// The document as it was deleted, or as trashed.
        model: D,
        /// `true` for `force_delete`.
        is_force: bool,
    }
}

model_event! {
    /// Fires after a document of `D` was soft deleted.
    Trashed {
        /// The trashed document.
        model: D,
    }
}

model_event! {
    /// Fires before a trashed document of `D` is restored. Cancellable.
    Restoring {
        /// The trashed document.
        model: D,
    }
}

model_event! {
    /// Fires after a trashed document of `D` was restored.
    Restored {
        /// The restored document.
        model: D,
    }
}

model_event! {
    /// Fires before `force_delete` removes a document of `D`.
    ForceDeleting {
        /// The document about to be removed.
        model: D,
    }
}

model_event! {
    /// Fires after `force_delete` removed a document of `D`.
    ForceDeleted {
        /// The removed document.
        model: D,
    }
}

/// A document model's lifecycle callbacks in one impl, the shape of the
/// SQL [`Observer`](crate::eloquent::observers::Observer).
///
/// Every method has a no-op default, so an impl writes only the ones it
/// needs. The cancellable ones answer [`EventResult`]; the others answer a
/// `Result` whose error the write returns, although the write has already
/// happened.
///
/// ```rust,ignore
/// use suprnova::bson::Document;
/// use suprnova::{DocumentObserver, EventResult};
///
/// pub struct UserObserver;
///
/// #[suprnova::observer(User)]
/// #[suprnova::async_trait]
/// impl DocumentObserver<User> for UserObserver {
///     async fn creating(&self, attrs: &mut Document) -> EventResult {
///         attrs.insert("plan", "free");
///         EventResult::ok()
///     }
/// }
/// ```
#[async_trait]
pub trait DocumentObserver<D: DocumentModel>: Send + Sync + 'static {
    /// Before a query reads documents.
    async fn retrieving(&self) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// For each document a query read.
    async fn retrieved(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Before an insert or an update. `is_creating` says which.
    async fn saving(&self, _attrs: &mut Document, _is_creating: bool) -> EventResult {
        EventResult::ok()
    }

    /// Before an insert.
    async fn creating(&self, _attrs: &mut Document) -> EventResult {
        EventResult::ok()
    }

    /// Before an update. `previous` is the document before it.
    async fn updating(&self, _previous: &D, _attrs: &mut Document) -> EventResult {
        EventResult::ok()
    }

    /// Before a delete, or a trash when the model soft deletes.
    async fn deleting(&self, _model: &D, _is_force: bool) -> EventResult {
        EventResult::ok()
    }

    /// Before a trashed document is restored.
    async fn restoring(&self, _model: &D) -> EventResult {
        EventResult::ok()
    }

    /// After an insert.
    async fn created(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After an update.
    async fn updated(&self, _previous: &D, _current: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After an insert or an update.
    async fn saved(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After a delete or a trash.
    async fn deleted(&self, _model: &D, _is_force: bool) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After a soft delete.
    async fn trashed(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After a restore.
    async fn restored(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Before `force_delete` removes a document.
    async fn force_deleting(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// After `force_delete` removed a document.
    async fn force_deleted(&self, _model: &D) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Register `observer` for every event of `D`, as Laravel's
/// `User::observe(UserObserver::class)`. Calling it twice registers it
/// twice; `#[suprnova::observer]` registers once at boot instead.
pub async fn observe<D: DocumentModel, O: DocumentObserver<D>>(observer: O) {
    let adapter = Arc::new(ObserverAdapter {
        observer: Arc::new(observer),
    });
    EventFacade::listen::<Retrieving<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Retrieved<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Created<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Updated<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Saved<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Deleted<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Trashed<D>, _>(adapter.clone()).await;
    EventFacade::listen::<Restored<D>, _>(adapter.clone()).await;
    EventFacade::listen::<ForceDeleting<D>, _>(adapter.clone()).await;
    EventFacade::listen::<ForceDeleted<D>, _>(adapter.clone()).await;
    listen_cancellable::<Saving<D>, _>(adapter.clone()).await;
    listen_cancellable::<Creating<D>, _>(adapter.clone()).await;
    listen_cancellable::<Updating<D>, _>(adapter.clone()).await;
    listen_cancellable::<Deleting<D>, _>(adapter.clone()).await;
    listen_cancellable::<Restoring<D>, _>(adapter).await;
}

/// One registered observer, as the listener of each event.
struct ObserverAdapter<O> {
    observer: Arc<O>,
}

macro_rules! adapt {
    ($event:ident, |$observer:ident, $e:ident| $call:expr) => {
        #[async_trait]
        impl<D: DocumentModel, O: DocumentObserver<D>> Listener<$event<D>> for ObserverAdapter<O> {
            async fn handle(&self, $e: &$event<D>) -> Result<(), FrameworkError> {
                let $observer = &self.observer;
                $call
            }
        }
    };
}

adapt!(Retrieving, |observer, _event| observer.retrieving().await);
adapt!(Retrieved, |observer, event| observer
    .retrieved(&event.model)
    .await);
adapt!(Created, |observer, event| observer
    .created(&event.model)
    .await);
adapt!(Updated, |observer, event| observer
    .updated(&event.previous, &event.current)
    .await);
adapt!(Saved, |observer, event| observer.saved(&event.model).await);
adapt!(Deleted, |observer, event| observer
    .deleted(&event.model, event.is_force)
    .await);
adapt!(Trashed, |observer, event| observer
    .trashed(&event.model)
    .await);
adapt!(Restored, |observer, event| observer
    .restored(&event.model)
    .await);
adapt!(ForceDeleting, |observer, event| observer
    .force_deleting(&event.model)
    .await);
adapt!(ForceDeleted, |observer, event| observer
    .force_deleted(&event.model)
    .await);

macro_rules! adapt_cancellable {
    ($event:ident, |$observer:ident, $e:ident| $call:expr) => {
        #[async_trait]
        impl<D: DocumentModel, O: DocumentObserver<D>> CancellableListener<$event<D>>
            for ObserverAdapter<O>
        {
            async fn handle(&self, $e: &$event<D>) -> EventResult {
                let $observer = &self.observer;
                $call
            }
        }
    };
}

adapt_cancellable!(Saving, |observer, event| {
    let mut attrs = event.attrs.lock().await;
    observer.saving(&mut attrs, event.is_creating).await
});
adapt_cancellable!(Creating, |observer, event| {
    let mut attrs = event.attrs.lock().await;
    observer.creating(&mut attrs).await
});
adapt_cancellable!(Updating, |observer, event| {
    let mut attrs = event.attrs.lock().await;
    observer.updating(&event.previous, &mut attrs).await
});
adapt_cancellable!(Deleting, |observer, event| observer
    .deleting(&event.model, event.is_force)
    .await);
adapt_cancellable!(Restoring, |observer, event| observer
    .restoring(&event.model)
    .await);

// --- Dispatch, for the model's calls -------------------------------------

pub(crate) async fn retrieving<D: DocumentModel>() -> Result<(), FrameworkError> {
    dispatch_after_with(|| Retrieving::<D> {
        document_type: PhantomData,
    })
    .await
}

pub(crate) async fn retrieved<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Retrieved {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn saving<D: DocumentModel>(
    attrs: &SharedAttributes,
    is_creating: bool,
) -> Result<(), FrameworkError> {
    dispatch_cancellable_with(|| Saving::<D> {
        attrs: attrs.clone(),
        is_creating,
        document_type: PhantomData,
    })
    .await
}

pub(crate) async fn creating<D: DocumentModel>(
    attrs: &SharedAttributes,
) -> Result<(), FrameworkError> {
    dispatch_cancellable_with(|| Creating::<D> {
        attrs: attrs.clone(),
        document_type: PhantomData,
    })
    .await
}

pub(crate) async fn created<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Created {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn updating<D: DocumentModel>(
    previous: &D,
    attrs: &SharedAttributes,
) -> Result<(), FrameworkError> {
    dispatch_cancellable_with(|| Updating {
        previous: previous.clone(),
        attrs: attrs.clone(),
    })
    .await
}

pub(crate) async fn updated<D: DocumentModel>(
    previous: &D,
    current: &D,
) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Updated {
        previous: previous.clone(),
        current: current.clone(),
    })
    .await
}

pub(crate) async fn saved<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Saved {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn deleting<D: DocumentModel>(
    model: &D,
    is_force: bool,
) -> Result<(), FrameworkError> {
    dispatch_cancellable_with(|| Deleting {
        model: model.clone(),
        is_force,
    })
    .await
}

pub(crate) async fn deleted<D: DocumentModel>(
    model: &D,
    is_force: bool,
) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Deleted {
        model: model.clone(),
        is_force,
    })
    .await
}

pub(crate) async fn trashed<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Trashed {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn restoring<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_cancellable_with(|| Restoring {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn restored<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| Restored {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn force_deleting<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| ForceDeleting {
        model: model.clone(),
    })
    .await
}

pub(crate) async fn force_deleted<D: DocumentModel>(model: &D) -> Result<(), FrameworkError> {
    dispatch_after_with(|| ForceDeleted {
        model: model.clone(),
    })
    .await
}
