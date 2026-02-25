# trace-tally

[![Crates.io](https://img.shields.io/crates/v/trace-tally.svg)](https://crates.io/crates/trace-tally)
[![Docs.rs](https://docs.rs/trace-tally/badge.svg)](https://docs.rs/trace-tally)

A [`tracing`](https://docs.rs/tracing) layer for rendering hierarchical task trees to the terminal.

![Trace Tally Demo](assets/demo.gif)

## Usage

Using trace-tally requires implementing two traits to control how your data is processed and displayed:

1. [`TraceMapper`]: Extracts the relevant data from [`spans`](https://docs.rs/tracing/latest/tracing/#spans) and [`events`](https://docs.rs/tracing/latest/tracing/#events).
2. [`Renderer`]: Dictates exactly how that extracted data is formatted and printed to the terminal.

### Complete Example

```rust
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
```

## Channel vs Inline

trace-tally provides two layer constructors:

- **[`inline_layer`]** renders synchronously on every span/event. No background thread needed. Best for short-lived CLI tools where simplicity matters.
- **[`channel_layer`]** sends actions over an `mpsc` channel to a separate render loop. This decouples tracing from rendering, enabling timed redraws and spinner animations.

The complete example above uses [`inline_layer`]. See [`examples/render_loop.rs`](./examples/render_loop.rs) for a [`channel_layer`] setup with animated output.

Both require that the `TraceMapper` associated types match the `Renderer` associated types (`TaskData` and `EventData`). A mismatch produces a compile error on the [`inline_layer`] / [`channel_layer`] call.

[`channel_layer`] accepts any [`ActionTransport`] implementation, not just [`std::sync::mpsc::Sender`]. Implement [`ActionTransport`] to use crossbeam, tokio, or other channel backends.

## Customizing Rendering

Override [`Renderer::render_task`] to change how the task tree is walked. The default renders the task line, then buffered events (skipped for completed tasks), then recurses into subtasks:

```rust,ignore
fn render_task(
    &mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
) -> Result<(), std::io::Error> {
    self.render_task_line(f, task)?;
    if task.active() {
        for event in task.events().rev().take(3).rev() {
            self.render_event_line(f, &event)?;
        }
    }
    for subtask in task.subtasks() {
        self.render_task(f, &subtask)?;
    }
    Ok(())
}
```

## API

| Type                         | Role                                                                           |
| ---------------------------- | ------------------------------------------------------------------------------ |
| [`Renderer`]                 | Trait — define `TaskData`/`EventData` types and rendering callbacks.           |
| [`TraceMapper`]              | Trait — extract custom data from tracing spans and events.                     |
| [`TaskRenderer`]             | Receives `Action`s, manages the task tree state, and drives rendering.         |
| [`FrameWriter`]              | Terminal writer with ANSI cursor control for frame clearing.                   |
| [`TaskView`] / [`EventView`] | Read-only views passed to renderer callbacks to access underlying data.        |
| [`Action`]                   | Enum representing state changes: `TaskStart`, `Event`, `TaskEnd`, `CancelAll`. |
| [`ActionTransport`]          | Trait for channel backends — implemented for `mpsc::Sender` by default.        |
