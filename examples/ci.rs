//! Multi-level task hierarchy with per-variant rendering and colored event levels.
//!
//! A three-tier `TaskData` enum (Pipeline → Stage → Step) where each variant
//! controls its own subtree via `render_task` dispatch. A custom `EventData`
//! carries semantic levels for colored output. See `tokio.rs` for the async
//! channel pattern used here.

use std::io::Write;
use std::time::{Duration, Instant};

use owo_colors::OwoColorize;
use tokio::sync::mpsc;
use tokio::time::sleep;
use trace_tally::*;
use tracing::{Instrument, info_span};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// ── Constants ────────────────────────────────────────────────────

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TOTAL_STAGES: usize = 7;

// ── Data types ───────────────────────────────────────────────────

// Three hierarchy levels, each with its own rendering logic.
#[derive(Clone)]
enum TaskData {
    Pipeline { name: String },
    Stage { name: String },
    Step { name: String },
}

// Custom event type with semantic levels for colored output.
#[derive(Clone)]
struct EventData {
    level: Level,
    message: String,
}

#[derive(Clone, Copy)]
enum Level {
    Info,
    Warn,
    Error,
    Success,
}

// ── Renderer ─────────────────────────────────────────────────────

#[derive(Default)]
struct CiRenderer {
    tick: usize,
}

impl Renderer for CiRenderer {
    type EventData = EventData;
    type TaskData = TaskData;

    fn on_render_start(&mut self) {
        self.tick = (self.tick + 1) % SPINNER.len();
    }

    // Dispatch to per-variant methods so each level controls its own subtree.
    fn render_task(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        match task.data() {
            TaskData::Pipeline { .. } => self.render_pipeline(f, task),
            TaskData::Stage { .. } => self.render_stage(f, task),
            TaskData::Step { .. } => self.render_step(f, task),
        }
    }
}

impl CiRenderer {
    fn render_pipeline(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, CiRenderer>,
    ) -> std::io::Result<()> {
        let TaskData::Pipeline { name } = task.data() else {
            return Ok(());
        };
        if task.completed() {
            writeln!(f, "{} {}", "✔".green().bold(), name.bold())?;
        } else {
            writeln!(f, "{} {}", SPINNER[self.tick].magenta(), name.bold())?;
        }
        for sub in task.subtasks() {
            self.render_task(f, &sub)?;
        }
        Ok(())
    }

    fn render_stage(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, CiRenderer>,
    ) -> std::io::Result<()> {
        let TaskData::Stage { name } = task.data() else {
            return Ok(());
        };
        // task.index() gives position among siblings — used for "[1/7]" labels.
        let idx = task.index() + 1;
        let label = format!("[{idx}/{TOTAL_STAGES}]");
        if task.completed() || task.cancelled() {
            // Completed stages collapse — no subtasks or events shown.
            writeln!(f, "  {} {} {name}", "✔".green(), label.dimmed())?;
        } else {
            writeln!(
                f,
                "  {} {} {}",
                SPINNER[self.tick].magenta(),
                label.dimmed(),
                name.cyan(),
            )?;
            for sub in task.subtasks() {
                self.render_task(f, &sub)?;
            }
        }
        Ok(())
    }

    fn render_step(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, CiRenderer>,
    ) -> std::io::Result<()> {
        let TaskData::Step { name } = task.data() else {
            return Ok(());
        };
        if task.completed() || task.cancelled() {
            writeln!(f, "    {} {name}", "✔".green().dimmed())?;
        } else {
            writeln!(f, "    {} {}", SPINNER[self.tick].magenta(), name.dimmed())?;
            // Show last 3 events as a tail window (same pattern as tokio.rs).
            for event in task.events().rev().take(3).rev() {
                self.render_event(f, &event)?;
            }
            for sub in task.subtasks() {
                self.render_task(f, &sub)?;
            }
        }
        Ok(())
    }

    fn render_event(
        &mut self,
        f: &mut FrameWriter<'_>,
        event: &EventView<'_, CiRenderer>,
    ) -> std::io::Result<()> {
        let data = event.data();
        let msg = &data.message;
        match data.level {
            Level::Info => writeln!(f, "      {} {}", "│".dimmed(), msg.dimmed()),
            Level::Warn => writeln!(f, "      {} {}", "│".yellow(), msg.yellow()),
            Level::Error => writeln!(f, "      {} {}", "│".red(), msg.red().bold()),
            Level::Success => writeln!(f, "      {} {}", "│".green(), msg.green()),
        }
    }
}

// ── Mapper ───────────────────────────────────────────────────────

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = EventData;
    type TaskData = TaskData;

    // Dispatch on "kind" field to determine which TaskData variant to create.
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> TaskData {
        let mut v = FieldVisitor::default();
        attrs.record(&mut v);
        let name = v
            .name
            .unwrap_or_else(|| attrs.metadata().name().to_string());
        match v.kind.as_deref() {
            Some("pipeline") => TaskData::Pipeline { name },
            Some("stage") => TaskData::Stage { name },
            _ => TaskData::Step { name },
        }
    }

    // Maps the "level" string field to a typed Level enum for colored rendering.
    fn map_event(event: &tracing::Event<'_>) -> EventData {
        let mut v = FieldVisitor::default();
        event.record(&mut v);
        let level = match v.level.as_deref() {
            Some("warn") => Level::Warn,
            Some("error") => Level::Error,
            Some("success") => Level::Success,
            _ => Level::Info,
        };
        EventData {
            level,
            message: v.message.unwrap_or_default(),
        }
    }
}

#[derive(Default)]
struct FieldVisitor {
    kind: Option<String>,
    name: Option<String>,
    level: Option<String>,
    message: Option<String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "kind" => self.kind = Some(value.to_string()),
            "name" => self.name = Some(value.to_string()),
            "level" => self.level = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

// ── Transport ────────────────────────────────────────────────────

struct Transport(mpsc::UnboundedSender<Action<CiRenderer>>);

impl ActionTransport<CiRenderer> for Transport {
    type Error = mpsc::error::SendError<Action<CiRenderer>>;
    fn send_action(&self, action: Action<CiRenderer>) -> Result<(), Self::Error> {
        self.0.send(action)
    }
}

// ── Main ─────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let (tx, rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let layer = Mapper::channel_layer::<CiRenderer, _>(Transport(tx))
        .with_error_handler(|e| eprintln!("transport error: {e}"));

    tracing_subscriber::registry().with(layer).init();

    let start = Instant::now();
    let render_handle = tokio::spawn(render_loop(rx, shutdown_rx));

    run_pipeline().await;

    let _ = shutdown_tx.send(());
    render_handle.await.unwrap();

    print_summary(start.elapsed());
}

/// Same drain-then-render pattern as the other async examples.
async fn render_loop(
    mut rx: mpsc::UnboundedReceiver<Action<CiRenderer>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut renderer = TaskRenderer::new(CiRenderer::default());
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
            _ = &mut shutdown => break,
        }
    }
}

// ── Pipeline stages ──────────────────────────────────────────────

async fn run_pipeline() {
    // All stages become children of this span via .instrument().
    let pipeline = info_span!("pipeline", kind = "pipeline", name = "deploy-main #47");

    async {
        // Stages run sequentially — each must pass before the next starts.
        git_checkout().await;
        dependencies().await;
        lint_and_format().await;
        test_suite().await;
        build().await;
        security_scan().await;
        deploy().await;
    }
    .instrument(pipeline)
    .await;
}

async fn git_checkout() {
    let span = info_span!("git_checkout", kind = "stage", name = "Git Checkout");
    async {
        step(
            "git clone",
            &[
                (200, "info", "Cloning into 'deploy-main'..."),
                (150, "info", "remote: Enumerating objects: 2847"),
                (100, "info", "remote: Counting objects: 100% (2847/2847)"),
                (
                    200,
                    "success",
                    "Receiving objects: 100% (2847/2847), 18.3 MiB",
                ),
            ],
        )
        .await;
        step(
            "checkout branch",
            &[
                (100, "info", "Switched to branch 'main'"),
                (
                    80,
                    "info",
                    "HEAD is now at a3f7c2d feat: add pipeline config",
                ),
            ],
        )
        .await;
        step(
            "submodule sync",
            &[
                (150, "info", "Synchronizing submodule url for 'lib/core'"),
                (100, "success", "Submodules synced (2 modules)"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

async fn dependencies() {
    let span = info_span!("dependencies", kind = "stage", name = "Dependencies");
    async {
        step(
            "restore cache",
            &[
                (100, "info", "Checking cache key: cargo-v3-a3f7c2d"),
                (200, "success", "Cache restored (342 MB)"),
            ],
        )
        .await;
        step(
            "cargo fetch",
            &[
                (150, "info", "Fetching index..."),
                (100, "info", "Downloading 47 crates"),
                (200, "info", "Downloaded serde v1.0.228"),
                (100, "info", "Downloaded tokio v1.44.2"),
                (80, "success", "Fetched 47 crates in 0.6s"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

async fn lint_and_format() {
    let span = info_span!("lint", kind = "stage", name = "Lint & Format");
    async {
        step(
            "cargo fmt --check",
            &[
                (200, "info", "Checking formatting for 38 files"),
                (300, "success", "All 38 files formatted correctly"),
            ],
        )
        .await;
        step(
            "cargo clippy",
            &[
                (200, "info", "Checking trace-tally v0.0.5"),
                (150, "info", "Checking 12 dependencies"),
                (200, "warn", "warning: unused import `std::fmt` in lib.rs:4"),
                (300, "success", "Clippy: 0 errors, 1 warning"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

async fn test_suite() {
    let span = info_span!("test", kind = "stage", name = "Test Suite");
    async {
        // tokio::join! runs steps concurrently within a sequential stage.
        tokio::join!(
            step(
                "unit tests",
                &[
                    (200, "info", "Running 84 tests"),
                    (150, "info", "test renderer::tests ... ok"),
                    (100, "info", "test view::tests ... ok"),
                    (100, "info", "test tracing::tests ... ok"),
                    (200, "success", "84 passed; 0 failed (1.2s)"),
                ]
            ),
            step(
                "integration tests",
                &[
                    (300, "info", "Running 12 tests"),
                    (200, "info", "test full_pipeline ... ok"),
                    (200, "info", "test concurrent_spans ... ok"),
                    (300, "success", "12 passed; 0 failed (2.1s)"),
                ]
            ),
        );
        step(
            "coverage",
            &[
                (200, "info", "Generating coverage report"),
                (300, "info", "Instrumenting 38 source files"),
                (200, "success", "Coverage: 87.3% (lines: 1240/1420)"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

async fn build() {
    let span = info_span!("build", kind = "stage", name = "Build");
    async {
        step(
            "cargo build --release",
            &[
                (200, "info", "Compiling trace-tally v0.0.5"),
                (150, "info", "Compiling 47 dependencies"),
                (400, "info", "Optimizing with LTO"),
                (200, "success", "Compiled in 4.8s (binary: 3.2 MB)"),
            ],
        )
        .await;
        // Nested span creates a Step with its own children, demonstrating
        // that the hierarchy can go deeper than three levels.
        let docker = info_span!("docker", kind = "step", name = "docker build");
        async {
            step(
                "build image",
                &[
                    (100, "info", "Step 1/5: FROM rust:1.82-slim"),
                    (150, "info", "Step 2/5: COPY Cargo.lock Cargo.toml ./"),
                    (200, "info", "Step 3/5: RUN cargo fetch"),
                    (300, "info", "Step 4/5: COPY . ."),
                    (150, "info", "Step 5/5: RUN cargo build --release"),
                    (100, "success", "Built image app:a3f7c2d (128 MB)"),
                ],
            )
            .await;
            step(
                "push image",
                &[
                    (200, "info", "Pushing to registry.example.com/app:a3f7c2d"),
                    (300, "info", "Pushing layer 1/3 (48 MB)"),
                    (200, "info", "Pushing layer 2/3 (64 MB)"),
                    (150, "success", "Pushed app:a3f7c2d (128 MB)"),
                ],
            )
            .await;
        }
        .instrument(docker)
        .await;
    }
    .instrument(span)
    .await;
}

async fn security_scan() {
    let span = info_span!("security", kind = "stage", name = "Security Scan");
    async {
        step(
            "dependency audit",
            &[
                (200, "info", "Scanning 47 crates for known vulnerabilities"),
                (300, "info", "Checking advisory database (2024-12-01)"),
                (200, "success", "No known vulnerabilities found"),
            ],
        )
        .await;
        step(
            "container scan",
            &[
                (200, "info", "Scanning image app:a3f7c2d"),
                (150, "info", "Analyzing 142 OS packages"),
                (200, "error", "CVE-2024-6387: openssh-server 9.2 (HIGH)"),
                (150, "error", "CVE-2024-3094: xz-utils 5.6.0 (CRITICAL)"),
                (200, "warn", "2 vulnerabilities found (1 critical, 1 high)"),
            ],
        )
        .await;
        step(
            "SAST analysis",
            &[
                (200, "info", "Analyzing 38 source files"),
                (300, "info", "Running 156 security rules"),
                (200, "success", "No issues found"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

async fn deploy() {
    let span = info_span!("deploy", kind = "stage", name = "Deploy");
    async {
        step(
            "preflight checks",
            &[
                (150, "info", "Checking cluster connectivity"),
                (100, "info", "Verifying namespace: production"),
                (100, "info", "Validating deployment manifest"),
                (150, "success", "All preflight checks passed"),
            ],
        )
        .await;
        step(
            "rolling update",
            &[
                (200, "info", "Updating deployment app-server"),
                (300, "info", "Pod app-server-7f8d9 → terminating"),
                (200, "info", "Pod app-server-a3f7c → creating"),
                (250, "info", "Pod app-server-a3f7c → running"),
                (200, "success", "Rollout complete: 3/3 replicas ready"),
            ],
        )
        .await;
        step(
            "health check",
            &[
                (200, "info", "Waiting for readiness probe"),
                (300, "info", "GET /healthz → 200 OK (12ms)"),
                (200, "info", "GET /readyz → 200 OK (8ms)"),
                (150, "success", "All health checks passing"),
            ],
        )
        .await;
    }
    .instrument(span)
    .await;
}

// ── Helpers ──────────────────────────────────────────────────────

// Generic step builder: creates a Step span and replays events with delays.
// The (delay_ms, level, message) tuples drive both timing and colored output.
async fn step(name: &str, events: &[(u64, &str, &str)]) {
    use tracing::info;
    let span = info_span!("step", kind = "step", name = name);
    async {
        for &(delay_ms, level, message) in events {
            sleep(Duration::from_millis(delay_ms)).await;
            info!(level = level, "{message}");
        }
    }
    .instrument(span)
    .await;
}

fn print_summary(elapsed: Duration) {
    let bar = "━".repeat(52);
    eprintln!("\n{}", bar.dimmed());
    eprintln!("  {} Pipeline completed with warnings", "⚠".yellow().bold());
    eprintln!(
        "    {} 6 passed  {} 0 failed  {} 1 warning",
        "✔".green().bold(),
        "✘".red().bold(),
        "⚠".yellow().bold(),
    );
    eprintln!("    {} {:.1}s elapsed", "⏱".dimmed(), elapsed.as_secs_f64());
    eprintln!("{}", bar.dimmed());
}
