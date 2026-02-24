//! Channel-based rendering with spinner animation and nested tasks.
//!
//! Unlike the inline example, `channel_layer` decouples tracing from rendering
//! via a channel. A dedicated thread runs a render loop that drains pending
//! actions and repaints at fixed intervals, enabling smooth animation.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use trace_tally::*;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// -- Renderer ----------------------------------------------------------------

#[derive(Debug, Default)]
struct SpinnerRenderer {
    tick: usize,
}

impl Renderer for SpinnerRenderer {
    type EventData = String;
    type TaskData = String;

    // Called once per frame — use for animation state like spinner position.
    fn on_render_start(&mut self) {
        self.tick = (self.tick + 1) % SPINNER_FRAMES.len();
    }

    fn render_task_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        if task.completed() {
            return writeln!(frame, "{}✓ {}", indent, task.data());
        }
        let spinner = SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()];
        writeln!(frame, "{} {} {}", indent, spinner, task.data())
    }

    fn render_event_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        // Events outside any span belong to the virtual root task.
        if event.is_root() {
            writeln!(frame, "{}", event.data())
        } else {
            let indent = " ".repeat(event.depth());
            writeln!(frame, "{}   -> {}", indent, event.data())
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
        let name = attrs.metadata().name().to_string();
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

    // channel_layer sends actions through tx instead of rendering inline.
    let (tx, rx) = mpsc::channel();

    let layer = Mapper::channel_layer::<SpinnerRenderer, _>(tx)
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();

    // Shared flag to signal the render thread to exit.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = stop.clone();

    let handle = std::thread::spawn(move || {
        let mut renderer = TaskRenderer::new(SpinnerRenderer::default());

        loop {
            // Drain all pending actions before rendering to avoid partial frames.
            while let Ok(action) = rx.try_recv() {
                renderer.update(action);
            }
            renderer.render(&mut std::io::stderr()).unwrap();
            if stop_signal.load(Ordering::Relaxed) {
                // Mark remaining tasks as cancelled for a clean final frame.
                renderer.update(Action::CancelAll);
                renderer.render(&mut std::io::stderr()).unwrap();
                break;
            }
            sleep(200);
        }
    });

    // Simulation: emit tracing events from multiple threads
    info!("loading config from deploy.toml");
    sleep(100);
    info!("resolved 14 packages in 0.3s");
    sleep(100);
    info!("starting build pipeline");

    let task1 = std::thread::spawn(move || {
        let span = info_span!("compile");
        let steps = [
            "parsing source files",
            "resolving dependencies",
            "type checking",
            "generating IR",
            "optimizing (level 2)",
            "linking objects",
            "emitting binary (2.4 MB)",
        ];
        for step in steps {
            sleep(500);
            span.in_scope(|| info!("{step}"));
        }
    });

    let task2 = std::thread::spawn(move || {
        let span = info_span!("docker build");
        let steps = [
            "pulling base image rust:1.82-slim",
            "layer 1/5: installing system deps",
            "layer 2/5: copying lockfile",
            "layer 3/5: fetching crate registry",
            "layer 4/5: compiling release binary",
            "layer 5/5: copying assets",
            "tagging image app:a3f7c2d",
            "pushing to registry.example.com",
        ];
        for step in steps {
            sleep(400);
            span.in_scope(|| info!("{step}"));
        }
    });

    let task3 = std::thread::spawn(move || {
        let root_span = info_span!("deploy");
        let environments = ["staging", "canary", "production"];

        for env in environments {
            sleep(600);
            root_span.in_scope(|| info!("preparing {env} rollout"));

            // parent: creates a nested task hierarchy under root_span.
            let child_span = info_span!(parent: &root_span, "deploy", message = format!("{env}"));
            let steps = [
                "running preflight checks",
                "draining existing connections",
                "swapping containers",
                "waiting for health check",
            ];

            for step in steps {
                sleep(800);
                child_span.in_scope(|| info!("{step}"));
            }

            child_span.in_scope(|| info!("{env} is live"));
        }
    });

    task1.join().unwrap();
    task2.join().unwrap();
    task3.join().unwrap();

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
}

fn sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
