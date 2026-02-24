use crate::Renderer;

mod channel;
mod inline;
mod layer;

pub(crate) use channel::ChannelHandler;
pub use channel::{ActionTransport, channel_layer};
pub(crate) use inline::InlineHandler;
pub use inline::inline_layer;
pub(crate) use layer::{ActionHandler, TaskLayer};

/// Extracts user-defined data from `tracing` spans and events.
///
/// The associated types must match the [`Renderer`] you pair it with — both
/// traits declare `TaskData` and `EventData`, and the layer constructors
/// enforce [`TraceMapper::TaskData`] == [`Renderer::TaskData`] (likewise for events).
///
/// ```rust,ignore
/// struct MyMapper;
/// impl TraceMapper for MyMapper {
///     type EventData = String;
///     type TaskData = String;
///
///     fn map_span(attrs: &tracing::span::Attributes<'_>) -> String {
///         attrs.metadata().name().to_string()
///     }
///     fn map_event(event: &tracing::Event<'_>) -> String {
///         format!("{:?}", event)
///     }
/// }
/// ```
pub trait TraceMapper: 'static {
    /// Data stored per event (e.g. a log message or span field snapshot).
    type EventData;
    /// Data stored per task (e.g. a task name or metadata).
    type TaskData;

    /// Converts span attributes into task data.
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> Self::TaskData;
    /// Converts a tracing event into event data.
    fn map_event(event: &tracing::Event<'_>) -> Self::EventData;
}

/// Convenience constructors for building tracing layers.
///
/// Automatically implemented for all [`TraceMapper`] types. These are
/// thin wrappers around [`inline_layer`] and [`channel_layer`] that let
/// you call the constructor as an associated function on your mapper type.
///
/// ```rust,ignore
/// // These two are equivalent:
/// let layer = MyMapper::inline_layer(MyRenderer::default(), std::io::stderr());
/// let layer = inline_layer::<MyMapper, _, _>(MyRenderer::default(), std::io::stderr());
/// ```
pub trait TraceMapperExt: TraceMapper + Sized {
    /// Creates a channel-based tracing layer. See [`channel_layer`] for details.
    fn channel_layer<R, H>(handler: H) -> TaskLayer<Self, R, ChannelHandler<R, H>>
    where
        R: Renderer + 'static,
        H: ActionTransport<R>,
        R: Renderer<TaskData = Self::TaskData, EventData = Self::EventData>,
    {
        channel_layer(handler)
    }

    /// Creates an inline tracing layer. See [`inline_layer`] for details.
    fn inline_layer<R, W>(renderer: R, writer: W) -> TaskLayer<Self, R, InlineHandler<R, W>>
    where
        R: Renderer + 'static,
        W: std::io::Write + Send + 'static,
        R: Renderer<TaskData = Self::TaskData, EventData = Self::EventData>,
    {
        inline_layer(renderer, writer)
    }
}

impl<M: TraceMapper + 'static> TraceMapperExt for M {}
