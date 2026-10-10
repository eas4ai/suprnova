//! Lazy singletons: one value per container, built by its factory on the
//! first resolve and shared by every resolve after it, as Laravel's
//! `singleton` and `singletonIf` build their concrete on first resolve.
//!
//! The factory runs with no container lock held, so it may resolve other
//! bindings. A resolve on another thread while the value is being built
//! waits for it, so the factory runs once. A resolve on the thread that is
//! building it is the factory asking for its own value, directly or through
//! other lazy singletons, and answers an error instead of waiting for a
//! value that can never be built. A factory that panics leaves the binding
//! unbuilt, so the next resolve runs it again.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

use super::scope::ScopedError;

/// A built value, as the container stores every binding's value.
pub(super) type LazyValue = Arc<dyn Any + Send + Sync>;

/// The factory of a lazy singleton.
pub(super) type LazyFactory = Arc<dyn Fn() -> LazyValue + Send + Sync>;

/// One lazy singleton binding.
pub(super) struct LazySingleton {
    /// The bound type, as `std::any::type_name` spells it, for the error.
    type_name: &'static str,
    factory: LazyFactory,
    state: Mutex<State>,
    /// Signalled when a build ends, built or not.
    finished: Condvar,
}

enum State {
    /// Not built yet.
    Unbuilt,
    /// Being built by the factory, on this thread.
    Building(ThreadId),
    /// Built: every resolve shares it.
    Built(LazyValue),
}

/// What one look at the state tells a resolve to do.
enum Step {
    Answer(LazyValue),
    SelfResolving,
    Wait,
    Build,
}

impl LazySingleton {
    pub(super) fn new(type_name: &'static str, factory: LazyFactory) -> Self {
        Self {
            type_name,
            factory,
            state: Mutex::new(State::Unbuilt),
            finished: Condvar::new(),
        }
    }

    /// The value: built now by this resolve if no resolve built it yet.
    pub(super) fn resolve(&self) -> Result<LazyValue, ScopedError> {
        let me = std::thread::current().id();
        let mut state = self.lock();
        loop {
            let step = match &*state {
                State::Built(value) => Step::Answer(value.clone()),
                State::Building(builder) if *builder == me => Step::SelfResolving,
                State::Building(_) => Step::Wait,
                State::Unbuilt => Step::Build,
            };
            match step {
                Step::Answer(value) => return Ok(value),
                Step::SelfResolving => {
                    return Err(ScopedError::LazyCycle {
                        type_name: self.type_name,
                    });
                }
                Step::Wait => {
                    state = self
                        .finished
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Step::Build => break,
            }
        }
        *state = State::Building(me);
        drop(state);

        // No lock is held while the factory runs: it may resolve other
        // bindings, and another thread resolving this one waits on the
        // condition variable.
        let outcome = catch_unwind(AssertUnwindSafe(|| (self.factory)()));
        let mut state = self.lock();
        match outcome {
            Ok(value) => {
                *state = State::Built(value.clone());
                self.finished.notify_all();
                Ok(value)
            }
            Err(panic) => {
                *state = State::Unbuilt;
                self.finished.notify_all();
                drop(state);
                resume_unwind(panic)
            }
        }
    }

    /// The state, recovered in place from a poisoned lock: the state is
    /// only ever replaced whole, so a panic cannot leave it half written.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
