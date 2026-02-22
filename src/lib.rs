#![doc = include_str!("../README.md")]

pub(crate) mod task;
pub(crate) mod tracing;
pub(crate) mod writer;

#[cfg(test)]
mod test;

pub mod prelude {
    pub use crate::{
        Renderer,
        task::{EventRef, TaskId},
        tracing::{ActionSender, EventMapper, TallyLayer, TaskLayer, layer},
        writer::{EventView, Target, TaskRenderer, TaskView},
    };
}

pub use crate::prelude::*;

pub trait Renderer: Sized {
    type EventData: Send + 'static;
    type TaskData: Send + 'static;

    fn on_render_start(&mut self) {}

    #[allow(unused_variables)]
    fn task_start(
        &mut self,
        target: &mut Target<'_>,
        task: TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn event(
        &mut self,
        target: &mut Target<'_>,
        event: EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn task_end(
        &mut self,
        target: &mut Target<'_>,
        task: TaskView<'_, Self>,
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
    Exit,
}

impl<R: Renderer> Action<R> {
    pub fn is_exit(&self) -> bool {
        matches!(self, Action::Exit)
    }
}
