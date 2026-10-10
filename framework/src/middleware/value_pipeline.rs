//! A pipeline that sends any value through steps, the general form of
//! Laravel's `Illuminate\Pipeline\Pipeline`.
//!
//! The HTTP [`Pipeline`](super::Pipeline) passes only a request through
//! middleware. Laravel's pipeline sends any passable, which applications
//! use to run a value through a list of transformations or checks;
//! [`ValuePipeline`], made by [`Pipeline::of`](super::Pipeline::of), does
//! the same here.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;

use crate::database::DB;
use crate::error::FrameworkError;

/// The future a pipeline step answers: the pipeline's result, or the error
/// that stopped it.
pub type PipelineFuture<R> = Pin<Box<dyn Future<Output = Result<R, FrameworkError>> + Send>>;

/// The rest of the pipeline, handed to each step. Call it with the value to
/// run the steps after this one and the destination, as Laravel's `$next`.
/// It can be called once; a step that answers without calling it stops the
/// pipeline there.
pub type PipelineNext<T, R = T> = Box<dyn FnOnce(T) -> PipelineFuture<R> + Send>;

/// One step of a [`ValuePipeline`]: it receives the value and the rest of
/// the pipeline, and answers the pipeline's result. It passes the value on
/// with `next(value).await`, can change the value before and the result
/// after, and stops the pipeline by answering an error.
///
/// Every function or closure of the shape
/// `Fn(T, PipelineNext<T, R>) -> impl Future<Output = Result<R, FrameworkError>>`
/// is a step, so an `async fn` or an `async move` closure needs no
/// implementation of its own. A closure passed to
/// [`pipe`](ValuePipeline::pipe) names the types of its parameters, because
/// Rust infers a closure's parameter types only from an `Fn` bound. A type
/// that holds configuration implements the trait.
pub trait PipelineStep<T, R = T>: Send + Sync {
    /// Handle `value`, passing it on with `next` or stopping the pipeline.
    fn handle(&self, value: T, next: PipelineNext<T, R>) -> PipelineFuture<R>;
}

impl<T, R, F, Fut> PipelineStep<T, R> for F
where
    F: Fn(T, PipelineNext<T, R>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<R, FrameworkError>> + Send + 'static,
{
    fn handle(&self, value: T, next: PipelineNext<T, R>) -> PipelineFuture<R> {
        Box::pin(self(value, next))
    }
}

/// A pipeline over a value of type `T` whose result is an `R`, made by
/// [`Pipeline::of`](super::Pipeline::of), as Laravel's `Pipeline::send`
/// sends any passable through its pipes.
///
/// ```
/// use suprnova::Pipeline;
/// use suprnova::middleware::{PipelineFuture, PipelineNext};
///
/// fn add_one(value: i32, next: PipelineNext<i32>) -> PipelineFuture<i32> {
///     next(value + 1)
/// }
///
/// fn double(value: i32, next: PipelineNext<i32>) -> PipelineFuture<i32> {
///     next(value * 2)
/// }
///
/// # async fn run() {
/// let answer = Pipeline::of(5).through([add_one, double]).then_return().await;
/// assert_eq!(answer.ok(), Some(12));
/// # }
/// ```
///
/// `R` is `T` unless [`then`](Self::then) gives the destination another
/// result type.
pub struct ValuePipeline<T, R = T> {
    value: T,
    steps: Vec<Arc<dyn PipelineStep<T, R>>>,
    finally: Option<Box<dyn FnOnce() + Send>>,
    within_transaction: bool,
}

impl<T, R> ValuePipeline<T, R>
where
    T: Send + 'static,
    R: Send + 'static,
{
    /// A pipeline that will send `value`. Laravel's `send`.
    /// [`Pipeline::of`](super::Pipeline::of) reads better at a call site.
    pub fn new(value: T) -> Self {
        Self {
            value,
            steps: Vec::new(),
            finally: None,
            within_transaction: false,
        }
    }

    /// Set the steps, replacing any added before. Laravel's `through`.
    pub fn through<I, S>(mut self, steps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: PipelineStep<T, R> + 'static,
    {
        self.steps = Vec::new();
        self.pipe_all(steps)
    }

    /// Add one step after those already added. Laravel's `pipe`.
    pub fn pipe<S>(mut self, step: S) -> Self
    where
        S: PipelineStep<T, R> + 'static,
    {
        self.steps.push(Arc::new(step));
        self
    }

    /// Add several steps, in order, after those already added, where
    /// [`through`](Self::through) replaces them. Laravel's `pipe` with an
    /// array.
    pub fn pipe_all<I, S>(mut self, steps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: PipelineStep<T, R> + 'static,
    {
        for step in steps {
            self.steps.push(Arc::new(step));
        }
        self
    }

    /// Run `callback` when the pipeline ends, whether a step failed or
    /// not, as Laravel's `finally`. It also runs when the future of
    /// [`then`](Self::then) is dropped before it ends, or a step panics.
    /// A second call replaces the first callback.
    pub fn finally<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.finally = Some(Box::new(callback));
        self
    }

    /// Run the steps and the destination in one database transaction on
    /// the default connection, as Laravel's `withinTransaction`. An error
    /// from any step or the destination rolls back every write they made;
    /// see [`DB::transaction`].
    pub fn within_transaction(mut self) -> Self {
        self.within_transaction = true;
        self
    }

    /// Run the steps in order around `destination`, and answer what the
    /// destination answers, as the steps pass it back. Laravel's `then`.
    ///
    /// # Errors
    ///
    /// The error of the step or the destination that failed, or of the
    /// transaction when [`within_transaction`](Self::within_transaction)
    /// is on.
    pub async fn then<F, Fut>(self, destination: F) -> Result<R, FrameworkError>
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, FrameworkError>> + Send + 'static,
    {
        let _finally = Finally(self.finally);
        let steps: Arc<[Arc<dyn PipelineStep<T, R>>]> = self.steps.into();
        let destination: PipelineNext<T, R> = Box::new(move |value| Box::pin(destination(value)));
        let run = run_from(steps, 0, self.value, destination);
        if self.within_transaction {
            DB::transaction(move |_tx| run).await
        } else {
            run.await
        }
    }
}

impl<T> ValuePipeline<T, T>
where
    T: Send + 'static,
{
    /// Run the steps and answer the value the last step passed on.
    /// Laravel's `thenReturn`.
    ///
    /// # Errors
    ///
    /// The error of the step that failed, or of the transaction when
    /// [`within_transaction`](Self::within_transaction) is on.
    pub async fn then_return(self) -> Result<T, FrameworkError> {
        self.then(|value| async move { Ok(value) }).await
    }
}

/// Run the step at `index`, handing it the steps after it and then the
/// destination as its `next`.
fn run_from<T, R>(
    steps: Arc<[Arc<dyn PipelineStep<T, R>>]>,
    index: usize,
    value: T,
    destination: PipelineNext<T, R>,
) -> PipelineFuture<R>
where
    T: Send + 'static,
    R: Send + 'static,
{
    let Some(step) = steps.get(index).cloned() else {
        return destination(value);
    };
    let next: PipelineNext<T, R> =
        Box::new(move |value| run_from(steps, index + 1, value, destination));
    step.handle(value, next)
}

/// Runs the `finally` callback when the pipeline's future ends or is
/// dropped. A callback that panics while a step's panic unwinds is caught,
/// so it cannot abort the process.
struct Finally(Option<Box<dyn FnOnce() + Send>>);

impl Drop for Finally {
    fn drop(&mut self) {
        let Some(callback) = self.0.take() else {
            return;
        };
        if std::thread::panicking() {
            let _ = catch_unwind(AssertUnwindSafe(callback));
        } else {
            callback();
        }
    }
}
