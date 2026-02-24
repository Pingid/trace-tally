//! Aggregated rendering with a `TaskData` enum and inverted traversal.
//!
//! Unlike the previous examples, this renderer doesn't display tasks
//! individually. Instead, the `Build` variant iterates its `Crate` subtasks
//! to produce a combined progress view — similar to `cargo build` output.

use std::io::Write;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::sleep;
use trace_tally::*;
use tracing::{Instrument, info_span};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// ── Data ──────────────────────────────────────────────────────────

// Different task types enable different rendering logic.
// Build is the parent that aggregates; Crate is a leaf task.
#[derive(Clone)]
enum TaskData {
    Build,
    Crate { name: String, version: String },
}

struct Dep {
    name: &'static str,
    version: &'static str,
    duration_ms: u64,
}

// ── Mapper ────────────────────────────────────────────────────────

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = ();
    type TaskData = TaskData;

    // Dispatch on field presence: name+version → Crate, otherwise → Build.
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> TaskData {
        let mut v = FieldVisitor::default();
        attrs.record(&mut v);
        match (v.name, v.version) {
            (Some(n), Some(ver)) => TaskData::Crate {
                name: n,
                version: ver,
            },
            _ => TaskData::Build,
        }
    }

    // No events used in this example — returning () skips them entirely.
    fn map_event(_: &tracing::Event<'_>) {}
}

#[derive(Default)]
struct FieldVisitor {
    name: Option<String>,
    version: Option<String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "name" => self.name = Some(value.to_string()),
            "version" => self.version = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{:?}", value);
        let s = s.trim_matches('"').to_string();
        match field.name() {
            "name" => self.name = Some(s),
            "version" => self.version = Some(s),
            _ => {}
        }
    }
}

// ── Transport ─────────────────────────────────────────────────────

struct Transport(mpsc::UnboundedSender<Action<CargoRenderer>>);

impl ActionTransport<CargoRenderer> for Transport {
    type Error = mpsc::error::SendError<Action<CargoRenderer>>;
    fn send_action(&self, action: Action<CargoRenderer>) -> Result<(), Self::Error> {
        self.0.send(action)
    }
}

// ── Renderer ──────────────────────────────────────────────────────

#[derive(Default)]
struct CargoRenderer;

impl Renderer for CargoRenderer {
    type EventData = ();
    type TaskData = TaskData;

    fn render_task(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        match task.data() {
            // Build aggregates its Crate children into a combined view.
            TaskData::Build => render_build(frame, task),
            // Crate tasks are rendered by render_build — skip here.
            TaskData::Crate { .. } => Ok(()),
        }
    }
}

// Iterates subtasks to produce cargo-style output: completed crates listed
// above an active progress bar showing in-flight work.
fn render_build(
    frame: &mut FrameWriter<'_>,
    task: &TaskView<'_, CargoRenderer>,
) -> std::io::Result<()> {
    let mut active: Vec<String> = Vec::new();
    let total = task.subtasks().count();
    let mut done = 0;

    for sub in task.subtasks() {
        if let TaskData::Crate { name, version } = sub.data() {
            if sub.completed() || sub.cancelled() {
                done += 1;
                writeln!(frame, "\x1b[32;1m   Compiling\x1b[0m {name} v{version}",)?;
            } else {
                active.push(name.clone());
            }
        }
    }

    if total > 0 && done < total {
        let w = 25;
        let filled = (done * w) / total;
        let bar = if done == total {
            "=".repeat(w)
        } else {
            format!("{}>{}", "=".repeat(filled), " ".repeat(w - filled - 1),)
        };

        write!(
            frame,
            "\x1b[32;1m    Building\x1b[0m [{bar}] {done}/{total}"
        )?;

        if !active.is_empty() {
            let names = if active.len() > 4 {
                format!("{}...", active[..4].join(", "))
            } else {
                active.join(", ")
            };
            write!(frame, ": {names}")?;
        }
        writeln!(frame)?;
    }

    Ok(())
}

// ── Main ──────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let (tx, rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let layer = Mapper::channel_layer::<CargoRenderer, _>(Transport(tx))
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();

    let start = Instant::now();

    let render_handle = tokio::spawn(render_loop(rx, shutdown_rx));

    // Simulate cargo build: compile crates in dependency waves
    let build = info_span!("build");
    let crate_waves = waves();
    let total: usize = crate_waves.iter().map(|w| w.len()).sum();

    for wave in crate_waves {
        let mut handles = Vec::new();
        for dep in wave {
            // parent: makes crate spans children of build, so subtasks() works.
            let span = info_span!(
                parent: &build,
                "compile",
                name = dep.name,
                version = dep.version,
            );
            handles.push(tokio::spawn(
                async move {
                    sleep(Duration::from_millis(dep.duration_ms)).await;
                }
                .instrument(span),
            ));
        }
        // Waves run sequentially to simulate dependency ordering.
        for handle in handles {
            handle.await.unwrap();
        }
    }

    // Explicit drop triggers TaskEnd before the render loop exits.
    drop(build);

    let _ = shutdown_tx.send(());
    render_handle.await.unwrap();

    let elapsed = start.elapsed();
    eprintln!(
        "\x1b[32;1m    Finished\x1b[0m `release` profile [optimized] {total} crates in {:.2}s",
        elapsed.as_secs_f64(),
    );
}

/// Same drain-then-render pattern as `spinner.rs` and `tokio.rs`.
async fn render_loop(
    mut rx: mpsc::UnboundedReceiver<Action<CargoRenderer>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut renderer = TaskRenderer::new(CargoRenderer::default());
    let mut interval = tokio::time::interval(Duration::from_millis(80));

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

// ── Crate graph ───────────────────────────────────────────────────
fn waves() -> Vec<Vec<Dep>> {
    vec![
        // Wave 1 — leaf crates
        vec![
            Dep {
                name: "proc-macro2",
                version: "1.0.92",
                duration_ms: 300,
            },
            Dep {
                name: "unicode-ident",
                version: "1.0.18",
                duration_ms: 150,
            },
            Dep {
                name: "cfg-if",
                version: "1.0.0",
                duration_ms: 100,
            },
            Dep {
                name: "libc",
                version: "0.2.172",
                duration_ms: 250,
            },
            Dep {
                name: "autocfg",
                version: "1.4.0",
                duration_ms: 120,
            },
            Dep {
                name: "pin-project-lite",
                version: "0.2.16",
                duration_ms: 100,
            },
        ],
        // Wave 2 — parser foundations
        vec![
            Dep {
                name: "quote",
                version: "1.0.40",
                duration_ms: 200,
            },
            Dep {
                name: "syn",
                version: "2.0.100",
                duration_ms: 800,
            },
            Dep {
                name: "itoa",
                version: "1.0.15",
                duration_ms: 100,
            },
            Dep {
                name: "ryu",
                version: "1.0.20",
                duration_ms: 100,
            },
            Dep {
                name: "memchr",
                version: "2.7.4",
                duration_ms: 180,
            },
        ],
        // Wave 3 — proc macros
        vec![
            Dep {
                name: "serde_derive",
                version: "1.0.228",
                duration_ms: 600,
            },
            Dep {
                name: "thiserror-impl",
                version: "2.0.17",
                duration_ms: 400,
            },
            Dep {
                name: "tokio-macros",
                version: "2.5.0",
                duration_ms: 350,
            },
            Dep {
                name: "futures-macro",
                version: "0.3.31",
                duration_ms: 300,
            },
            Dep {
                name: "pin-project-internal",
                version: "1.1.10",
                duration_ms: 250,
            },
        ],
        // Wave 4 — core crates
        vec![
            Dep {
                name: "serde",
                version: "1.0.228",
                duration_ms: 500,
            },
            Dep {
                name: "thiserror",
                version: "2.0.17",
                duration_ms: 150,
            },
            Dep {
                name: "futures-core",
                version: "0.3.31",
                duration_ms: 200,
            },
            Dep {
                name: "futures-io",
                version: "0.3.31",
                duration_ms: 150,
            },
            Dep {
                name: "pin-project",
                version: "1.1.10",
                duration_ms: 150,
            },
            Dep {
                name: "bytes",
                version: "1.10.1",
                duration_ms: 200,
            },
        ],
        // Wave 5 — async ecosystem
        vec![
            Dep {
                name: "futures-util",
                version: "0.3.31",
                duration_ms: 700,
            },
            Dep {
                name: "tokio",
                version: "1.44.2",
                duration_ms: 800,
            },
            Dep {
                name: "serde_json",
                version: "1.0.140",
                duration_ms: 400,
            },
            Dep {
                name: "mio",
                version: "1.0.4",
                duration_ms: 300,
            },
        ],
        // Wave 6 — application layer
        vec![
            Dep {
                name: "hyper",
                version: "1.6.0",
                duration_ms: 500,
            },
            Dep {
                name: "reqwest",
                version: "0.12.15",
                duration_ms: 600,
            },
            Dep {
                name: "tower",
                version: "0.5.2",
                duration_ms: 350,
            },
            Dep {
                name: "my-app",
                version: "0.1.0",
                duration_ms: 400,
            },
        ],
    ]
}
