//! Channel-based rendering with [`RenderLoop`] and [`Spinner`] animation.
//!
//! Decouples tracing from rendering via `mpsc::channel`. A dedicated thread
//! runs `RenderLoop`, which handles draining, repainting, and shutdown
//! automatically — no hand-rolled loop needed.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use trace_tally::util::Spinner;
use trace_tally::*;

// -- Renderer ----------------------------------------------------------------

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
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        let indent = " ".repeat(task.depth());
        if task.completed() {
            return writeln!(frame, "{indent}✓ {}", task.data());
        }
        writeln!(frame, "{indent}{} {}", self.spinner.frame(), task.data())
    }

    fn render_event_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> std::io::Result<()> {
        if event.is_root() {
            writeln!(frame, "{}", event.data())
        } else {
            writeln!(frame, "{}  -> {}", " ".repeat(event.depth()), event.data())
        }
    }
}

// -- Tracing integration -----------------------------------------------------

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = String;
    type TaskData = String;

    fn map_event(event: &tracing::Event<'_>) -> String {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        message
    }

    fn map_span(attrs: &tracing::span::Attributes<'_>) -> String {
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

fn main() {
    use tracing::{info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, rx) = mpsc::channel();

    let layer = Mapper::channel_layer::<MyRenderer, _>(tx)
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    // RenderLoop handles drain → render → sleep → shutdown in one call.
    let render_thread = std::thread::spawn(move || {
        RenderLoop::new(
            MyRenderer {
                spinner: Spinner::dots(),
            },
            std::io::stderr(),
        )
        .interval(Duration::from_millis(80))
        .run_until(rx, || stop_flag.load(Ordering::Relaxed));
    });

    // Simulation: concurrent tasks from multiple threads.
    info!("starting build pipeline");

    let t1 = std::thread::spawn(|| {
        let span = info_span!("compile");
        for step in [
            "parsing sources",
            "type checking",
            "generating IR",
            "linking",
        ] {
            sleep(400);
            span.in_scope(|| info!("{step}"));
        }
    });

    let t2 = std::thread::spawn(|| {
        let span = info_span!("docker build");
        for step in [
            "pulling base image",
            "layer 1/3: deps",
            "layer 2/3: build",
            "layer 3/3: assets",
        ] {
            sleep(350);
            span.in_scope(|| info!("{step}"));
        }
    });

    let t3 = std::thread::spawn(|| {
        let root = info_span!("deploy");
        for env in ["staging", "production"] {
            let child = info_span!(parent: &root, "deploy", message = env);
            for step in ["preflight checks", "swapping containers", "health check"] {
                sleep(500);
                child.in_scope(|| info!("{step}"));
            }
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();
    t3.join().unwrap();

    stop.store(true, Ordering::Relaxed);
    render_thread.join().unwrap();
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
