//! Drawing the task tree: the [`Renderer`] trait, inline terminal output,
//! and render loops.

use std::io::Write;

use crate::model::{Action, EventView, TaskTree, TaskView};

mod frame;
mod runner;

pub use frame::{Frame, LiveRegion};
pub use runner::{ActionSource, RenderLoop};

/// Defines how tasks and events are drawn.
///
/// Each frame, [`TaskRenderer`] calls [`render_task`](Renderer::render_task)
/// for every top-level task. The default traversal renders the task line,
/// the last 3 buffered events (skipped for inactive tasks), then recurses
/// into subtasks. Override it to change the traversal.
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
///         &mut self, f: &mut Frame<'_>, task: &TaskView<'_, String, String>,
///     ) -> std::io::Result<()> {
///         writeln!(f, "{}{}", " ".repeat(task.depth()), task.data())
///     }
///
///     fn render_event_line(
///         &mut self, f: &mut Frame<'_>, event: &EventView<'_, String, String>,
///     ) -> std::io::Result<()> {
///         writeln!(f, "{}  {}", " ".repeat(event.depth()), event.data())
///     }
/// }
/// ```
pub trait Renderer {
    /// Data stored per task (e.g. a task name or metadata).
    type TaskData;
    /// Data stored per event (e.g. a log message).
    type EventData;

    /// Called once at the start of each render frame, before any tasks are visited.
    fn on_render_start(&mut self) {}

    /// Called once at the end of each render frame, after all tasks have been visited.
    fn on_render_end(&mut self) {}

    /// Renders a task and its descendants.
    fn render_task(
        &mut self,
        f: &mut Frame<'_>,
        task: &TaskView<'_, Self::TaskData, Self::EventData>,
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

    /// Renders the task header.
    #[allow(unused_variables)]
    fn render_task_line(
        &mut self,
        f: &mut Frame<'_>,
        task: &TaskView<'_, Self::TaskData, Self::EventData>,
    ) -> std::io::Result<()> {
        Ok(())
    }

    /// Renders a single buffered event.
    #[allow(unused_variables)]
    fn render_event_line(
        &mut self,
        f: &mut Frame<'_>,
        event: &EventView<'_, Self::TaskData, Self::EventData>,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

/// The [`TaskTree`] type a [`Renderer`] draws.
pub type TreeOf<R> = TaskTree<<R as Renderer>::TaskData, <R as Renderer>::EventData>;

/// The [`Action`] type a [`Renderer`] consumes.
pub type ActionOf<R> = Action<<R as Renderer>::TaskData, <R as Renderer>::EventData>;

/// Combines a [`TaskTree`], a [`Renderer`] and a [`LiveRegion`] into an
/// inline terminal display.
///
/// Each [`render`](Self::render) prints root events and finished top-level
/// tasks permanently (removing them from the tree), then redraws active
/// tasks in the live region.
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
///     id: TaskId::new(1),
///     parent: None,
///     data: "my task".into(),
/// });
/// renderer.render(&mut std::io::stderr()).unwrap();
/// ```
pub struct TaskRenderer<R: Renderer> {
    tree: TreeOf<R>,
    region: LiveRegion,
    renderer: R,
}

impl<R: Renderer + Default> Default for TaskRenderer<R> {
    fn default() -> Self {
        Self::new(R::default())
    }
}

impl<R> std::fmt::Debug for TaskRenderer<R>
where
    R: Renderer + std::fmt::Debug,
    R::TaskData: std::fmt::Debug,
    R::EventData: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskRenderer")
            .field("tree", &self.tree)
            .field("region", &self.region)
            .field("renderer", &self.renderer)
            .finish()
    }
}

impl<R> Clone for TaskRenderer<R>
where
    R: Renderer + Clone,
    R::TaskData: Clone,
    R::EventData: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tree: self.tree.clone(),
            region: self.region.clone(),
            renderer: self.renderer.clone(),
        }
    }
}

impl<R: Renderer> TaskRenderer<R> {
    /// Creates a new [`TaskRenderer`] with the given [`Renderer`].
    pub fn new(renderer: R) -> Self {
        Self {
            tree: TaskTree::new(),
            region: LiveRegion::new(),
            renderer,
        }
    }

    /// Cap the number of events retained per task. Default is
    /// [`DEFAULT_MAX_EVENTS`](crate::model::DEFAULT_MAX_EVENTS).
    pub fn max_events_per_task(mut self, n: usize) -> Self {
        self.tree = std::mem::take(&mut self.tree).with_max_events(n);
        self
    }

    /// Terminal width used to count soft-wrapped rows. See [`LiveRegion::set_width`].
    pub fn width(mut self, width: Option<usize>) -> Self {
        self.region.set_width(width);
        self
    }

    /// Applies a single [`Action`] to the task tree.
    pub fn update(&mut self, action: ActionOf<R>) {
        self.tree.apply(action);
    }

    /// The current task tree.
    pub fn tree(&self) -> &TreeOf<R> {
        &self.tree
    }

    /// Mutable access to the task tree.
    pub fn tree_mut(&mut self) -> &mut TreeOf<R> {
        &mut self.tree
    }

    /// The inner [`Renderer`].
    pub fn renderer(&self) -> &R {
        &self.renderer
    }

    /// Mutable access to the inner [`Renderer`].
    pub fn renderer_mut(&mut self) -> &mut R {
        &mut self.renderer
    }

    /// Renders the current task tree to `target`.
    pub fn render(&mut self, target: &mut dyn Write) -> std::io::Result<()> {
        self.renderer.on_render_start();
        self.region.clear(target)?;

        let mut f = self.region.committed(target);
        for event in self.tree.root_events() {
            self.renderer.render_event_line(&mut f, &event)?;
        }
        let mut finished = Vec::new();
        for task in self.tree.roots().filter(|t| !t.active()) {
            self.renderer.render_task(&mut f, &task)?;
            finished.push(task.id());
        }
        self.tree.clear_root_events();
        for id in finished {
            self.tree.remove(id);
        }

        let mut f = self.region.live(target);
        for task in self.tree.roots() {
            self.renderer.render_task(&mut f, &task)?;
        }
        f.flush()?;

        self.renderer.on_render_end();
        Ok(())
    }
}
