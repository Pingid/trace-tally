use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
/// A default implementation is provided for [`std::sync::mpsc::Sender`].
pub trait ActionTransport<R: Renderer> {
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

/// Convenience trait for constructing a [`TaskLayer`] from an [`TraceMapper`] type.
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

#[derive(Clone, Default)]
pub struct SpanIdMapper {
    generation: Arc<AtomicUsize>,
    mapping: Arc<Mutex<HashMap<tracing::Id, TaskId>>>,
}

impl SpanIdMapper {
    pub fn get_or_create(&self, id: &tracing::Id) -> TaskId {
        let mut map = self.mapping.lock().unwrap();
        *map.entry(id.clone()).or_insert_with(|| {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed);
            TaskId::new(generation + 1)
        })
    }

    pub fn remove(&self, id: &tracing::Id) -> Option<TaskId> {
        self.mapping.lock().unwrap().remove(id)
    }
}

/// A `tracing` [`Layer`] that captures spans and events as task-tree [`Action`]s.
pub struct TaskLayer<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> {
    tx: T,
    ids: SpanIdMapper,
    _mapper: std::marker::PhantomData<M>,
    _renderer: std::marker::PhantomData<R>,
}

impl<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> Clone for TaskLayer<R, M, T>
where T: Clone
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            ids: self.ids.clone(),
            _mapper: PhantomData,
            _renderer: PhantomData,
        }
    }
}

impl<R: Renderer, M: TraceMapper<R>, T: ActionTransport<R>> TaskLayer<R, M, T> {
    /// Creates a new layer that sends actions through `tx`.
    pub fn new(tx: T) -> Self {
        Self {
            tx,
            ids: SpanIdMapper::default(),
            _mapper: PhantomData,
            _renderer: PhantomData,
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
        &self, attrs: &Attributes<'_>, id: &Id, ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let u_id = self.ids.get_or_create(id);
        let parent_id = ctx
            .span(id)
            .and_then(|s| s.parent())
            .map(|p| self.ids.get_or_create(&p.id()));

        let _ = self.tx.send_action(Action::TaskStart {
            id: u_id, // Pass the decoupled ID
            parent: parent_id,
            data: M::map_span(attrs),
        });
    }

    fn on_close(&self, id: Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(u_id) = self.ids.remove(&id) {
            let _ = self.tx.send_action(Action::TaskEnd { id: u_id });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let parent = ctx
            .lookup_current()
            .map(|span| self.ids.get_or_create(&span.id()));

        // Use the mapper to create LogEvent
        let log_data = M::map_event(event);

        let _ = self.tx.send_action(Action::Event {
            parent,
            data: log_data,
        });
    }
}
