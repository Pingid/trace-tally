use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::Layer;
use tracing_subscriber::registry::LookupSpan;

use super::TraceMapper;
use crate::{Action, Renderer, TaskId};

/// Action delivery strategy.
pub trait ActionHandler<R: Renderer>: 'static {
    fn handle(&self, action: Action<R>);
}

/// A `tracing` [`tracing_subscriber::Layer`] that captures spans and events as task-tree [`Action`]s.
pub struct TaskLayer<M, R, H> {
    pub(crate) handler: H,
    ids: IdGenerator,
    _mapper: PhantomData<M>,
    _renderer: PhantomData<R>,
}

impl<M, R, H> TaskLayer<M, R, H>
where
    M: TraceMapper + 'static,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + 'static,
    H: ActionHandler<R>,
{
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            ids: IdGenerator::default(),
            _mapper: PhantomData,
            _renderer: PhantomData,
        }
    }

    fn task_id<S>(&self, span: &tracing_subscriber::registry::SpanRef<'_, S>) -> Option<TaskId>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        span.extensions().get::<TaskIdExt>().map(|ext| ext.0)
    }
}

impl<S, M, R, H> Layer<S> for TaskLayer<M, R, H>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    M: TraceMapper + 'static,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + 'static,
    H: ActionHandler<R>,
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

        let parent_id = ctx
            .span(id)
            .and_then(|s| s.parent())
            .and_then(|parent| self.task_id(&parent));

        self.handler.handle(Action::TaskStart {
            id: task_id,
            parent: parent_id,
            data: M::map_span(attrs),
        });
    }

    fn on_close(&self, id: Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(task_id) = ctx.span(&id).and_then(|span| self.task_id(&span)) {
            self.handler.handle(Action::TaskEnd { id: task_id });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let parent = ctx.lookup_current().and_then(|span| self.task_id(&span));
        self.handler.handle(Action::Event {
            parent,
            data: M::map_event(event),
        });
    }
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
