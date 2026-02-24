use std::marker::PhantomData;

use super::{ActionHandler, TaskLayer, TraceMapper};
use crate::{Action, Renderer};

/// Sends [`Action`]s to a render loop.
///
/// Implemented for [`std::sync::mpsc::Sender`] out of the box.
/// Implement this trait for custom transports (crossbeam, tokio channels, etc).
pub trait ActionTransport<R: Renderer>: Send + Sync + 'static {
    type Error;
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error>;
}

impl<R: Renderer + 'static> ActionTransport<R> for std::sync::mpsc::Sender<Action<R>> {
    type Error = std::sync::mpsc::SendError<Action<R>>;
    fn send_action(&self, action: Action<R>) -> Result<(), Self::Error> {
        self.send(action)
    }
}

/// Delivers actions over an mpsc channel to a separate render loop.
pub struct ChannelHandler<R: Renderer, H: ActionTransport<R>> {
    pub(crate) transport: H,
    pub(crate) _renderer: PhantomData<R>,
    pub(crate) error_handler: Option<Box<dyn Fn(H::Error) + Send + Sync>>,
}

impl<R: Renderer + 'static, H: ActionTransport<R>> ActionHandler<R> for ChannelHandler<R, H> {
    fn handle(&self, action: Action<R>) {
        match self.transport.send_action(action) {
            Ok(_) => (),
            Err(error) => {
                if let Some(error_handler) = &self.error_handler {
                    error_handler(error);
                }
            }
        }
    }
}

impl<M, R, H> TaskLayer<M, R, ChannelHandler<R, H>>
where
    M: TraceMapper + 'static,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + 'static,
    H: ActionTransport<R>,
{
    /// Registers a callback invoked when the transport fails to deliver an action.
    ///
    /// Without an error handler, send failures are silently ignored.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (tx, rx) = std::sync::mpsc::channel();
    /// let layer = MyMapper::channel_layer(tx)
    ///     .with_error_handler(|e| eprintln!("transport error: {e}"));
    /// ```
    pub fn with_error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(H::Error) + Send + Sync + 'static,
    {
        self.handler.error_handler = Some(Box::new(f));
        self
    }
}

/// Creates a tracing layer that delivers actions over an [`ActionTransport`].
///
/// Use this when rendering runs on a separate thread. The transport
/// receives [`Action`]s which can be fed to [`crate::TaskRenderer`] in a render loop.
///
/// ```rust,ignore
/// let (tx, rx) = std::sync::mpsc::channel();
/// let layer = channel_layer::<MyMapper, MyRenderer, _>(tx);
///
/// // On the render thread:
/// let mut renderer = TaskRenderer::new(MyRenderer::default());
/// loop {
///     while let Ok(action) = rx.try_recv() {
///         renderer.update(action);
///     }
///     renderer.render(&mut std::io::stderr()).unwrap();
///     std::thread::sleep(std::time::Duration::from_millis(100));
/// }
/// ```
pub fn channel_layer<M, R, T: ActionTransport<R>>(
    transport: T,
) -> TaskLayer<M, R, ChannelHandler<R, T>>
where
    M: TraceMapper,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + 'static,
{
    TaskLayer::new(ChannelHandler {
        transport,
        _renderer: PhantomData,
        error_handler: None,
    })
}
