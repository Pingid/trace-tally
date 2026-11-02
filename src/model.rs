//! The task tree: plain data, independent of how it is captured or drawn.
//!
//! [`Action`]s describe state changes, [`TaskTree`] applies them, and
//! [`TaskView`] / [`EventView`] give read-only access for rendering.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use indexmap::{IndexMap, IndexSet};

/// Unique identifier for a task in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    /// Creates a new [`TaskId`].
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw id value.
    pub const fn get(&self) -> u64 {
        self.0
    }
}

impl From<u64> for TaskId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

/// A state change in the task tree.
///
/// `T` is the data stored per task and `E` the data stored per event.
///
/// # Example
///
/// ```rust
/// use trace_tally::*;
///
/// let mut tree = TaskTree::<String, String>::new();
/// tree.apply(Action::TaskStart {
///     id: TaskId::new(1),
///     parent: None,
///     data: "my task".into(),
/// });
/// assert_eq!(tree.roots().next().unwrap().data(), "my task");
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action<T, E> {
    /// A new task has started.
    ///
    /// If `parent` is `None` or refers to an unknown ID, the task is
    /// attached at the top level.
    TaskStart {
        id: TaskId,
        parent: Option<TaskId>,
        data: T,
    },
    /// Replaces the data of an existing task.
    TaskUpdate { id: TaskId, data: T },
    /// A new event on an existing task (or the top level if `parent` is `None`).
    ///
    /// Events for an unknown task are dropped.
    Event { parent: Option<TaskId>, data: E },
    /// A task has completed.
    TaskEnd { id: TaskId },
    /// Mark all pending tasks as cancelled.
    ///
    /// Cancelled tasks still render (via [`TaskView::cancelled`]) and are
    /// treated like completed tasks by [`TaskView::active`].
    CancelAll,
}

/// Default cap on the number of events retained per task.
pub const DEFAULT_MAX_EVENTS: usize = 64;

/// The hierarchical task state built up from [`Action`]s.
///
/// ```rust
/// use trace_tally::*;
///
/// let mut tree = TaskTree::<&str, &str>::new();
/// tree.apply(Action::TaskStart { id: TaskId::new(1), parent: None, data: "build" });
/// tree.apply(Action::Event { parent: Some(TaskId::new(1)), data: "compiling" });
///
/// let task = tree.get(TaskId::new(1)).unwrap();
/// assert_eq!(task.events().next().unwrap().data(), &"compiling");
/// ```
#[derive(Debug, Clone)]
pub struct TaskTree<T, E> {
    tasks: IndexMap<TaskId, Node<T, E>>,
    roots: IndexSet<TaskId>,
    root_events: VecDeque<E>,
    max_events: usize,
}

#[derive(Debug, Clone)]
struct Node<T, E> {
    data: T,
    depth: usize,
    completed: bool,
    cancelled: bool,
    started_at: Instant,
    parent: Option<TaskId>,
    events: VecDeque<E>,
    subtasks: IndexSet<TaskId>,
}

impl<T, E> Default for TaskTree<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, E> TaskTree<T, E> {
    /// Creates an empty tree retaining [`DEFAULT_MAX_EVENTS`] events per task.
    pub fn new() -> Self {
        Self {
            tasks: IndexMap::new(),
            roots: IndexSet::new(),
            root_events: VecDeque::new(),
            max_events: DEFAULT_MAX_EVENTS,
        }
    }

    /// Cap the number of events retained per task. Oldest events are
    /// dropped when the limit is exceeded.
    pub fn with_max_events(mut self, n: usize) -> Self {
        self.max_events = n;
        self
    }

    /// Applies a single [`Action`] to the tree.
    pub fn apply(&mut self, action: Action<T, E>) {
        match action {
            Action::TaskStart { id, parent, data } => {
                let parent = parent.filter(|p| self.tasks.contains_key(p));
                let depth = parent.map_or(0, |p| self.tasks[&p].depth) + 1;
                self.tasks.insert(
                    id,
                    Node {
                        data,
                        depth,
                        completed: false,
                        cancelled: false,
                        started_at: Instant::now(),
                        parent,
                        events: VecDeque::new(),
                        subtasks: IndexSet::new(),
                    },
                );
                match parent {
                    Some(p) => self.tasks[&p].subtasks.insert(id),
                    None => self.roots.insert(id),
                };
            }
            Action::TaskUpdate { id, data } => {
                if let Some(task) = self.tasks.get_mut(&id) {
                    task.data = data;
                }
            }
            Action::Event { parent, data } => {
                let events = match parent {
                    Some(id) => match self.tasks.get_mut(&id) {
                        Some(task) => &mut task.events,
                        None => return,
                    },
                    None => &mut self.root_events,
                };
                events.push_back(data);
                while events.len() > self.max_events {
                    events.pop_front();
                }
            }
            Action::TaskEnd { id } => {
                if let Some(task) = self.tasks.get_mut(&id) {
                    task.completed = true;
                }
            }
            Action::CancelAll => {
                for task in self.tasks.values_mut() {
                    task.cancelled = true;
                }
            }
        }
    }

    /// Returns a view of the task with the given `id`.
    pub fn get(&self, id: TaskId) -> Option<TaskView<'_, T, E>> {
        self.tasks
            .contains_key(&id)
            .then_some(TaskView { tree: self, id })
    }

    /// Iterates over top-level tasks in insertion order.
    pub fn roots(&self) -> impl DoubleEndedIterator<Item = TaskView<'_, T, E>> + ExactSizeIterator {
        self.roots
            .iter()
            .map(move |&id| TaskView { tree: self, id })
    }

    /// Iterates over events not attached to any task.
    pub fn root_events(
        &self,
    ) -> impl DoubleEndedIterator<Item = EventView<'_, T, E>> + ExactSizeIterator {
        (0..self.root_events.len()).map(move |index| EventView {
            tree: self,
            owner: None,
            index,
        })
    }

    /// Discards all events not attached to any task.
    pub fn clear_root_events(&mut self) {
        self.root_events.clear();
    }

    /// Discards the buffered events of a task.
    pub fn clear_events(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.events.clear();
        }
    }

    /// Removes a task and all of its descendants.
    pub fn remove(&mut self, id: TaskId) {
        let Some(task) = self.tasks.shift_remove(&id) else {
            return;
        };
        match task.parent.and_then(|p| self.tasks.get_mut(&p)) {
            Some(parent) => parent.subtasks.shift_remove(&id),
            None => self.roots.shift_remove(&id),
        };
        for subtask in task.subtasks {
            self.remove(subtask);
        }
    }

    /// Number of tasks in the tree.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Returns `true` if there are no tasks in the tree.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    fn node(&self, id: TaskId) -> &Node<T, E> {
        // Views are only constructed for ids present in the tree, and the
        // tree cannot be mutated while a view borrows it.
        &self.tasks[&id]
    }

    fn siblings(&self, parent: Option<TaskId>) -> &IndexSet<TaskId> {
        match parent {
            Some(p) => &self.node(p).subtasks,
            None => &self.roots,
        }
    }
}

/// Read-only view of a task.
///
/// ```rust,ignore
/// fn render_task_line(&mut self, f: &mut Frame<'_>, task: &TaskView<'_, String, String>) -> io::Result<()> {
///     let prefix = if task.active() { ".." } else { "done" };
///     writeln!(f, "[{}] {} (depth={})", prefix, task.data(), task.depth())
/// }
/// ```
pub struct TaskView<'a, T, E> {
    tree: &'a TaskTree<T, E>,
    id: TaskId,
}

impl<T, E> Clone for TaskView<'_, T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, E> Copy for TaskView<'_, T, E> {}

impl<'a, T, E> TaskView<'a, T, E> {
    /// Returns the id of this task.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Returns the tree this task belongs to.
    pub fn tree(&self) -> &'a TaskTree<T, E> {
        self.tree
    }

    /// Returns the user-defined data stored on this task.
    pub fn data(&self) -> &'a T {
        &self.node().data
    }

    /// Returns the nesting depth of this task (top-level tasks are depth 1).
    pub fn depth(&self) -> usize {
        self.node().depth
    }

    /// When this task started.
    pub fn started_at(&self) -> Instant {
        self.node().started_at
    }

    /// How long since this task started.
    pub fn elapsed(&self) -> Duration {
        self.node().started_at.elapsed()
    }

    /// Returns `true` if the task is active (not completed or cancelled).
    pub fn active(&self) -> bool {
        !self.completed() && !self.cancelled()
    }

    /// Returns `true` if the task has ended.
    pub fn completed(&self) -> bool {
        self.node().completed
    }

    /// Returns `true` if the task was marked cancelled by [`Action::CancelAll`].
    pub fn cancelled(&self) -> bool {
        self.node().cancelled
    }

    /// Returns the parent task, or `None` for top-level tasks.
    pub fn parent(&self) -> Option<TaskView<'a, T, E>> {
        self.node().parent.map(|id| TaskView {
            tree: self.tree,
            id,
        })
    }

    /// Returns an iterator over the task's buffered events.
    pub fn events(
        &self,
    ) -> impl DoubleEndedIterator<Item = EventView<'a, T, E>> + ExactSizeIterator {
        let (tree, owner) = (self.tree, Some(self.id));
        (0..self.node().events.len()).map(move |index| EventView { tree, owner, index })
    }

    /// Returns an iterator over the task's direct children.
    pub fn subtasks(
        &self,
    ) -> impl DoubleEndedIterator<Item = TaskView<'a, T, E>> + ExactSizeIterator {
        let tree = self.tree;
        self.node()
            .subtasks
            .iter()
            .map(move |&id| TaskView { tree, id })
    }

    /// Returns the position of this task among its siblings.
    pub fn index(&self) -> usize {
        self.tree
            .siblings(self.node().parent)
            .get_index_of(&self.id)
            .unwrap_or(0)
    }

    /// Returns `true` if this task is the last of its siblings.
    pub fn is_last(&self) -> bool {
        self.index() + 1 == self.tree.siblings(self.node().parent).len()
    }

    fn node(&self) -> &'a Node<T, E> {
        self.tree.node(self.id)
    }
}

/// Read-only view of a buffered event.
///
/// ```rust,ignore
/// fn render_event_line(&mut self, f: &mut Frame<'_>, event: &EventView<'_, String, String>) -> io::Result<()> {
///     writeln!(f, "{}> {}", " ".repeat(event.depth()), event.data())
/// }
/// ```
pub struct EventView<'a, T, E> {
    tree: &'a TaskTree<T, E>,
    owner: Option<TaskId>,
    index: usize,
}

impl<T, E> Clone for EventView<'_, T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, E> Copy for EventView<'_, T, E> {}

impl<'a, T, E> EventView<'a, T, E> {
    /// Returns `true` if this event is not attached to any task.
    pub fn is_root(&self) -> bool {
        self.owner.is_none()
    }

    /// Returns the user-defined data stored on this event.
    pub fn data(&self) -> &'a E {
        match self.owner {
            Some(id) => &self.tree.node(id).events[self.index],
            None => &self.tree.root_events[self.index],
        }
    }

    /// Returns the nesting depth of the owning task (0 for root events).
    pub fn depth(&self) -> usize {
        self.task().map_or(0, |t| t.depth())
    }

    /// Returns the owning task, or `None` for root events.
    pub fn task(&self) -> Option<TaskView<'a, T, E>> {
        self.owner.map(|id| TaskView {
            tree: self.tree,
            id,
        })
    }
}
