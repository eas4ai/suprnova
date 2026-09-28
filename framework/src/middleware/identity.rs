//! The middleware type behind a boxed middleware, and the order the
//! priority list gives a chain.
//!
//! A [`BoxedMiddleware`] is a closure, and a closure does not say which
//! [`Middleware`] it wraps. Middleware priority needs to know, because it
//! orders a chain by type. The registration points know the type:
//! `.middleware(M)` on a route or a group, `global_middleware!`, an alias.
//! They box through [`boxed_as`], which remembers the type beside the box.
//! [`sort_by_priority`] reads it back when a chain is about to run.
//!
//! # Why a table beside the box
//!
//! The type could live in the box itself if `BoxedMiddleware` were a struct.
//! It is a public alias of `Arc<dyn Fn(...)>`, and applications build one by
//! hand for `.middleware_boxed(...)`, so making it a struct would break
//! them to serve a list most applications leave empty.
//!
//! The table is written at registration only. [`into_boxed`] itself writes
//! nothing, because the request path calls it for every request, and a
//! table that grew with every request would be a leak. A middleware boxed
//! by hand or by `into_boxed` has no known type, and the priority list
//! leaves it where it is.
//!
//! An entry holds a `Weak`, never the middleware. The address of a dropped
//! box can be given to a new one, and the `Weak` is how a lookup tells that
//! the box it was handed is the one the entry was written for.

use super::{BoxedMiddleware, Middleware, MiddlewareFuture, Next, into_boxed};
use crate::http::Request;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock, Weak};

type Boxed = dyn Fn(Request, Next) -> MiddlewareFuture + Send + Sync;

struct Known {
    middleware: Weak<Boxed>,
    type_id: TypeId,
}

#[derive(Default)]
struct Table {
    known: HashMap<usize, Known>,
    /// Entries of dropped middleware are removed when the table reaches
    /// this size. It doubles with the live entries, so the sweep costs a
    /// constant amount per registration.
    sweep_at: usize,
}

static TABLE: OnceLock<RwLock<Table>> = OnceLock::new();

fn table() -> &'static RwLock<Table> {
    TABLE.get_or_init(|| RwLock::new(Table::default()))
}

/// The key of a box: the address of what the `Arc` points to.
fn key(boxed: &BoxedMiddleware) -> usize {
    Arc::as_ptr(boxed).cast::<()>() as usize
}

/// Box `middleware` and remember its type, so the priority list can order
/// it. Every registration point boxes through this.
pub(crate) fn boxed_as<M: Middleware + 'static>(middleware: M) -> BoxedMiddleware {
    let boxed = into_boxed(middleware);
    // A poisoned table is recovered, as the registries beside it are: a
    // panic elsewhere must not stop middleware from being registered.
    let mut table = table().write().unwrap_or_else(|p| p.into_inner());
    if table.known.len() >= table.sweep_at {
        table
            .known
            .retain(|_, known| known.middleware.strong_count() > 0);
        table.sweep_at = (table.known.len() * 2).max(64);
    }
    table.known.insert(
        key(&boxed),
        Known {
            middleware: Arc::downgrade(&boxed),
            type_id: TypeId::of::<M>(),
        },
    );
    boxed
}

/// The middleware type `boxed` was registered as, when it was registered
/// through [`boxed_as`].
pub(crate) fn type_of(boxed: &BoxedMiddleware) -> Option<TypeId> {
    let table = table().read().unwrap_or_else(|p| p.into_inner());
    let known = table.known.get(&key(boxed))?;
    known
        .middleware
        .upgrade()
        .is_some_and(|live| Arc::ptr_eq(&live, boxed))
        .then_some(known.type_id)
}

/// Order `chain` by the priority list, the way Laravel's `SortedMiddleware`
/// does.
///
/// Only middleware named in the list move, and only forward: when one
/// stands behind a middleware that the list places after it, it is moved
/// to just before that one. Everything else keeps its place relative to
/// its neighbours. So a middleware registered after `AuthMiddleware` still
/// runs after it, whatever the list moves in front of it.
///
/// An empty list returns the chain as it was given, which is the cost every
/// application without a priority list pays: one read of the list.
pub(crate) fn sort_by_priority(chain: Vec<BoxedMiddleware>) -> Vec<BoxedMiddleware> {
    let priority = super::middleware_priority();
    if priority.is_empty() || chain.len() < 2 {
        return chain;
    }
    let ranks: Vec<Option<usize>> = chain
        .iter()
        .map(|boxed| {
            let type_id = type_of(boxed)?;
            priority.iter().position(|listed| *listed == type_id)
        })
        .collect();
    let mut order: Vec<usize> = (0..chain.len()).collect();
    sort_order(&mut order, &ranks);

    let mut slots: Vec<Option<BoxedMiddleware>> = chain.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots.get_mut(index).and_then(Option::take))
        .collect()
}

/// The ordering itself, on positions and ranks alone. `order` lists the
/// positions of a chain in the order they run, and `ranks[position]` is the
/// place of that middleware in the priority list, `None` when it is not
/// listed.
fn sort_order(order: &mut Vec<usize>, ranks: &[Option<usize>]) {
    // One pass finds the first listed middleware that stands behind a
    // listed middleware of a later rank, and moves it in front of that one.
    // Each move removes at least one inversion between listed middleware
    // and adds none, so the passes end.
    loop {
        let mut last: Option<(usize, usize)> = None;
        let mut moved = false;
        for at in 0..order.len() {
            let Some(rank) = ranks.get(order[at]).copied().flatten() else {
                continue;
            };
            if let Some((last_at, last_rank)) = last
                && rank < last_rank
            {
                let position = order.remove(at);
                order.insert(last_at, position);
                moved = true;
                break;
            }
            last = Some((at, rank));
        }
        if !moved {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Response;
    use async_trait::async_trait;

    /// Run the ordering on a chain written as ranks, and give back the
    /// original positions in their new order.
    fn sorted(ranks: &[Option<usize>]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..ranks.len()).collect();
        sort_order(&mut order, ranks);
        order
    }

    #[test]
    fn a_listed_middleware_moves_in_front_of_the_one_the_list_places_after_it() {
        // [unlisted, auth (rank 1), unlisted, session (rank 0)]
        assert_eq!(
            sorted(&[None, Some(1), None, Some(0)]),
            [0, 3, 1, 2],
            "session moves to just before auth, and what stood after auth still does"
        );
    }

    #[test]
    fn a_chain_already_in_order_is_left_alone() {
        assert_eq!(
            sorted(&[Some(0), None, Some(1), None, Some(2)]),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(sorted(&[None, None, None]), [0, 1, 2]);
        assert_eq!(sorted(&[]), Vec::<usize>::new());
    }

    #[test]
    fn several_listed_middleware_end_in_list_order() {
        // ranks 2, 1, 0 registered in the reverse of the list
        assert_eq!(sorted(&[Some(2), None, Some(1), Some(0)]), [3, 2, 0, 1]);
        // the same middleware type twice keeps its registration order
        assert_eq!(sorted(&[Some(1), Some(0), Some(1), Some(0)]), [1, 3, 0, 2]);
    }

    struct First;
    struct Second;

    #[async_trait]
    impl Middleware for First {
        async fn handle(&self, request: Request, next: Next) -> Response {
            next(request).await
        }
    }

    #[async_trait]
    impl Middleware for Second {
        async fn handle(&self, request: Request, next: Next) -> Response {
            next(request).await
        }
    }

    #[test]
    fn a_registered_box_knows_its_type_and_a_plain_box_does_not() {
        let first = boxed_as(First);
        let second = boxed_as(Second);
        let plain = into_boxed(First);

        assert_eq!(type_of(&first), Some(TypeId::of::<First>()));
        assert_eq!(type_of(&second), Some(TypeId::of::<Second>()));
        assert_eq!(type_of(&first.clone()), Some(TypeId::of::<First>()));
        assert_eq!(
            type_of(&plain),
            None,
            "into_boxed writes nothing: the request path calls it for every request"
        );
    }

    #[test]
    fn the_entries_of_dropped_middleware_are_swept() {
        // Far more registrations than the first sweep threshold, each
        // dropped at once. Without the sweep the table would hold them all.
        for _ in 0..1_000 {
            drop(boxed_as(First));
        }
        let kept = boxed_as(Second);

        let table = table().read().unwrap_or_else(|p| p.into_inner());
        let dead = table
            .known
            .values()
            .filter(|known| known.middleware.strong_count() == 0)
            .count();
        assert!(
            dead < 200,
            "{dead} entries of dropped middleware are still in the table"
        );
        drop(table);
        assert_eq!(type_of(&kept), Some(TypeId::of::<Second>()));
    }
}
