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
        f: &mut Frame<'_>,
        task: &TaskView<'_, String, String>,
    ) -> std::io::Result<()> {
        write!(f, "{}", " ".repeat(task.depth()))?;
        if task.completed() {
            write!(f, "✓ ")?;
        }
        writeln!(f, "{}", task.data())
    }

    fn render_event_line(
        &mut self,
        f: &mut Frame<'_>,
        event: &EventView<'_, String, String>,
    ) -> std::io::Result<()> {
        writeln!(f, "{}  -> {}", " ".repeat(event.depth()), event.data())
    }
}

// Define how to extract data from tracing primitives
struct MyMapper;
impl TraceMapper for MyMapper {
    type EventData = String;
    type TaskData = String;

    fn map_event(&self, event: &tracing::Event<'_>) -> String {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        message
    }
    fn map_span(&self, attrs: &tracing::span::Attributes<'_>) -> String {
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
    let layer = MyMapper.inline_layer(MyRenderer, std::io::stderr());

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

## Architecture

The crate is split into three independent layers, each usable on its own:

```text
 TaskLayer ──Action<T, E>──▶ ActionSink ──▶ TaskTree<T, E> ──TaskView──▶ Renderer ──▶ Frame
 (capture)                  (transport)       (model)                    (draw)       (output)
```

- [`model`]: [`Action`], [`TaskTree`], [`TaskView`] and [`EventView`]. Plain data, generic over your task data `T` and event data `E`, with no dependency on `tracing` or rendering.
- [`layer`]: [`TaskLayer`] maps spans and events with a [`TraceMapper`] and hands [`Action`]s to any [`ActionSink`].
- [`render`]: the [`Renderer`] trait, [`LiveRegion`] for erasable inline terminal output, [`TaskRenderer`] which combines the two with a tree, and [`RenderLoop`].

## Delivering actions

[`TaskLayer::new`] takes any [`ActionSink`]. These are implemented out of the box:

- **Closures**: `TaskLayer::new(MyMapper, |action| ...)` covers tokio, crossbeam, flume, a test `Vec`, or anything else.
- **[`std::sync::mpsc::Sender`]**: `MyMapper.layer(tx)`, paired with a [`RenderLoop`] on another thread. This decouples tracing from rendering and enables timed redraws and spinner animations. See [`examples/render_loop.rs`](./examples/render_loop.rs).
- **[`InlineSink`]**: `MyMapper.inline_layer(renderer, writer)` renders synchronously on every span and event, with no background thread. Best for short-lived CLI tools.

Rendering is optional. To use the tree with another backend (ratatui, a web view, snapshot tests), apply actions to a [`TaskTree`] yourself and read it through [`TaskView`]s.

## Updating tasks

Implement [`TraceMapper::map_record`] to update task data when fields are recorded on an open span (`span.record("progress", 42)`). The layer emits [`Action::TaskUpdate`] with the new data.

## Customizing Rendering

Override [`Renderer::render_task`] to change how the task tree is walked. The default renders the task line, then buffered events (skipped for inactive tasks), then recurses into subtasks:

```rust,ignore
fn render_task(
    &mut self, f: &mut Frame<'_>, task: &TaskView<'_, String, String>,
) -> std::io::Result<()> {
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

Set [`TaskRenderer::width`] to the terminal width so lines that soft-wrap are erased correctly between frames.

## API

| Type                         | Role                                                                                   |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| [`Action`]                   | State changes: `TaskStart`, `TaskUpdate`, `Event`, `TaskEnd`, `CancelAll`.             |
| [`TaskTree`]                 | Applies actions and holds the task hierarchy.                                          |
| [`TaskView`] / [`EventView`] | Read-only views into the tree.                                                         |
| [`TraceMapper`]              | Trait: extract custom data from tracing spans and events.                              |
| [`TaskLayer`]                | The `tracing` layer; delivers actions to an [`ActionSink`].                            |
| [`ActionSink`]               | Where the layer sends actions: closures, `mpsc::Sender`, [`InlineSink`].               |
| [`Renderer`]                 | Trait: rendering callbacks for tasks and events.                                       |
| [`TaskRenderer`]             | Combines a tree, a renderer and a [`LiveRegion`] into an inline terminal display.      |
| [`LiveRegion`] / [`Frame`]   | Erasable inline terminal region and the writer passed to renderer callbacks.           |
| [`RenderLoop`]               | Drains an [`ActionSource`] and repaints on an interval (sync or async).                |
