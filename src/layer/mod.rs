//! Capturing `tracing` spans and events as [`Action`]s.
//!
//! A [`TaskLayer`] maps spans and events with a [`TraceMapper`] and hands
//! the resulting actions to an [`ActionSink`]. Neither knows anything about
//! rendering.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::Subscriber;
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::model::{Action, TaskId};
use crate::render::{ActionOf, Renderer, TaskRenderer};

/// Extracts user-defined data from `tracing` spans and events.
///
/// ```rust,ignore
/// struct MyMapper;
/// impl TraceMapper for MyMapper {
///     type EventData = String;
///     type TaskData = String;
///
///     fn map_span(&self, attrs: &tracing::span::Attributes<'_>) -> String {
///         attrs.metadata().name().to_string()
///     }
///     fn map_event(&self, event: &tracing::Event<'_>) -> String {
///         format!("{:?}", event)
///     }
/// }
/// ```
pub trait TraceMapper: Send + Sync + 'static {
    /// Data stored per task.
    type TaskData;
    /// Data stored per event.
    type EventData;

    /// Converts span attributes into task data.
    fn map_span(&self, attrs: &Attributes<'_>) -> Self::TaskData;

    /// Converts a tracing event into event data.
    fn map_event(&self, event: &tracing::Event<'_>) -> Self::EventData;

    /// Called when fields are recorded on an open span (`span.record(..)`).
    ///
    /// Return `Some` to replace the task's data. `previous` is the data
    /// produced by the last call to [`map_span`](Self::map_span) or this method.
    #[allow(unused_variables)]
    fn map_record(&self, previous: &Self::TaskData, values: &Record<'_>) -> Option<Self::TaskData> {
        None
    }
}

/// Receives [`Action`]s produced by a [`TaskLayer`].
///
/// Implemented for closures, [`std::sync::mpsc::Sender`] and [`InlineSink`].
///
/// ```rust,ignore
/// // Any channel works through a closure:
/// let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
/// let layer = TaskLayer::new(MyMapper, move |action| {
///     let _ = tx.send(action);
/// });
/// ```
pub trait ActionSink<T, E>: Send + Sync + 'static {
    fn send(&self, action: Action<T, E>);
}

impl<T, E, F> ActionSink<T, E> for F
where
    F: Fn(Action<T, E>) + Send + Sync + 'static,
{
    fn send(&self, action: Action<T, E>) {
        self(action)
    }
}

/// Send errors are ignored: they only occur once the receiver (the render
/// loop) has gone away. Use a closure sink to handle them.
impl<T: Send + 'static, E: Send + 'static> ActionSink<T, E>
    for std::sync::mpsc::Sender<Action<T, E>>
{
    fn send(&self, action: Action<T, E>) {
        let _ = std::sync::mpsc::Sender::send(self, action);
    }
}

/// Renders synchronously on every action. No channel or background thread.
pub struct InlineSink<R: Renderer, W> {
    inner: Mutex<(TaskRenderer<R>, W)>,
}

impl<R: Renderer, W> InlineSink<R, W> {
    pub fn new(renderer: TaskRenderer<R>, writer: W) -> Self {
        Self {
            inner: Mutex::new((renderer, writer)),
        }
    }

    /// Runs `f` with the renderer and writer locked.
    pub fn with<O>(&self, f: impl FnOnce(&mut TaskRenderer<R>, &mut W) -> O) -> O {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (renderer, writer) = &mut *guard;
        f(renderer, writer)
    }
}

impl<R, W> ActionSink<R::TaskData, R::EventData> for InlineSink<R, W>
where
    R: Renderer + Send + 'static,
    R::TaskData: Send + 'static,
    R::EventData: Send + 'static,
    W: std::io::Write + Send + 'static,
{
    fn send(&self, action: ActionOf<R>) {
        self.with(|renderer, writer| {
            renderer.update(action);
            let _ = renderer.render(writer);
        })
    }
}

/// A [`tracing_subscriber::Layer`] that captures spans and events as task-tree [`Action`]s.
///
/// ```rust,ignore
/// // Render inline:
/// let layer = TaskLayer::inline(MyMapper, MyRenderer, std::io::stderr());
///
/// // Or send to a render loop on another thread:
/// let (tx, rx) = std::sync::mpsc::channel();
/// let layer = TaskLayer::new(MyMapper, tx);
/// ```
pub struct TaskLayer<M, S> {
    mapper: M,
    sink: S,
    ids: AtomicU64,
}

impl<M, S> TaskLayer<M, S>
where
    M: TraceMapper,
    S: ActionSink<M::TaskData, M::EventData>,
{
    /// Creates a layer that maps with `mapper` and delivers to `sink`.
    pub fn new(mapper: M, sink: S) -> Self {
        Self {
            mapper,
            sink,
            ids: AtomicU64::new(1),
        }
    }

    /// The mapper.
    pub fn mapper(&self) -> &M {
        &self.mapper
    }

    /// The sink.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    fn next_id(&self) -> TaskId {
        TaskId::new(self.ids.fetch_add(1, Ordering::Relaxed))
    }
}

impl<M, R, W> TaskLayer<M, InlineSink<R, W>>
where
    M: TraceMapper,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + Send + 'static,
    R::TaskData: Send + 'static,
    R::EventData: Send + 'static,
    W: std::io::Write + Send + 'static,
{
    /// Creates a layer that renders to `writer` on every span and event.
    pub fn inline(mapper: M, renderer: R, writer: W) -> Self {
        Self::new(mapper, InlineSink::new(TaskRenderer::new(renderer), writer))
    }
}

/// Stored in span extensions: the task id and its latest data, which
/// [`TraceMapper::map_record`] receives as `previous`.
struct SpanTask<T> {
    id: TaskId,
    data: T,
}

impl<S, M, K> Layer<S> for TaskLayer<M, K>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    M: TraceMapper,
    M::TaskData: Clone + Send + Sync + 'static,
    K: ActionSink<M::TaskData, M::EventData>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let task_id = self.next_id();
        let parent = span
            .parent()
            .and_then(|p| p.extensions().get::<SpanTask<M::TaskData>>().map(|t| t.id));
        let data = self.mapper.map_span(attrs);
        span.extensions_mut().insert(SpanTask {
            id: task_id,
            data: data.clone(),
        });
        self.sink.send(Action::TaskStart {
            id: task_id,
            parent,
            data,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut ext = span.extensions_mut();
        let Some(task) = ext.get_mut::<SpanTask<M::TaskData>>() else {
            return;
        };
        if let Some(data) = self.mapper.map_record(&task.data, values) {
            task.data = data.clone();
            let id = task.id;
            drop(ext);
            self.sink.send(Action::TaskUpdate { id, data });
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let task_id = ctx
            .span(&id)
            .and_then(|s| s.extensions().get::<SpanTask<M::TaskData>>().map(|t| t.id));
        if let Some(id) = task_id {
            self.sink.send(Action::TaskEnd { id });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let parent = ctx
            .event_span(event)
            .and_then(|s| s.extensions().get::<SpanTask<M::TaskData>>().map(|t| t.id));
        self.sink.send(Action::Event {
            parent,
            data: self.mapper.map_event(event),
        });
    }
}

/// Method-style constructors on any [`TraceMapper`].
///
/// ```rust,ignore
/// let layer = MyMapper.inline_layer(MyRenderer, std::io::stderr());
/// let layer = MyMapper.layer(tx);
/// ```
pub trait TraceMapperExt: TraceMapper + Sized {
    /// Creates a layer delivering to `sink`. See [`TaskLayer::new`].
    fn layer<S: ActionSink<Self::TaskData, Self::EventData>>(self, sink: S) -> TaskLayer<Self, S> {
        TaskLayer::new(self, sink)
    }

    /// Creates an inline-rendering layer. See [`TaskLayer::inline`].
    fn inline_layer<R, W>(self, renderer: R, writer: W) -> TaskLayer<Self, InlineSink<R, W>>
    where
        R: Renderer<TaskData = Self::TaskData, EventData = Self::EventData> + Send + 'static,
        R::TaskData: Send + 'static,
        R::EventData: Send + 'static,
        W: std::io::Write + Send + 'static,
    {
        TaskLayer::inline(self, renderer, writer)
    }
}

impl<M: TraceMapper> TraceMapperExt for M {}
