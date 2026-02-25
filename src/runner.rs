use std::io::Write;
use std::time::Duration;

use crate::{Action, Renderer, TaskRenderer};

/// Drain available actions from a channel or queue into a [`TaskRenderer`].
///
/// Returns `true` while the source is still alive (senders exist),
/// `false` once all senders have been dropped / the channel is closed.
///
/// Implement this for custom channel backends (crossbeam, flume, etc.).
/// For tokio, see the async example below.
pub trait ActionSource<R: Renderer> {
    /// Drain all immediately-available actions into `renderer`.
    ///
    /// Must not block — return as soon as the channel is empty.
    /// Returns `false` when the source is permanently closed.
    fn drain_into(&mut self, renderer: &mut TaskRenderer<R>) -> bool;
}

impl<R: Renderer> ActionSource<R> for std::sync::mpsc::Receiver<Action<R>> {
    fn drain_into(&mut self, renderer: &mut TaskRenderer<R>) -> bool {
        use std::sync::mpsc::TryRecvError;
        loop {
            match self.try_recv() {
                Ok(action) => renderer.update(action),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    // Channel closed — but still drain any remaining buffered items.
                    // (mpsc guarantees no more after Disconnected, so just return.)
                    return false;
                }
            }
        }
    }
}

/// A self-contained render loop that drains actions and repaints on a fixed
/// interval until the source closes.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::mpsc;
/// use std::time::Duration;
///
/// let (tx, rx) = mpsc::channel();
/// let layer = Mapper::channel_layer::<MyRenderer, _>(tx);
/// tracing_subscriber::registry().with(layer).init();
///
/// // The entire render thread becomes one line:
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
    pub fn new(r: R, writer: W) -> Self {
        Self {
            renderer: TaskRenderer::new(r),
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

    /// Whether to send [`Action::CancelAll`] when the source closes.
    /// Enabled by default — gives you a clean "all done" final frame.
    pub fn cancel_on_close(mut self, yes: bool) -> Self {
        self.cancel_on_close = yes;
        self
    }

    /// Borrow the inner [`TaskRenderer`] (e.g. to snapshot state).
    pub fn renderer(&self) -> &TaskRenderer<R> {
        &self.renderer
    }

    /// Run the loop until the source closes. Blocks the calling thread.
    ///
    /// Each cycle drains all buffered actions, renders a frame, then sleeps
    /// for [`interval`](Self::interval). When the source closes, one final
    /// drain + render occurs, preceded by [`Action::CancelAll`] if
    /// [`cancel_on_close`](Self::cancel_on_close) is enabled.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// std::thread::spawn(move || {
    ///     RenderLoop::new(MyRenderer::default(), std::io::stderr())
    ///         .interval(Duration::from_millis(80))
    ///         .run(rx);
    /// });
    ///
    /// // Dropping all senders closes the channel and stops the loop.
    /// drop(tx);
    /// ```
    pub fn run(mut self, mut source: impl ActionSource<R>) {
        loop {
            let alive = source.drain_into(&mut self.renderer);
            // Ignore render errors — stderr can't really fail in practice,
            // and panicking in the render thread is worse than a dropped frame.
            let _ = self.renderer.render(&mut self.writer);
            if !alive {
                break;
            }
            std::thread::sleep(self.interval);
        }

        if self.cancel_on_close {
            self.renderer.update(Action::CancelAll);
            let _ = self.renderer.render(&mut self.writer);
        }
    }

    /// Like [`run`](Self::run), but also exits when `stop` returns `true`.
    ///
    /// The predicate is checked after each drain-and-render cycle,
    /// so the final frame always reflects the latest actions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// let stop = Arc::new(AtomicBool::new(false));
    /// let flag = stop.clone();
    ///
    /// std::thread::spawn(move || {
    ///     RenderLoop::new(MyRenderer::default(), std::io::stderr())
    ///         .run_until(rx, || flag.load(Ordering::Relaxed));
    /// });
    ///
    /// // Later, signal the render loop to stop:
    /// stop.store(true, Ordering::Relaxed);
    /// ```
    pub fn run_until(mut self, mut source: impl ActionSource<R>, stop: impl Fn() -> bool) {
        loop {
            let alive = source.drain_into(&mut self.renderer);
            let _ = self.renderer.render(&mut self.writer);
            if !alive || stop() {
                break;
            }
            std::thread::sleep(self.interval);
        }
        if self.cancel_on_close {
            self.renderer.update(Action::CancelAll);
            let _ = self.renderer.render(&mut self.writer);
        }
    }

    /// Run the loop asynchronously.
    ///
    /// The `wait_fn` closure must return a future that resolves to `true` to
    /// continue the loop, or `false` to abort immediately.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use tokio::sync::oneshot;
    /// use std::time::Duration;
    ///
    /// let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    ///
    /// render_loop.run_async(source, move |duration| {
    ///     let mut rx = cancel_rx.clone();
    ///     async move {
    ///         tokio::select! {
    ///             _ = tokio::time::sleep(duration) => true, // Normal tick, continue
    ///             _ = rx.changed() => false,                // Cancelled, abort!
    ///         }
    ///     }
    /// }).await;
    /// ```
    pub async fn run_async<S, D, F>(mut self, mut source: S, mut wait_fn: D)
    where
        S: ActionSource<R>,
        D: FnMut(Duration) -> F,
        F: Future<Output = bool>,
    {
        loop {
            let alive = source.drain_into(&mut self.renderer);
            let _ = self.renderer.render(&mut self.writer);

            if !alive {
                break;
            }

            if !wait_fn(self.interval).await {
                break;
            }
        }

        if self.cancel_on_close {
            self.renderer.update(Action::CancelAll);
            let _ = self.renderer.render(&mut self.writer);
        }
    }

    /// Run a single tick: drain + render. Returns `false` when the source
    /// has closed (but still renders that final drain).
    ///
    /// Use this if you need a custom outer loop (e.g. checking additional
    /// shutdown conditions) but still want the drain-then-render logic.
    pub fn tick(&mut self, source: &mut impl ActionSource<R>) -> bool {
        let alive = source.drain_into(&mut self.renderer);
        let _ = self.renderer.render(&mut self.writer);
        alive
    }
}
