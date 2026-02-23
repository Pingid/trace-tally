# trace-tally

[![Crates.io](https://img.shields.io/crates/v/trace-tally.svg)](https://crates.io/crates/trace-tally)
[![Docs.rs](https://docs.rs/trace-tally/badge.svg)](https://docs.rs/trace-tally)

A [`tracing`](https://docs.rs/tracing) layer for rendering hierarchical task trees to the terminal.

![Trace Tally Demo](assets/demo.gif)

## Usage

Using trace-tally requires implementing two traits to control how your data is processed and displayed:

1. `TraceMapper`: Extracts the relevant data from `tracing` spans and events.
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

    fn render_task_line(
        &mut self, frame: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
    ) -> std::io::Result<()> {
        if task.completed() {
            return writeln!(frame, "{}✓ {}", " ".repeat(task.depth()), task.data());
        }
        writeln!(frame, "{} {}", " ".repeat(task.depth()), task.data())
    }

    fn render_event_line(
        &mut self, frame: &mut FrameWriter<'_>, event: &EventView<'_, Self>,
    ) -> std::io::Result<()> {
        writeln!(frame, "{}  -> {}", " ".repeat(event.depth()), event.data())
    }
}

// Define how to extract data from tracing primitives
struct MyMapper;
impl TraceMapper<MyRenderer> for MyMapper {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

    use tracing::{info, info_span};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (tx, rx) = mpsc::channel();

    // Create tracing subscriber layer
    let layer = MyMapper::task_layer(tx.clone());

    // Setup tracing subscriber
    tracing_subscriber::registry().with(layer).init();

    // Setup signal for stopping render thread
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = stop.clone();

    // Render loop on a background thread
    let handle = std::thread::spawn(move || {
        let mut writer = TaskRenderer::new(MyRenderer::default());
        loop {
            while let Ok(action) = rx.try_recv() {
                writer.update(action);
            }
            if stop_signal.load(Ordering::Relaxed) {
                // Cancel any pending tasks
                writer.update(Action::CancelAll);
                writer.render(&mut std::io::stderr()).unwrap();
                break;
            }
            writer.render(&mut std::io::stderr()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // Traced work
    let span = info_span!("my_task");
    span.in_scope(|| info!("working..."));

    // Signal render thread to stop
    stop.store(true, Ordering::Relaxed);

    // Wait for thread to close
    handle.join().unwrap();
}
```

## Customizing Rendering

Override `render_task` to change how the task tree is walked. The default renders the task line, then buffered events (skipped for completed tasks), then recurses into subtasks:

```rust,ignore
fn render_task(
    &mut self, frame: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
) -> Result<(), std::io::Error> {
    self.render_task_line(frame, task)?;
    if !task.completed() {
        for event in task.events() {
            self.render_event_line(frame, &event)?;
        }
    }
    for subtask in task.subtasks() {
        self.render_task(frame, &subtask)?;
    }
    Ok(())
}
```

Override `push_event` to control how events are buffered per task. The default keeps a rolling window of the 3 most recent events:

```rust,ignore
fn push_event(
    events: &mut VecDeque<Self::EventData>, event: Self::EventData,
) {
    events.push_back(event);
    if events.len() > 3 {
        events.pop_front();
    }
}
```

## API

| Type                     | Role                                                                           |
| ------------------------ | ------------------------------------------------------------------------------ |
| `Renderer`               | Trait — define `TaskData`/`EventData` types and rendering callbacks.           |
| `TraceMapper`            | Trait — extract custom data from tracing spans and events.                     |
| `TaskRenderer`           | Receives `Action`s, manages the task tree state, and drives rendering.         |
| `TaskLayer`              | The tracing `Layer` that captures spans/events and sends actions.              |
| `FrameWriter`            | Terminal writer with ANSI cursor control for frame clearing.                   |
| `TaskView` / `EventView` | Read-only views passed to renderer callbacks to access underlying data.        |
| `Action`                 | Enum representing state changes: `TaskStart`, `Event`, `TaskEnd`, `CancelAll`. |
