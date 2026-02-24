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

    /// Run the loop until the source closes.
    ///
    /// Blocks the calling thread. On shutdown:
    /// 1. Drains any remaining actions.
    /// 2. Optionally sends `CancelAll`.
    /// 3. Renders one final frame.
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

// ---------------------------------------------------------------------------
// Convenience on TaskRenderer for async / custom loops
// ---------------------------------------------------------------------------

impl<R: Renderer> TaskRenderer<R> {
    /// Drain all available actions from `source` and return whether it's
    /// still alive.
    ///
    /// This is the building block for async render loops where you can't
    /// use [`RenderLoop::run`] because you need `select!` or custom timing:
    ///
    /// ```rust,ignore
    /// // In a tokio::select! loop:
    /// loop {
    ///     tokio::select! {
    ///         _ = interval.tick() => {
    ///             renderer.render(&mut stderr).unwrap();
    ///         }
    ///         action = rx.recv() => {
    ///             match action {
    ///                 Some(a) => {
    ///                     renderer.update(a);
    ///                     // drain remaining buffered actions
    ///                     while let Ok(a) = rx.try_recv() {
    ///                         renderer.update(a);
    ///                     }
    ///                     renderer.render(&mut stderr).unwrap();
    ///                 }
    ///                 None => break,
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    pub fn drain(&mut self, source: &mut impl ActionSource<R>) -> bool {
        source.drain_into(self)
    }
}
