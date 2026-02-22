# trace-tally

[![Crates.io](https://img.shields.io/crates/v/trace-tally.svg)](https://crates.io/crates/trace-tally)
[![Docs.rs](https://docs.rs/trace-tally/badge.svg)](https://docs.rs/trace-tally)

A [`tracing`](https://docs.rs/tracing) layer for rendering hierarchical task tree's to the terminal.

![Trace Tally Demo](assets/demo.gif)

## Usage

Using trace-tally requires implementing two traits to control how your data is processed and displayed:

1. `EventMapper`: Extracts the relevant data from `tracing` spans and events.
2. `Renderer`: Dictates exactly how that extracted data is formatted and printed to the terminal.

### Complete Example

```rust
use std::io::Write;
use trace_tally::*;

// Define how to display spans and events
#[derive(Default)]
struct MyRenderer;

impl Renderer for MyRenderer {
    type EventData = String;
    type TaskData = String;

    fn task_start(&mut self, target: &mut Target<'_>, task: TaskView<'_, Self>) -> std::io::Result<()> {
        writeln!(target, "{} {}", " ".repeat(task.depth()), task.data())
    }

    fn event(&mut self, target: &mut Target<'_>, event: EventView<'_, Self>) -> std::io::Result<()> {
        writeln!(target, "{}  -> {}", " ".repeat(event.depth()), event.data())
    }

    fn task_end(&mut self, target: &mut Target<'_>, task: TaskView<'_, Self>) -> std::io::Result<()> {
        writeln!(target, "{}done: {}", " ".repeat(task.depth()), task.data())
    }
}

// Define how to extract data from tracing primitives
struct MyMapper;
impl EventMapper<MyRenderer> for MyMapper {
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
    use std::sync::mpsc;
    use tracing::{info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, rx) = mpsc::channel();

    // Create tracing subscriber layer
    let layer = MyMapper::layer(tx);

    // Setup tracing subscriber
    tracing_subscriber::registry().with(layer.clone()).init();

    // Render loop on a background thread
    let handle = std::thread::spawn(move || {
        let mut writer = TaskRenderer::new(MyRenderer::default());
        loop {
            while let Ok(action) = rx.try_recv() {
                let exit = action.is_exit();
                writer.update(action);
                if exit {
                    writer.render(&mut std::io::stderr()).unwrap();
                    return;
                }
            }
            writer.render(&mut std::io::stderr()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // Traced work
    let span = info_span!("my_task");
    span.in_scope(|| info!("working..."));

    layer.exit();
    handle.join().unwrap();
}
```

## API

| Type                     | Role                                                                      |
| ------------------------ | ------------------------------------------------------------------------- |
| `Renderer`               | Trait — define `TaskData`/`EventData` types and rendering callbacks.      |
| `EventMapper`            | Trait — extract custom data from tracing spans and events.                |
| `TaskRenderer`           | Receives `Action`s, manages the task tree state, and drives rendering.    |
| `TaskLayer`              | The tracing `Layer` that captures spans/events and sends actions.         |
| `Target`                 | Terminal writer with ANSI cursor control for frame clearing.              |
| `TaskView` / `EventView` | Read-only views passed to renderer callbacks to access underlying data.   |
| `Action`                 | Enum representing state changes: `TaskStart`, `Event`, `TaskEnd`, `Exit`. |
