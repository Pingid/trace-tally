use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::Layer;
use tracing_subscriber::registry::LookupSpan;

use crate::{Action, Renderer, TaskId};

/// Extracts user-defined data from `tracing` spans and events.
pub trait TraceMapper<R: Renderer> {
    /// Converts span attributes into [`Renderer::TaskData`].
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> R::TaskData;
    /// Converts a tracing event into [`Renderer::EventData`].
    fn map_event(event: &tracing::Event<'_>) -> R::EventData;
}

/// Sends [`Action`]s from the tracing layer to the render loop.
///
/// Implementations MUST be non-blocking. The tracing `Layer` calls
/// this from synchronous `on_new_span`, `on_event`, and `on_close`
/// hooks — blocking here stalls the instrumented application.
///
/// A default implementation is provided for [`std::sync::mpsc::Sender`].
pub trait ActionTransport<R: Renderer>: Send + Sync + 'static {
    type Error;
    /// Dispatches a single action.
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error>;
}

impl<R: Renderer + 'static> ActionTransport<R> for std::sync::mpsc::Sender<Action<R>> {
    type Error = std::sync::mpsc::SendError<Action<R>>;
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error> {
        self.send(action)
    }
}

/// Convenience trait for constructing a [`TaskLayer`] from a [`TraceMapper`] type.
pub trait TaskTraceLayer<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> {
    /// Creates a [`TaskLayer`] using the given sender.
    fn task_layer(sender: T) -> TaskLayer<R, M, T>;
}

impl<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> TaskTraceLayer<R, M, T> for M {
    fn task_layer(sender: T) -> TaskLayer<R, M, T> {
        TaskLayer::new(sender)
    }
}

/// Creates a [`TaskLayer`] that maps spans and events using `M` and sends
/// actions through `sender`.
pub fn task_layer<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>>(
    sender: T,
) -> TaskLayer<R, M, T> {
    TaskLayer::new(sender)
}

/// Atomic counter that produces unique `TaskId`s without any locking.
#[derive(Debug)]
struct IdGenerator(AtomicUsize);

impl Default for IdGenerator {
    fn default() -> Self {
        Self(AtomicUsize::new(1)) // 0 is reserved for ROOT
    }
}

impl IdGenerator {
    fn next(&self) -> TaskId {
        let id = self.0.fetch_add(1, Ordering::Relaxed);
        TaskId::new(NonZeroUsize::new(id).expect("TaskId generation overflow"))
    }
}

#[derive(Debug, Clone, Copy)]
struct TaskIdExt(TaskId);

/// A `tracing` [`Layer`] that captures spans and events as task-tree [`Action`]s.
pub struct TaskLayer<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> {
    tx: T,
    ids: IdGenerator,
    error_handler: Option<Box<dyn Fn(T::Error) + Send + Sync>>,
    _mapper: PhantomData<M>,
    _renderer: PhantomData<R>,
}

impl<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> TaskLayer<R, M, T> {
    /// Creates a new layer that sends actions through `tx`.
    pub fn new(tx: T) -> Self {
        Self {
            tx,
            ids: IdGenerator::default(),
            error_handler: None,
            _mapper: PhantomData,
            _renderer: PhantomData,
        }
    }

    /// Sets an error handler to be called when an action fails to send.
    pub fn with_error_handler(
        mut self,
        error_handler: impl Fn(T::Error) + Send + Sync + 'static,
    ) -> Self {
        self.error_handler = Some(Box::new(error_handler));
        self
    }

    /// Look up the TaskId we previously stored in a span's extensions.
    fn task_id<S>(&self, span: &tracing_subscriber::registry::SpanRef<'_, S>) -> Option<TaskId>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        span.extensions().get::<TaskIdExt>().map(|ext| ext.0)
    }

    fn send_action(&self, action: Action<R>) {
        match self.tx.send_action(action) {
            Ok(_) => (),
            Err(error) => {
                if let Some(error_handler) = &self.error_handler {
                    error_handler(error);
                }
            }
        }
    }
}

impl<S, R, M, T> Layer<S> for TaskLayer<R, M, T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    R: Renderer + 'static,
    M: TraceMapper<R> + 'static,
    T: ActionTransport<R> + 'static,
{
    fn on_new_span(
        &self,
        attrs: &Attributes<'_>,
        id: &Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let task_id = self.ids.next();
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(TaskIdExt(task_id));
        }

        // Resolve parent by reading *its* stored TaskId.
        let parent_id = ctx
            .span(id)
            .and_then(|s| s.parent())
            .and_then(|parent| self.task_id(&parent));

        self.send_action(Action::TaskStart {
            id: task_id,
            parent: parent_id,
            data: M::map_span(attrs),
        })
    }

    fn on_close(&self, id: Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let task_id = ctx.span(&id).and_then(|span| self.task_id(&span));

        if let Some(task_id) = task_id {
            self.send_action(Action::TaskEnd { id: task_id });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let parent = ctx.lookup_current().and_then(|span| self.task_id(&span));
        self.send_action(Action::Event {
            parent,
            data: M::map_event(event),
        });
    }
}
