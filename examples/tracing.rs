use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use trace_tally::{
    Action, EventView, FrameWriter, Renderer, TaskRenderer, TaskTraceLayer, TaskView, TraceMapper,
};

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Default)]
struct TestRenderer {
    tick: usize,
}

impl Renderer for TestRenderer {
    type EventData = String;
    type TaskData = String;

    fn on_render_start(&mut self) {
        self.tick = (self.tick + 1) % SPINNER_FRAMES.len();
    }

    fn render_task(
        &mut self, target: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        self.render_task_line(target, task)?;
        if !task.completed() {
            for event in task.events() {
                self.render_event_line(target, &event)?;
            }
        }
        for subtask in task.subtasks() {
            self.render_task_line(target, &subtask)?;
        }
        Ok(())
    }

    fn render_task_line(
        &mut self, target: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        if task.completed() {
            return writeln!(target, "{}✓ {}", indent, task.data());
        }
        let frame = SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()];
        writeln!(target, "{} {} {}", indent, frame, task.data())
    }

    fn render_event_line(
        &mut self, target: &mut FrameWriter<'_>, event: &EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        if event.is_root() {
            writeln!(target, "{}", event.data())
        } else {
            let indent = " ".repeat(event.depth());
            writeln!(target, "{}   -> {}", indent, event.data())
        }
    }
}

struct TestTraceMapper;

impl TraceMapper<TestRenderer> for TestTraceMapper {
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

fn main() {
    use tracing::{info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, rx) = mpsc::channel();

    let layer = TestTraceMapper::task_layer(tx.clone());
    tracing_subscriber::registry().with(layer.clone()).init();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = stop.clone();

    let handle = std::thread::spawn(move || {
        let mut writer = TaskRenderer::new(TestRenderer::default());

        loop {
            while let Ok(action) = rx.try_recv() {
                writer.update(action);
            }
            writer.render(&mut std::io::stderr()).unwrap();
            if stop_signal.load(Ordering::Relaxed) {
                writer.update(Action::CancelAll);
                writer.render(&mut std::io::stderr()).unwrap();
                break;
            }
            sleep(200);
        }
    });

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
