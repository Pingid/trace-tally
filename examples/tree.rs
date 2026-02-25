//! Hierarchical output with [`TreeIndent`] box-drawing characters.
//!
//! Uses the inline layer to focus on rendering. `TreeIndent::of(task)` walks
//! the task ancestry to produce `├──`, `└──`, and `│` prefixes automatically.

use std::io::Write;

use trace_tally::widgets::TreeIndent;
use trace_tally::*;

// -- Renderer ----------------------------------------------------------------

struct TreeRenderer;

impl Renderer for TreeRenderer {
    type EventData = String;
    type TaskData = String;

    fn render_task_line(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        let prefix = TreeIndent::of(task);
        if task.completed() {
            writeln!(f, "{prefix}✓ {}", task.data())
        } else {
            writeln!(f, "{prefix}{}", task.data())
        }
    }

    fn render_event_line(
        &mut self,
        _f: &mut FrameWriter<'_>,
        _event: &EventView<'_, Self>,
    ) -> std::io::Result<()> {
        Ok(()) // suppress events — tree structure is the focus
    }
}

// -- Tracing integration -----------------------------------------------------

struct Mapper;

impl TraceMapper for Mapper {
    type EventData = String;
    type TaskData = String;

    fn map_event(event: &tracing::Event<'_>) -> String {
        let mut msg = String::new();
        event.record(&mut MessageVisitor(&mut msg));
        msg
    }

    fn map_span(attrs: &tracing::span::Attributes<'_>) -> String {
        let mut msg = String::new();
        attrs.record(&mut MessageVisitor(&mut msg));
        match msg.is_empty() {
            true => attrs.metadata().name().to_string(),
            false => msg,
        }
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
    use tracing::info_span;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let layer = Mapper::inline_layer(TreeRenderer, std::io::stderr());
    tracing_subscriber::registry().with(layer).init();

    // Simulate a test suite with hierarchical spans.
    let suite = info_span!("test suite");
    suite.in_scope(|| {
        let auth = info_span!(parent: &suite, "auth");
        auth.in_scope(|| {
            run_test(&auth, "login with valid credentials");
            run_test(&auth, "reject expired token");
            run_test(&auth, "refresh token rotation");
        });

        let api = info_span!(parent: &suite, "api");
        api.in_scope(|| {
            run_test(&api, "GET /users returns 200");
            run_test(&api, "POST /users validates body");

            let nested = info_span!(parent: &api, "pagination");
            nested.in_scope(|| {
                run_test(&nested, "default page size");
                run_test(&nested, "cursor-based navigation");
            });
        });

        let db = info_span!(parent: &suite, "database");
        db.in_scope(|| {
            run_test(&db, "migrations run in order");
            run_test(&db, "rollback on failure");
        });
    });
}

fn run_test(parent: &tracing::Span, name: &str) {
    use tracing::info_span;
    let span = info_span!(parent: parent, "test", message = name);
    let _guard = span.enter();
    std::thread::sleep(std::time::Duration::from_millis(100));
    // Each test is just a leaf span — entering and dropping it shows the tree.
}
