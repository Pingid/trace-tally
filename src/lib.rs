#![doc = include_str!("../README.md")]

pub(crate) mod task;
pub(crate) mod tracing;
pub(crate) mod writer;

#[cfg(test)]
mod test;

/// Re-exports of all public types and traits.
pub mod prelude {
    pub use crate::Renderer;
    pub use crate::task::TaskId;
    pub use crate::tracing::{ActionTransport, TaskLayer, TaskTraceLayer, TraceMapper, task_layer};
    pub use crate::writer::{EventView, FrameWriter, TaskRenderer, TaskView};
}

use std::collections::VecDeque;

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
pub trait Renderer: Sized {
    /// Data stored per event (e.g. a log message or span field snapshot).
    type EventData: Send + 'static;
    /// Data stored per task (e.g. a task name or metadata).
    type TaskData: Send + 'static;

    /// Called once at the start of each render frame, before any tasks are visited.
    fn on_render_start(&mut self) {}

    /// Called once at the end of each render frame, after all tasks have been visited.
    fn on_render_end(&mut self) {}

    /// Controls how events are buffered per task.
    /// Override this to change the default rolling window of 3.
    fn event_buffer_strategy() -> BufferStrategy {
        BufferStrategy::default()
    }

    /// Renders a complete task and its descendants.
    ///
    /// The default implementation renders the task line, then each buffered
    /// event (skipped for completed tasks), then recurses into subtasks.
    #[allow(unused_variables)]
    fn render_task(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
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

    /// Renders the task header on task start.
    #[allow(unused_variables)]
    fn render_task_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        task: &TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// Renders a single buffered event within a task.
    #[allow(unused_variables)]
    fn render_event_line(
        &mut self,
        frame: &mut FrameWriter<'_>,
        event: &EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }
}

/// A state change in the task tree.
#[derive(Debug, Clone)]
pub enum Action<R: Renderer> {
    /// A new event on an existing task (or the root if `parent` is `None`).
    Event {
        parent: Option<TaskId>,
        data: R::EventData,
    },
    /// A new task has started.
    TaskStart {
        id: TaskId,
        parent: Option<TaskId>,
        data: R::TaskData,
    },
    /// A task has completed.
    TaskEnd { id: TaskId },
    /// Mark all pending tasks as cancelled.
    CancelAll,
}

/// Controls how events are retained in a task's event buffer.
#[derive(Debug, Clone)]
pub enum BufferStrategy {
    /// Keep only the most recent `n` events (sliding window).
    Rolling(usize),
    /// Keep only the single most recent event.
    KeepLast,
    /// Keep all events (unbounded).
    KeepAll,
    /// Don't buffer events at all.
    None,
}

impl Default for BufferStrategy {
    fn default() -> Self {
        Self::Rolling(3)
    }
}

impl BufferStrategy {
    /// Push an event into the buffer, enforcing the retention policy.
    pub(crate) fn push<T>(&self, buffer: &mut VecDeque<T>, event: T) {
        match self {
            Self::None => {}
            Self::KeepLast => {
                buffer.clear();
                buffer.push_back(event);
            }
            Self::KeepAll => {
                buffer.push_back(event);
            }
            Self::Rolling(max) => {
                buffer.push_back(event);
                while buffer.len() > *max {
                    buffer.pop_front();
                }
            }
        }
    }
}
