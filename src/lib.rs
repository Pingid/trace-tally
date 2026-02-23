#![doc = include_str!("../README.md")]

pub(crate) mod task;
pub(crate) mod tracing;
pub(crate) mod writer;

#[cfg(test)]
mod test;

pub mod prelude {
    pub use crate::Renderer;
    pub use crate::task::{EventRef, TaskId};
    pub use crate::tracing::{ActionSender, EventMapper, TaskLayer, TaskTraceLayer, task_layer};
    pub use crate::writer::{EventView, Target, TaskRenderer, TaskView};
}

pub use crate::prelude::*;

/// Defines how tasks and events are rendered to the terminal.
///
/// Implement this trait to control the visual output of the task tree.
/// Each frame, the writer walks the task hierarchy and calls the render
/// methods in order: [`render_task_start`], [`render_event`] for each
/// buffered event, then [`render_task_end`], recursing into child tasks
/// between events and the end call.
///
/// [`render_task_start`]: Renderer::render_task_start
/// [`render_event`]: Renderer::render_event
/// [`render_task_end`]: Renderer::render_task_end
pub trait Renderer: Sized {
    /// Data stored per event (e.g. a log message or span field snapshot).
    type EventData: Send + 'static;
    /// Data stored per task (e.g. a task name or metadata).
    type TaskData: Send + 'static;

    /// Called once at the start of each render frame, before any tasks are visited.
    fn on_render_start(&mut self) {}

    /// Called once at the end of each render frame, after all tasks have been visited.
    fn on_render_end(&mut self) {}

    /// Adds a new event to the task's event buffer.
    /// The default implementation keeps a rolling window of the 3 most recent events.
    fn buffer_event(
        events: &mut std::collections::VecDeque<Self::EventData>, event: Self::EventData,
    ) {
        events.push_back(event);
        if events.len() > 3 {
            events.pop_front();
        }
    }

    /// Renders the opening of a task (before its events and children).
    #[allow(unused_variables)]
    fn render_task_start(
        &mut self, target: &mut Target<'_>, task: TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// Renders a single buffered event within a task.
    #[allow(unused_variables)]
    fn render_event(
        &mut self, target: &mut Target<'_>, event: EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    /// Renders the closing of a task (after its events and children).
    #[allow(unused_variables)]
    fn render_task_end(
        &mut self, target: &mut Target<'_>, task: TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Action<R: Renderer> {
    Event {
        parent: Option<TaskId>,
        event: R::EventData,
    },
    TaskStart {
        id: TaskId,
        parent: Option<TaskId>,
        event: R::TaskData,
    },
    TaskEnd {
        id: TaskId,
    },
    Finnish,
}
