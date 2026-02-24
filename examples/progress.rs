//! Typed event data with progress bars and `render_task` override.
//!
//! Shows that `EventData` can be any type — here a struct carrying typed
//! progress fields. Uses [`ProgressBar`] and [`Spinner`] utilities, and
//! overrides `render_task` to suppress event lines when a bar is visible.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use trace_tally::util::{ProgressBar, Spinner};
use trace_tally::*;

// -- Data types --------------------------------------------------------------

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

struct MyRenderer {
    spinner: Spinner,
}

impl Renderer for MyRenderer {
    type EventData = Event;
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

        let spinner = self.spinner.frame();

        // Show a progress bar inline when the latest event has progress data.
        let progress = task.events().last().and_then(|e| {
            let p = e.data().progress.as_ref()?;
            Some((p.done, p.total))
        });
        if let Some((done, total)) = progress {
            let bar = ProgressBar::new(done, total);
            return writeln!(frame, "{indent}{spinner} {} {bar}", task.data());
        }

        writeln!(frame, "{indent}{spinner} {}", task.data())
    }

    // Override render_task to suppress event lines when a progress bar is shown.
    fn render_task(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
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
    ) -> std::io::Result<()> {
        if event.is_root() {
            writeln!(frame, "{}", event.data().message)
        } else {
            writeln!(
                frame,
                "{}  -> {}",
                " ".repeat(event.depth()),
                event.data().message
            )
        }
    }
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

// record_u64 gives native u64 values for done/total without string parsing.
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

// -- Simulation --------------------------------------------------------------

async fn download_package(pkg: &Package) {
    use tracing::{Instrument, info, info_span};

    let chunks = 8u64;
    let chunk_size = pkg.size_kb / chunks;

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

    // std::sync::mpsc implements ActionTransport, so no newtype needed.
    let (tx, rx) = std::sync::mpsc::channel();

    let layer = Mapper::channel_layer::<MyRenderer, _>(tx)
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    // Render on a std thread while the async simulation runs on tokio.
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

    for dl in downloads {
        dl.await.unwrap();
    }

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

    stop.store(true, Ordering::Relaxed);
    render_thread.join().unwrap();
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
