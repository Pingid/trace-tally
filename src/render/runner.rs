use std::io::Write;
use std::time::Duration;

use super::{Renderer, TaskRenderer};
use crate::model::{Action, TaskTree};

/// Drains available actions from a channel or queue into a [`TaskTree`].
///
/// Implemented for [`std::sync::mpsc::Receiver`]. Implement it for other
/// channel backends (crossbeam, flume, tokio, etc.).
pub trait ActionSource<T, E> {
    /// Apply all immediately-available actions to `tree`.
    ///
    /// Must not block. Returns `false` once the source is permanently closed.
    fn drain_into(&mut self, tree: &mut TaskTree<T, E>) -> bool;
}

impl<T, E, F> ActionSource<T, E> for F
where
    F: FnMut(&mut TaskTree<T, E>) -> bool,
{
    fn drain_into(&mut self, tree: &mut TaskTree<T, E>) -> bool {
        self(tree)
    }
}

impl<T, E> ActionSource<T, E> for std::sync::mpsc::Receiver<Action<T, E>> {
    fn drain_into(&mut self, tree: &mut TaskTree<T, E>) -> bool {
        use std::sync::mpsc::TryRecvError;
        loop {
            match self.try_recv() {
                Ok(action) => tree.apply(action),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }
}

/// A render loop that drains actions and repaints on a fixed interval
/// until the source closes.
///
/// ```rust,ignore
/// let (tx, rx) = std::sync::mpsc::channel();
/// tracing_subscriber::registry().with(TaskLayer::new(MyMapper, tx)).init();
///
/// std::thread::spawn(move || {
///     RenderLoop::new(MyRenderer::default(), std::io::stderr())
///         .interval(Duration::from_millis(80))
///         .run(rx);
/// });
/// ```
pub struct RenderLoop<R: Renderer, W: Write> {
    renderer: TaskRenderer<R>,
    writer: W,
    interval: Duration,
    cancel_on_close: bool,
}

impl<R: Renderer, W: Write> RenderLoop<R, W> {
    /// Create a loop with sensible defaults (100 ms interval, cancel on close).
    pub fn new(renderer: R, writer: W) -> Self {
        Self::from_task_renderer(TaskRenderer::new(renderer), writer)
    }

    /// Create a loop around a configured [`TaskRenderer`].
    pub fn from_task_renderer(renderer: TaskRenderer<R>, writer: W) -> Self {
        Self {
            renderer,
            writer,
            interval: Duration::from_millis(100),
            cancel_on_close: true,
        }
    }

    /// Set the repaint interval.
    pub fn interval(mut self, d: Duration) -> Self {
        self.interval = d;
        self
    }

    /// Whether to apply [`Action::CancelAll`] when the loop ends.
    /// Enabled by default, giving a clean final frame.
    pub fn cancel_on_close(mut self, yes: bool) -> Self {
        self.cancel_on_close = yes;
        self
    }

    /// Borrow the inner [`TaskRenderer`].
    pub fn renderer(&self) -> &TaskRenderer<R> {
        &self.renderer
    }

    /// Run a single tick: drain + render. Returns `false` once the source
    /// has closed (the final drain is still rendered).
    pub fn tick(&mut self, source: &mut impl ActionSource<R::TaskData, R::EventData>) -> bool {
        let alive = source.drain_into(self.renderer.tree_mut());
        // A dropped frame is better than panicking in the render thread.
        let _ = self.renderer.render(&mut self.writer);
        alive
    }

    /// Run until the source closes. Blocks the calling thread.
    pub fn run(self, source: impl ActionSource<R::TaskData, R::EventData>) {
        self.run_until(source, || false)
    }

    /// Like [`run`](Self::run), but also exits when `stop` returns `true`.
    ///
    /// Actions still buffered in the source when the loop stops are drained
    /// into the final frame.
    pub fn run_until(
        mut self,
        mut source: impl ActionSource<R::TaskData, R::EventData>,
        stop: impl Fn() -> bool,
    ) {
        while self.tick(&mut source) && !stop() {
            std::thread::sleep(self.interval);
        }
        self.finish(&mut source);
    }

    /// Run asynchronously.
    ///
    /// `wait` is called with the interval between ticks and must return a
    /// future resolving to `true` to continue, or `false` to stop.
    ///
    /// ```rust,ignore
    /// render_loop
    ///     .run_async(rx, |d| async move {
    ///         tokio::time::sleep(d).await;
    ///         true
    ///     })
    ///     .await;
    /// ```
    pub async fn run_async<S, D, F>(mut self, mut source: S, mut wait: D)
    where
        S: ActionSource<R::TaskData, R::EventData>,
        D: FnMut(Duration) -> F,
        F: Future<Output = bool>,
    {
        while self.tick(&mut source) && wait(self.interval).await {}
        self.finish(&mut source);
    }

    fn finish(mut self, source: &mut impl ActionSource<R::TaskData, R::EventData>) {
        source.drain_into(self.renderer.tree_mut());
        if self.cancel_on_close {
            self.renderer.update(Action::CancelAll);
        }
        let _ = self.renderer.render(&mut self.writer);
    }
}
