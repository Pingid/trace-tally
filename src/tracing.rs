use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::Layer;
use tracing_subscriber::registry::LookupSpan;

use crate::{Action, Renderer, TaskId};

pub trait EventMapper<R: Renderer> {
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> R::TaskData;
    fn map_event(event: &tracing::Event<'_>) -> R::EventData;
}

pub trait ActionSender<R: Renderer> {
    type Error;
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error>;
}

impl<R: Renderer + 'static> ActionSender<R> for std::sync::mpsc::Sender<Action<R>> {
    type Error = std::sync::mpsc::SendError<Action<R>>;
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error> {
        self.send(action)
    }
}

pub trait TaskTraceLayer<R: Renderer, M: EventMapper<R>, T: ActionSender<R>> {
    fn task_layer(sender: T) -> TaskLayer<R, M, T>;
}

impl<R: Renderer, M: EventMapper<R>, T: ActionSender<R>> TaskTraceLayer<R, M, T> for M {
    fn task_layer(sender: T) -> TaskLayer<R, M, T> {
        TaskLayer::new(sender)
    }
}

pub fn task_layer<R: Renderer, M: EventMapper<R>, T: ActionSender<R>>(
    sender: T,
) -> TaskLayer<R, M, T> {
    TaskLayer::new(sender)
}

#[derive(Clone, Default)]
pub struct IdProvider {
    generation: Arc<AtomicUsize>,
    mapping: Arc<Mutex<HashMap<tracing::Id, TaskId>>>,
}

impl IdProvider {
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

pub struct TaskLayer<R: Renderer, M: EventMapper<R>, T: ActionSender<R>> {
    tx: T,
    ids: IdProvider,
    _mapper: std::marker::PhantomData<M>,
    _renderer: std::marker::PhantomData<R>,
}

impl<R: Renderer, M: EventMapper<R>, T: ActionSender<R>> Clone for TaskLayer<R, M, T>
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

impl<R: Renderer, M: EventMapper<R>, T: ActionSender<R>> TaskLayer<R, M, T> {
    pub fn new(tx: T) -> Self {
        Self {
            tx,
            ids: IdProvider::default(),
            _mapper: PhantomData,
            _renderer: PhantomData,
        }
    }
}

impl<S, R, M, T> Layer<S> for TaskLayer<R, M, T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    R: Renderer + 'static,
    M: EventMapper<R> + 'static,
    T: ActionSender<R> + 'static,
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
            event: M::map_span(attrs),
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
            event: log_data,
        });
    }
}
