//! Async rendering with typed event data, progress bars, and `render_task` override.
//!
//! Builds on the channel pattern from `spinner.rs`, adapted for tokio. Introduces
//! a custom [`ActionTransport`] newtype, typed `EventData` (not just `String`),
//! and a `render_task` override that suppresses event lines when a progress bar
//! is already shown on the task line.

use std::io::Write;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time;
use trace_tally::*;

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const BAR_WIDTH: usize = 20;

// -- Data types --------------------------------------------------------------

// EventData can be any type — here we use a struct to carry both a display
// message and optional typed progress fields.
struct Event {
    message: String,
    progress: Option<Progress>,
}

struct Progress {
    done: u64,
    total: u64,
}

impl Event {
    fn message(msg: String) -> Self {
        Self {
            message: msg,
            progress: None,
        }
    }

    fn progress(done: u64, total: u64) -> Self {
        Self {
            message: format!("{done}/{total} KB"),
            progress: Some(Progress { done, total }),
        }
    }
}

// -- Renderer ----------------------------------------------------------------

#[derive(Debug, Default)]
struct SpinnerRenderer {
    tick: usize,
}

impl Renderer for SpinnerRenderer {
    type EventData = Event;
    type TaskData = String;

    fn on_render_start(&mut self) {
        self.tick = (self.tick + 1) % SPINNER.len();
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

        let spinner = SPINNER[self.tick];

        // If the latest event has progress, render a bar inline
        if let Some(event) = task.events().last() {
            if let Some(p) = &event.data().progress {
                let bar = progress_bar(p.done, p.total);
                return writeln!(frame, "{} {} {} {}", indent, spinner, task.data(), bar);
            }
        }

        writeln!(frame, "{} {} {}", indent, spinner, task.data())
    }

    // Override render_task to control what appears beneath each task.
    // The default renders: task line → event lines → subtasks.
    // Here we suppress event lines when a progress bar is on the task line.
    fn render_task(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        self.render_task_line(frame, task)?;
        let has_progress = task
            .events()
            .last()
            .is_some_and(|e| e.data().progress.is_some());
        if !task.completed() && !has_progress {
            for event in task.events().rev().take(3).rev() {
                self.render_event_line(frame, &event)?;
            }
        }
        for subtask in task.subtasks() {
            self.render_task(frame, &subtask)?;
        }
        Ok(())
    }

    fn render_event_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(event.depth());
        if event.is_root() {
            writeln!(frame, "{}", event.data().message)
        } else {
            writeln!(frame, "{}  → {}", indent, event.data().message)
        }
    }
}

fn progress_bar(done: u64, total: u64) -> String {
    let ratio = done as f64 / total as f64;
    let filled = (ratio * BAR_WIDTH as f64) as usize;
    let empty = BAR_WIDTH - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

// -- Tracing integration -----------------------------------------------------

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = Event;
    type TaskData = String;

    fn map_event(event: &tracing::Event<'_>) -> Event {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        match (visitor.done, visitor.total) {
            (Some(done), Some(total)) => Event::progress(done, total),
            _ => Event::message(visitor.message.unwrap_or_default()),
        }
    }

    fn map_span(attrs: &tracing::span::Attributes<'_>) -> String {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        match visitor.message {
            Some(msg) => format!("{}: {msg}", attrs.metadata().name()),
            None => attrs.metadata().name().to_string(),
        }
    }
}

// Visitor that extracts typed numeric fields (done/total) alongside the message.
// record_u64 gives us native u64 values without parsing strings.
#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    done: Option<u64>,
    total: Option<u64>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "done" => self.done = Some(value),
            "total" => self.total = Some(value),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

// ActionTransport is generic over the channel type. The orphan rule prevents
// implementing it directly on tokio's Sender, so we use a newtype.
struct TokioTransport(mpsc::UnboundedSender<Action<SpinnerRenderer>>);

impl ActionTransport<SpinnerRenderer> for TokioTransport {
    type Error = mpsc::error::SendError<Action<SpinnerRenderer>>;
    fn send_action(&self, action: Action<SpinnerRenderer>) -> Result<(), Self::Error> {
        self.0.send(action)
    }
}

// -- Simulation --------------------------------------------------------------

async fn download_package(pkg: &Package) {
    use tracing::{Instrument, info, info_span};

    let chunks = 8u64;
    let chunk_size = pkg.size_kb / chunks;

    // .instrument() attaches the span to the future so events inside it
    // parent correctly even across await points.
    async {
        for i in 1..=chunks {
            sleep(80 + pkg.size_kb / 3).await;
            info!(done = chunk_size * i, total = pkg.size_kb, "downloading");
        }
    }
    .instrument(info_span!(
        "fetch",
        message = format!("{} v{}", pkg.name, pkg.version)
    ))
    .await;
}

#[tokio::main]
async fn main() {
    use tracing::{Instrument, info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, rx) = mpsc::unbounded_channel();
    // Oneshot channel for shutdown — cleaner than a shared flag for select!
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let layer = Mapper::channel_layer::<SpinnerRenderer, _>(TokioTransport(tx))
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();

    let render_handle = tokio::spawn(render_loop(rx, shutdown_rx));

    // Resolve phase
    info!("resolving dependencies for my-project v0.1.0");
    sleep(600).await;

    let dep_list: Vec<_> = PACKAGES
        .iter()
        .map(|p| format!("{} v{}", p.name, p.version))
        .collect();
    info!(
        "resolved {} packages: {}",
        PACKAGES.len(),
        dep_list.join(", ")
    );
    sleep(400).await;

    // Download phase — fetch all packages concurrently
    let downloads: Vec<_> = PACKAGES
        .iter()
        .map(|pkg| tokio::spawn(download_package(pkg)))
        .collect();

    async {
        for dl in downloads {
            dl.await.unwrap();
        }
    }
    .await;

    // Link phase
    async {
        for pkg in PACKAGES {
            if !pkg.deps.is_empty() {
                sleep(200).await;
                info!("{} <- {}", pkg.name, pkg.deps.join(", "));
            }
        }
    }
    .instrument(info_span!("link"))
    .await;

    let total_kb: u64 = PACKAGES.iter().map(|p| p.size_kb).sum();
    info!("installed {} packages ({total_kb} KB)", PACKAGES.len());

    let _ = shutdown_tx.send(());
    render_handle.await.unwrap();
}

/// Same drain-then-render pattern as `spinner.rs`, adapted for async with
/// `tokio::select!`. Actions are batched before each repaint.
async fn render_loop(
    mut rx: mpsc::UnboundedReceiver<Action<SpinnerRenderer>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut renderer = TaskRenderer::new(SpinnerRenderer::default());
    let mut interval = time::interval(Duration::from_millis(80));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                renderer.render(&mut std::io::stderr()).unwrap();
            }
            action = rx.recv() => {
                match action {
                    Some(action) => {
                        renderer.update(action);
                        while let Ok(action) = rx.try_recv() {
                            renderer.update(action);
                        }
                        renderer.render(&mut std::io::stderr()).unwrap();
                    }
                    None => break,
                }
            }
            _ = &mut shutdown => {
                break;
            }
        }
    }
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

// -- Mock dependency data ----------------------------------------------------

struct Package {
    name: &'static str,
    version: &'static str,
    size_kb: u64,
    deps: &'static [&'static str],
}

const PACKAGES: &[Package] = &[
    Package {
        name: "serde",
        version: "1.0.217",
        size_kb: 320,
        deps: &["serde_derive", "proc-macro2"],
    },
    Package {
        name: "serde_derive",
        version: "1.0.217",
        size_kb: 180,
        deps: &["syn", "quote"],
    },
    Package {
        name: "tokio",
        version: "1.49.0",
        size_kb: 890,
        deps: &["mio", "pin-project-lite"],
    },
    Package {
        name: "syn",
        version: "2.0.100",
        size_kb: 1240,
        deps: &["proc-macro2", "quote"],
    },
    Package {
        name: "quote",
        version: "1.0.38",
        size_kb: 85,
        deps: &["proc-macro2"],
    },
    Package {
        name: "proc-macro2",
        version: "1.0.93",
        size_kb: 120,
        deps: &[],
    },
    Package {
        name: "mio",
        version: "1.0.3",
        size_kb: 210,
        deps: &[],
    },
    Package {
        name: "pin-project-lite",
        version: "0.2.16",
        size_kb: 45,
        deps: &[],
    },
];
