//! Synchronous inline rendering — the simplest way to use trace-tally.
//!
//! Demonstrates the two core extension points: [`Renderer`] (how to display)
//! and [`TraceMapper`] (how to extract data from tracing). The inline layer
//! renders synchronously on every tracing event, so no render loop is needed.

use std::io::Write;

use trace_tally::*;

// -- Renderer ----------------------------------------------------------------

// Renderer controls how tasks and events appear in the terminal.
// Its associated types must match the TraceMapper below.
struct MyRenderer;

impl Renderer for MyRenderer {
    type EventData = String;
    type TaskData = String;

    fn render_task_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        if task.completed() {
            return writeln!(frame, "{}✓ {}", " ".repeat(task.depth()), task.data());
        }
        // task.depth() gives the nesting level for indentation
        writeln!(frame, "{} {}", " ".repeat(task.depth()), task.data())
    }

    fn render_event_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> std::io::Result<()> {
        writeln!(frame, "{}  -> {}", " ".repeat(event.depth()), event.data())
    }
}

// -- Tracing integration -----------------------------------------------------

// TraceMapper bridges tracing's untyped fields into the Renderer's typed data.
struct MyMapper;
impl TraceMapper for MyMapper {
    type EventData = String;
    type TaskData = String;

    fn map_event(event: &tracing::Event<'_>) -> String {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        message
    }
    fn map_span(attrs: &tracing::span::Attributes<'_>) -> String {
        attrs.metadata().name().to_string()
    }
}

// tracing fields are visited via the Visit trait — there's no direct field access.
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

    // inline_layer renders synchronously inside the tracing call — no render
    // thread or channel needed. Good for simple one-shot output.
    let layer = MyMapper::inline_layer(MyRenderer, std::io::stderr());

    tracing_subscriber::registry().with(layer).init();

    // info_span! creates a task, in_scope runs code inside it, and dropping
    // the span closes the task.
    let span = info_span!("my_task");
    span.in_scope(|| {
        info!("working...");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        info!("done");
    });
}
