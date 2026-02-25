use std::io::Write;
use trace_tally::*;

// Define how to display spans and events
struct MyRenderer;
impl Renderer for MyRenderer {
    type EventData = String;
    type TaskData = String;

    fn render_task_line(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        write!(f, "{}", " ".repeat(task.depth()))?;
        if task.completed() {
            write!(f, "✓ ")?;
        }
        writeln!(f, "{}", task.data())
    }

    fn render_event_line(
        &mut self,
        f: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> std::io::Result<()> {
        writeln!(f, "{}  -> {}", " ".repeat(event.depth()), event.data())
    }
}

// Define how to extract data from tracing primitives
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

struct MessageVisitor<'a>(&'a mut String);
impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{:?}", value);
        }
    }
}

// Setup render loop
fn main() {
    use tracing::{info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Create tracing subscriber layer
    let layer = MyMapper::inline_layer(MyRenderer, std::io::stderr());

    // Setup tracing subscriber
    tracing_subscriber::registry().with(layer).init();

    // Traced work
    let span = info_span!("my_task");
    span.in_scope(|| {
        info!("working...");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        info!("done");
    });
}
