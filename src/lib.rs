#![cfg_attr(feature = "tracing", doc = include_str!("../README.md"))]

pub(crate) mod runner;
pub(crate) mod task;
#[cfg(feature = "tracing")]
pub(crate) mod tracing;
pub(crate) mod view;
pub(crate) mod writer;

pub mod widgets;

#[cfg(test)]
mod test;

/// Re-exports of all public types and traits.
pub mod prelude {
    pub use crate::Renderer;
    pub use crate::runner::{ActionSource, RenderLoop};
    pub use crate::task::TaskId;
    #[cfg(feature = "tracing")]
    pub use crate::tracing::*;
    pub use crate::view::{EventView, FrameWriter, TaskView};
    pub use crate::widgets::*;
    pub use crate::writer::TaskRenderer;
}

pub use crate::prelude::*;

/// Defines how tasks and events are rendered to the terminal.
///
/// Implement this trait to control the visual output of the task tree.
/// Each frame, the writer walks the task hierarchy and calls the render
/// methods in order: [`render_task_line`], [`render_event_line`] for each
/// buffered event, then recurses into child tasks. Override [`render_task`]
/// to change this traversal.
///
/// [`render_task`]: Renderer::render_task
/// [`render_task_line`]: Renderer::render_task_line
/// [`render_event_line`]: Renderer::render_event_line
///
/// # Example
///
/// ```rust
/// use trace_tally::*;
/// use std::io::Write;
///
/// struct MyRenderer;
///
/// impl Renderer for MyRenderer {
///     type EventData = String;
///     type TaskData = String;
///
///     fn render_task_line(
///         &mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
///     ) -> std::io::Result<()> {
///         writeln!(f, "{}{}", " ".repeat(task.depth()), task.data())
///     }
///
///     fn render_event_line(
///         &mut self, f: &mut FrameWriter<'_>, event: &EventView<'_, Self>,
///     ) -> std::io::Result<()> {
///         writeln!(f, "{}  {}", " ".repeat(event.depth()), event.data())
///     }
/// }
/// ```
pub trait Renderer: Sized {
    /// Data stored per event (e.g. a log message or span field snapshot).
    type EventData: Send + 'static;
    /// Data stored per task (e.g. a task name or metadata).
    type TaskData: Send + 'static;

    /// Called once at the start of each render frame, before any tasks are visited.
    fn on_render_start(&mut self) {}

    /// Called once at the end of each render frame, after all tasks have been visited.
    fn on_render_end(&mut self) {}

    /// Renders a complete task and its descendants.
    ///
    /// The default implementation renders the task line, then the last 3
    /// buffered events (skipped for completed tasks), then recurses into
    /// subtasks. Override this to change traversal order or the event cap.
    #[allow(unused_variables)]
    fn render_task(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        self.render_task_line(f, task)?;
        if !task.completed() {
            for event in task.events().rev().take(3).rev() {
                self.render_event_line(f, &event)?;
            }
        }
        for subtask in task.subtasks() {
            self.render_task(f, &subtask)?;
        }
        Ok(())
    }

    /// Renders the task header on task start.
    #[allow(unused_variables)]
    fn render_task_line(
        &mut self,
        f: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// Renders a single buffered event within a task.
    #[allow(unused_variables)]
    fn render_event_line(
        &mut self,
        f: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }
}

/// A state change in the task tree.
///
/// # Example
///
/// ```rust
/// use trace_tally::*;
///
/// struct MyRenderer;
/// impl Renderer for MyRenderer {
///     type EventData = String;
///     type TaskData = String;
/// }
///
/// let mut renderer = TaskRenderer::new(MyRenderer);
/// renderer.update(Action::TaskStart {
///     id: TaskId::from(2),
///     parent: None,
///     data: "my task".into(),
/// });
/// renderer.render(&mut std::io::stderr()).unwrap();
/// ```
#[derive(Debug, Clone)]
pub enum Action<R: Renderer> {
    /// A new event on an existing task (or the root if `parent` is `None`).
    Event {
        parent: Option<TaskId>,
        data: R::EventData,
    },
    /// A new task has started.
    ///
    /// If `parent` is `None` or refers to an unknown ID, the task is
    /// attached to the virtual root.
    TaskStart {
        id: TaskId,
        parent: Option<TaskId>,
        data: R::TaskData,
    },
    /// A task has completed.
    TaskEnd { id: TaskId },
    /// Mark all pending tasks as cancelled.
    ///
    /// Walks every task reachable from root and sets them as cancelled.
    /// Cancelled tasks still render (via [`TaskView::cancelled`]) but are
    /// flushed from the active frame on the next render, like completed tasks.
    CancelAll,
}
