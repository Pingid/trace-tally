use std::io::Write;
use std::time::Duration;

use crate::task::{EventIndex, Task, TaskStore};
use crate::{Renderer, TaskId};

/// Write target with ANSI cursor control for frame clearing.
///
/// Wraps an [`std::io::Write`] target. Use `write!` / `writeln!` to produce
/// output within renderer callbacks.
///
/// ```rust,ignore
/// fn render_task_line(
///     &mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
/// ) -> std::io::Result<()> {
///     writeln!(f, "Task: {}", task.data())
/// }
/// ```
pub struct FrameWriter<'a> {
    target: &'a mut dyn Write,
    frame_lines: usize,
}

impl<'a> FrameWriter<'a> {
    pub(crate) fn new(target: &'a mut dyn Write, frame_lines: usize) -> Self {
        Self {
            target,
            frame_lines,
        }
    }

    pub(crate) fn clear_frame(&mut self) -> Result<(), std::io::Error> {
        let lines_drawn = self.frame_lines;
        if lines_drawn > 0 {
            write!(self, "\r\x1b[{}A\x1b[2K\x1b[J", lines_drawn).unwrap();
            self.target.flush()?;
        }
        self.frame_lines = 0;
        Ok(())
    }

    pub(crate) fn frame_lines(&self) -> usize {
        self.frame_lines
    }
}

impl<'a> Write for FrameWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let newlines = buf.iter().filter(|&&b| b == b'\n').count();
        self.frame_lines += newlines;
        self.target.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.target.flush()
    }
}

/// Read-only view of a task, passed to [`Renderer`] callbacks.
///
/// ```rust,ignore
/// fn render_task_line(
///     &mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>,
/// ) -> std::io::Result<()> {
///     let prefix = if task.active() { ".." } else { "done" };
///     writeln!(f, "[{}] {} (depth={})", prefix, task.data(), task.depth())
/// }
/// ```
#[derive(Clone, Copy)]
pub struct TaskView<'a, R: Renderer> {
    id: TaskId,
    tasks: &'a TaskStore<R>,
}

impl<'a, R: Renderer> TaskView<'a, R> {
    /// Creates a view over the task with the given `id`.
    pub fn new(tasks: &'a TaskStore<R>, id: TaskId) -> Self {
        Self { id, tasks }
    }

    /// Returns TaskId of this task.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Returns the user-defined data stored on this task.
    pub fn data(&self) -> &R::TaskData {
        self.tasks.task(&self.id).data.as_ref().unwrap()
    }

    /// Returns the nesting depth of this task (root children are depth 1).
    pub fn depth(&self) -> usize {
        self.tasks.task(&self.id).depth
    }

    /// How long since this task started.
    pub fn elapsed(&self) -> Duration {
        self.tasks.task(&self.id).started_at.elapsed()
    }

    /// Returns `true` if the task is active (not completed or cancelled).
    pub fn active(&self) -> bool {
        !self.completed() && !self.cancelled()
    }

    /// Returns `true` if the task's span has closed.
    pub fn completed(&self) -> bool {
        self.tasks.task(&self.id).completed
    }

    /// Returns `true` if the task was marked cancelled by [`crate::Action::CancelAll`].
    pub fn cancelled(&self) -> bool {
        self.tasks.task(&self.id).cancelled
    }

    /// Returns the parent task, or `None` for root-level tasks.
    pub fn parent<'b>(&'b self) -> Option<TaskView<'b, R>> {
        self.tasks
            .task(&self.id)
            .parent
            .map(|id| TaskView::new(self.tasks, id))
    }

    /// Returns an iterator over the task's buffered events.
    pub fn events<'b>(
        &'b self,
    ) -> impl DoubleEndedIterator<Item = EventView<'b, R>> + ExactSizeIterator {
        (0..self.tasks.task(&self.id).events.len())
            .map(move |id| EventView::new(self.tasks, self.id, id))
    }

    /// Returns an iterator over the task's direct children.
    pub fn subtasks<'b>(
        &'b self,
    ) -> impl DoubleEndedIterator<Item = TaskView<'b, R>> + ExactSizeIterator {
        self.tasks
            .task(&self.id)
            .subtasks
            .iter()
            .map(move |id| TaskView::new(self.tasks, *id))
    }

    /// Returns the position of this task among its parent's children.
    pub fn index(&self) -> usize {
        let parent = self.tasks.task(&self.id).parent.unwrap();
        let parent = self.tasks.task(&parent);
        parent.subtasks.get_index_of(&self.id).unwrap()
    }

    /// Create a view of another task in the same tree.
    pub fn view(&self, id: TaskId) -> TaskView<'a, R> {
        TaskView::new(self.tasks, id)
    }
}

/// Read-only view of a buffered event, passed to [`Renderer::render_event_line`].
///
/// ```rust,ignore
/// fn render_event_line(
///     &mut self, f: &mut FrameWriter<'_>, event: &EventView<'_, Self>,
/// ) -> std::io::Result<()> {
///     writeln!(f, "{}> {}", " ".repeat(event.depth()), event.data())
/// }
/// ```
pub struct EventView<'a, R: Renderer> {
    tasks: &'a TaskStore<R>,
    task: TaskId,
    id: EventIndex,
}

impl<'a, R: Renderer> EventView<'a, R> {
    pub(crate) fn new(tasks: &'a TaskStore<R>, task: TaskId, id: usize) -> Self {
        Self {
            tasks,
            task,
            id: EventIndex(id),
        }
    }

    /// Returns `true` if this event belongs to the virtual root task.
    pub fn is_root(&self) -> bool {
        self.task == TaskId::ROOT
    }

    /// Returns the user-defined data stored on this event.
    pub fn data(&self) -> &R::EventData {
        let task = self.get_task();
        task.events().get(self.id.0).unwrap()
    }

    /// Returns the nesting depth of the owning task.
    pub fn depth(&self) -> usize {
        self.get_task().depth
    }

    /// Returns the TaskView of the owning task.
    pub fn task<'b>(&'b self) -> TaskView<'b, R> {
        TaskView::new(self.tasks, self.task)
    }

    fn get_task(&self) -> &Task<R> {
        self.tasks.task(&self.task)
    }
}
