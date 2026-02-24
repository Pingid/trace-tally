use std::sync::Mutex;

use super::{ActionHandler, TaskLayer, TraceMapper};
use crate::{Action, Renderer, TaskRenderer};

/// Renders immediately on every action. No channel, no background thread.
pub struct InlineHandler<R: Renderer, W: std::io::Write + Send + 'static> {
    inner: Mutex<(TaskRenderer<R>, W)>,
}

impl<R: Renderer + 'static, W: std::io::Write + Send + 'static> ActionHandler<R>
    for InlineHandler<R, W>
{
    #[inline]
    fn handle(&self, action: Action<R>) {
        let mut guard = self.inner.lock().unwrap();
        let (ref mut renderer, ref mut writer) = *guard;
        renderer.update(action);
        let _ = renderer.render(writer);
    }
}

/// Creates a tracing layer that renders immediately on every action.
///
/// Each span open/close and event triggers a full re-render to `writer`.
/// No background thread or channel is needed.
///
/// ```rust,ignore
/// let layer = inline_layer::<MyMapper, MyRenderer, _>(
///     MyRenderer::default(),
///     std::io::stderr(),
/// );
/// tracing_subscriber::registry().with(layer).init();
/// ```
pub fn inline_layer<M, R, W>(renderer: R, writer: W) -> TaskLayer<M, R, InlineHandler<R, W>>
where
    M: TraceMapper + 'static,
    R: Renderer<TaskData = M::TaskData, EventData = M::EventData> + 'static,
    W: std::io::Write + Send + 'static,
{
    TaskLayer::new(InlineHandler {
        inner: Mutex::new((TaskRenderer::new(renderer), writer)),
    })
}
