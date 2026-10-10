//! Panic-isolating driver for host and component futures.
//!
//! Every boxed future handed to the engine by a host or component is polled and dropped behind
//! `catch_unwind` so an unwinding panic cannot cross the engine boundary. Panics are reported
//! through the caller's closed error type via `on_panic`.
//!
//! Precedence: a panic while dropping the future is reported as a panic even when the future
//! already completed with a value or an error. A panicking destructor proves the host or
//! component is in an undefined state, which is more severe than the failure it had reported.

use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::Poll;

/// Boxed future shape shared by every host and component port in the engine.
pub(crate) type IsolatedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Panic-contains future construction and returns the isolated future driver.
///
/// Construction panics are reported immediately through `on_panic`; poll and drop panics are
/// reported when the returned future completes.
pub(crate) fn catch_isolated<'a, T, E, R>(
    operation: impl FnOnce() -> IsolatedFuture<'a, Result<T, E>>,
    map_error: impl FnOnce(E) -> R + Send + 'a,
    on_panic: impl FnOnce() -> R + Send + 'a,
) -> Result<impl Future<Output = Result<T, R>> + Send + 'a, R>
where
    T: Send + 'a,
    E: Send + 'a,
    R: Send + 'a,
{
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(future) => Ok(poll_isolated(future, map_error, on_panic)),
        Err(_) => Err(on_panic()),
    }
}

/// Panic-contains construction, polling, and dropping of one future.
pub(crate) async fn run_isolated<'a, T, E, R>(
    operation: impl FnOnce() -> IsolatedFuture<'a, Result<T, E>>,
    map_error: impl FnOnce(E) -> R + Send + 'a,
    on_panic: impl FnOnce() -> R + Send + 'a,
) -> Result<T, R>
where
    T: Send + 'a,
    E: Send + 'a,
    R: Send + 'a,
{
    catch_isolated(operation, map_error, on_panic)?.await
}

/// Panic-contains polling and dropping of an already constructed future.
pub(crate) async fn poll_isolated<T, E, R>(
    mut future: IsolatedFuture<'_, Result<T, E>>,
    map_error: impl FnOnce(E) -> R,
    on_panic: impl FnOnce() -> R,
) -> Result<T, R> {
    let polled =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(result)) => Poll::Ready(Some(result)),
                Ok(Poll::Pending) => Poll::Pending,
                Err(_) => Poll::Ready(None),
            },
        )
        .await;
    if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
        return Err(on_panic());
    }
    match polled {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => Err(map_error(error)),
        None => Err(on_panic()),
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::{catch_isolated, poll_isolated, run_isolated};

    #[derive(Debug, Eq, PartialEq)]
    enum Outcome {
        Inner,
        Panicked,
    }

    struct DropPanic<T> {
        output: Option<T>,
    }

    impl<T: Unpin> Future for DropPanic<T> {
        type Output = T;

        fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<T> {
            Poll::Ready(self.output.take().expect("polled once"))
        }
    }

    impl<T> Drop for DropPanic<T> {
        fn drop(&mut self) {
            panic!("drop panic");
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = Box::pin(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    fn map(_: ()) -> Outcome {
        Outcome::Inner
    }

    fn panicked() -> Outcome {
        Outcome::Panicked
    }

    #[test]
    fn passes_through_value_and_maps_inner_error() {
        let ok = block_on(run_isolated(
            || Box::pin(ready(Ok::<u8, ()>(7))),
            map,
            panicked,
        ));
        assert_eq!(ok, Ok(7));
        let err = block_on(run_isolated(
            || Box::pin(ready(Err::<u8, ()>(()))),
            map,
            panicked,
        ));
        assert_eq!(err, Err(Outcome::Inner));
    }

    #[test]
    fn construction_and_poll_panics_are_contained() {
        let constructed = catch_isolated(
            || -> super::IsolatedFuture<'static, Result<u8, ()>> { panic!("construct") },
            map,
            panicked,
        );
        assert!(matches!(constructed, Err(Outcome::Panicked)));
        let polled = block_on(run_isolated(
            || {
                Box::pin(async {
                    if true {
                        panic!("poll");
                    }
                    Ok::<u8, ()>(0)
                })
            },
            map,
            panicked,
        ));
        assert_eq!(polled, Err(Outcome::Panicked));
    }

    #[test]
    fn drop_panic_overrides_value_and_inner_error() {
        let after_ok = block_on(poll_isolated(
            Box::pin(DropPanic {
                output: Some(Ok::<u8, ()>(1)),
            }),
            map,
            panicked,
        ));
        assert_eq!(after_ok, Err(Outcome::Panicked));
        let after_err = block_on(poll_isolated(
            Box::pin(DropPanic {
                output: Some(Err::<u8, ()>(())),
            }),
            map,
            panicked,
        ));
        assert_eq!(after_err, Err(Outcome::Panicked));
    }
}
