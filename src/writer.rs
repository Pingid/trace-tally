use std::collections::VecDeque;
use std::io::Write;

use crate::task::{EventIndex, Task, TaskId, TaskStore};
use crate::{Action, Renderer};

/// Write target with ANSI cursor control for frame clearing.
pub struct FrameWriter<'a> {
    target: &'a mut dyn Write,
    frame_lines: usize,
}

impl<'a> FrameWriter<'a> {
    fn new(target: &'a mut dyn Write, frame_lines: usize) -> Self {
        Self {
            target,
            frame_lines,
        }
    }

    fn clear_frame(&mut self) -> Result<(), std::io::Error> {
        let lines_drawn = self.frame_lines;
        if lines_drawn > 0 {
            write!(self, "\r\x1b[{}A\x1b[2K\x1b[J", lines_drawn).unwrap();
            self.target.flush()?;
        }
        self.frame_lines = 0;
        Ok(())
    }

    fn frame_lines(&self) -> usize {
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
#[derive(Clone, Copy)]
pub struct TaskView<'a, R: Renderer> {
    id: TaskId,
    tasks: &'a TaskStore<R>,
}

fn task_view<'a, R: Renderer>(tasks: &'a TaskStore<R>, id: TaskId) -> TaskView<'a, R> {
    TaskView::new(tasks, id)
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

    /// Returns `true` if the task's span has closed.
    pub fn completed(&self) -> bool {
        self.tasks.task(&self.id).completed
    }

    /// Returns `true` if the task was marked cancelled by [`Action::CancelAll`].
    pub fn cancelled(&self) -> bool {
        self.tasks.task(&self.id).cancelled
    }

    pub fn parent<'b>(&'b self) -> Option<TaskView<'b, R>> {
        self.tasks
            .task(&self.id)
            .parent
            .map(|id| task_view(self.tasks, id))
    }

    /// Returns an iterator over the task's buffered events.
    pub fn events<'b>(&'b self) -> impl Iterator<Item = EventView<'b, R>> {
        (0..self.tasks.task(&self.id).events.len())
            .map(move |id| event_view(self.tasks, self.id, id))
    }

    /// Returns an iterator over the task's direct children.
    pub fn subtasks<'b>(&'b self) -> impl Iterator<Item = TaskView<'b, R>> {
        self.tasks
            .task(&self.id)
            .subtasks
            .iter()
            .map(move |id| task_view(self.tasks, *id))
    }

    // Returns the index of this task in the parent's subtasks.
    pub fn index(&self) -> usize {
        let parent = self.tasks.task(&self.id).parent.unwrap();
        let parent = self.tasks.task(&parent);
        parent.subtasks.get_index_of(&self.id).unwrap()
    }
}

/// Read-only view of a buffered event, passed to [`Renderer::render_event_line`].
pub struct EventView<'a, R: Renderer> {
    tasks: &'a TaskStore<R>,
    task: TaskId,
    id: EventIndex,
}

fn event_view<'a, R: Renderer>(
    tasks: &'a TaskStore<R>,
    task: TaskId,
    id: usize,
) -> EventView<'a, R> {
    EventView::new(tasks, task, id)
}

impl<'a, R: Renderer> EventView<'a, R> {
    fn new(tasks: &'a TaskStore<R>, task: TaskId, id: usize) -> Self {
        Self {
            tasks,
            task,
            id: id.into(),
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

    /// Returns the nesting depth of the parent task.
    pub fn depth(&self) -> usize {
        self.get_task().depth
    }

    /// Returns the TaskView of the parent task.
    pub fn task<'b>(&'b self) -> TaskView<'b, R> {
        task_view(self.tasks, self.task)
    }

    fn get_task(&self) -> &Task<R> {
        self.tasks.task(&self.task)
    }
}

/// Receives [`Action`]s, manages the task tree, and drives rendering.
pub struct TaskRenderer<R: Renderer> {
    tasks: TaskStore<R>,
    frame_lines: usize,
    r: R,
}

impl<R: Renderer> Default for TaskRenderer<R>
where
    R: Default,
{
    fn default() -> Self {
        Self::new(R::default())
    }
}

impl<R: Renderer> std::fmt::Debug for TaskRenderer<R>
where
    R: std::fmt::Debug,
    R::TaskData: std::fmt::Debug,
    R::EventData: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "TaskRenderer {{")?;
        writeln!(f, "  tasks: {:?}", self.tasks)?;
        writeln!(f, "  frame_lines: {}", self.frame_lines)?;
        writeln!(f, "  r: {:?}", self.r)?;
        writeln!(f, "}}")
    }
}

impl<R: Renderer> Clone for TaskRenderer<R>
where
    R: Clone,
    TaskStore<R>: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
            frame_lines: self.frame_lines,
            r: self.r.clone(),
        }
    }
}

impl<R: Renderer> TaskRenderer<R> {
    /// Creates a new [`TaskRenderer`] with the given [`Renderer`] implementation.
    pub fn new(renderer: R) -> Self {
        Self {
            tasks: TaskStore::new(),
            frame_lines: 0,
            r: renderer,
        }
    }

    /// Applies a single [`Action`] to the task tree.
    pub fn update(&mut self, action: Action<R>) {
        self.tasks.apply(action);
    }

    /// Renders the current task tree to `target`.
    ///
    /// Completed and cancelled root tasks are rendered first (and removed),
    /// then active root tasks are rendered in the erasable frame region.
    pub fn render(&mut self, target: &mut dyn Write) -> Result<(), std::io::Error> {
        self.r.on_render_start();

        // Move the cursor to top of the active tasks frame
        let mut t = FrameWriter::new(target, self.frame_lines);
        t.clear_frame()?;

        // Render root task
        let mut queue = self.flush_root(&mut t)?;

        // Start active task frame
        let mut t = FrameWriter::new(target, 0);
        while let Some(task) = queue.pop_front() {
            if self.tasks.task(&task).data.is_some() {
                let view = task_view(&self.tasks, task);
                self.r.render_task(&mut t, &view)?;
            }
        }

        // Store the number of lines drawn in the active task frame
        self.frame_lines = t.frame_lines();

        t.flush()?;

        self.r.on_render_end();

        Ok(())
    }

    fn flush_root(
        &mut self,
        target: &mut FrameWriter<'_>,
    ) -> Result<VecDeque<TaskId>, std::io::Error> {
        let mut active = VecDeque::new();
        let mut completed = VecDeque::new();

        for i in 0..self.tasks.task(&TaskId::ROOT).events().len() {
            let view = event_view(&self.tasks, TaskId::ROOT, i);
            self.r.render_event_line(target, &view)?;
        }
        self.tasks.root().clear_events();

        for i in 0..self.tasks.task(&TaskId::ROOT).subtasks().len() {
            let task = *self
                .tasks
                .task(&TaskId::ROOT)
                .subtasks()
                .get_index(i)
                .unwrap();
            if self.tasks.task(&task).completed || self.tasks.task(&task).cancelled {
                let view = task_view(&self.tasks, task);
                self.r.render_task(target, &view)?;
                completed.push_back(task);
            } else {
                active.push_back(task);
            }
        }

        for id in completed {
            self.tasks.remove(id);
        }

        Ok(active)
    }
}
