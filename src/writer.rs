use std::collections::VecDeque;
use std::io::Write;

use crate::task::TaskStore;
use crate::{Action, EventView, FrameWriter, Renderer, TaskId, TaskView};

/// Receives [`Action`]s, manages the task tree, and drives rendering.
///
/// For channel-based setups, create a `TaskRenderer` on the render thread
/// and feed it actions from the receiver.
///
/// ```rust,ignore
/// let mut renderer = TaskRenderer::new(MyRenderer::default());
/// while let Ok(action) = rx.try_recv() {
///     renderer.update(action);
/// }
/// renderer.render(&mut std::io::stderr()).unwrap();
/// ```
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

    /// Cap the number of events retained per task. Oldest events are
    /// dropped when the limit is exceeded. Default is 64.
    pub fn max_events_per_task(mut self, n: usize) -> Self {
        self.tasks = TaskStore::with_max_events(n);
        self
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
                let view = TaskView::new(&self.tasks, task);
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
            let view = EventView::new(&self.tasks, TaskId::ROOT, i);
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
                let view = TaskView::new(&self.tasks, task);
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
