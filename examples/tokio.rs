//! Async channel rendering with [`RenderLoop::run_async`].
//!
//! Shows the canonical async setup: closures as the [`ActionSink`] and
//! [`ActionSource`] for tokio's `mpsc`, and shutdown via a watch channel.
//! Copy this pattern for any tokio-based application.

use std::io::Write;
use std::time::Duration;

use tokio::sync::mpsc;
use trace_tally::widgets::Spinner;
use trace_tally::*;

// -- Renderer ----------------------------------------------------------------

#[derive(Default)]
struct MyRenderer {
    spinner: Spinner,
}

impl Renderer for MyRenderer {
    type EventData = String;
    type TaskData = String;

    fn on_render_start(&mut self) {
        self.spinner.tick();
    }

    fn render_task_line(
        &mut self,
        f: &mut Frame<'_>,
        task: &TaskView<'_, String, String>,
    ) -> std::io::Result<()> {
        let indent = " ".repeat(task.depth());
        if !task.active() {
            return writeln!(f, "{indent}✓ {}", task.data());
        }
        writeln!(f, "{indent}{} {}", self.spinner.frame(), task.data())
    }

    fn render_event_line(
        &mut self,
        f: &mut Frame<'_>,
        event: &EventView<'_, String, String>,
    ) -> std::io::Result<()> {
        if event.is_root() {
            writeln!(f, "{}", event.data())
        } else {
            writeln!(f, "{}  -> {}", " ".repeat(event.depth()), event.data())
        }
    }
}

// -- Tracing integration -----------------------------------------------------

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = String;
    type TaskData = String;

    fn map_event(&self, event: &tracing::Event<'_>) -> String {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        message
    }

    fn map_span(&self, attrs: &tracing::span::Attributes<'_>) -> String {
        let mut message = String::new();
        attrs.record(&mut MessageVisitor(&mut message));
        let name = attrs.metadata().name();
        format!("{name}: {message}")
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{:?}", value);
        }
    }
}

// -- Main --------------------------------------------------------------------

#[tokio::main]
async fn main() {
    use tracing::{Instrument, info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, mut rx) = mpsc::unbounded_channel();

    // Any closure works as a sink, so tokio's sender needs no wrapper.
    let layer = Mapper.layer(move |action| {
        if let Err(e) = tx.send(action) {
            eprintln!("render loop gone: {e}");
        }
    });
    tracing_subscriber::registry().with(layer).init();

    // Create a oneshot channel to signal the render loop to stop.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Create a renderer.
    let renderer = MyRenderer::default();

    // Any `FnMut(&mut TaskTree) -> bool` closure is an ActionSource.
    let source = move |tree: &mut TaskTree<String, String>| loop {
        match rx.try_recv() {
            Ok(action) => tree.apply(action),
            Err(mpsc::error::TryRecvError::Empty) => return true,
            Err(mpsc::error::TryRecvError::Disconnected) => return false,
        }
    };

    // Create a render loop that runs asynchronously.
    let render_handle = tokio::spawn(
        RenderLoop::new(renderer, std::io::stderr())
            .interval(Duration::from_millis(80))
            .cancel_on_close(true)
            .run_async(source, move |dur| {
                let mut tick_rx = shutdown_rx.clone();
                async move {
                    tokio::select! {
                        _ = tokio::time::sleep(dur) => true,
                        _ = tick_rx.changed() => false,
                    }
                }
            }),
    );

    // Simulation: concurrent async tasks.
    info!("starting pipeline");

    let t1 = tokio::spawn(
        async {
            for step in ["parsing", "type checking", "codegen", "linking"] {
                sleep(300).await;
                info!("{step}");
            }
        }
        .instrument(info_span!("compile", message = "my-project")),
    );

    let t2 = tokio::spawn(
        async {
            for step in ["pulling image", "building layers", "pushing"] {
                sleep(400).await;
                info!("{step}");
            }
        }
        .instrument(info_span!("docker", message = "app:latest")),
    );

    t1.await.unwrap();
    t2.await.unwrap();

    info!("pipeline complete");

    let _ = shutdown_tx.send(true);
    render_handle.await.unwrap();
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}
