use indexmap::{IndexMap, IndexSet};
use std::collections::VecDeque;

use crate::{Action, Renderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId {
    /// The raw ID from the source (e.g., tracing::Id)
    inner: u64,
    /// A unique session or generation counter to prevent collisions
    /// if IDs are recycled by the subscriber.
    generation: u64,
}

impl TaskId {
    /// A reserved ID for the virtual root task.
    pub const ROOT: Self = Self {
        inner: 0,
        generation: 0,
    };

    /// Check if this ID refers to the root.
    pub fn is_root(&self) -> bool {
        self.inner == 0 && self.generation == 0
    }

    pub fn new(id: u64, generation: u64) -> Self {
        Self {
            inner: id,
            generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventRef(pub(crate) usize);

impl From<usize> for EventRef {
    fn from(id: usize) -> Self {
        Self(id)
    }
}

pub struct TaskRegistry<R: Renderer> {
    pub(crate) tasks: IndexMap<TaskId, Task<R>>,
}

impl<R: Renderer> Clone for TaskRegistry<R>
where
    Task<R>: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
        }
    }
}

impl<R: Renderer> std::fmt::Debug for TaskRegistry<R>
where
    Task<R>: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "TaskRegistry {{")?;
        for (id, task) in &self.tasks {
            writeln!(f, "  {:?}: {:?}", id, task)?;
        }
        writeln!(f, "}}")
    }
}

impl<R: Renderer> TaskRegistry<R> {
    pub(crate) fn new() -> Self {
        let mut tasks = IndexMap::new();
        let task = Task::new(0, None, None);
        tasks.insert(TaskId::ROOT, task);
        Self { tasks }
    }

    pub(crate) fn root(&mut self) -> &mut Task<R> {
        self.tasks.get_mut(&TaskId::ROOT).unwrap()
    }

    pub(crate) fn task(&self, id: &TaskId) -> &Task<R> {
        self.tasks.get(id).unwrap()
    }

    pub(crate) fn remove(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.shift_remove(&id)
            && let Some(parent) = self.resolve_task(task.parent)
        {
            parent.subtasks.shift_remove(&id);
        }
    }

    pub(crate) fn apply_action(&mut self, action: Action<R>) {
        match action {
            Action::Event { parent, event } => {
                if parent == Some(TaskId::ROOT) {
                    self.root().events.push_back(event);
                    return;
                }
                if let Some(task) = self.resolve_task(parent) {
                    task.events.push_back(event);
                    if task.events.len() > 3 {
                        task.events.pop_front();
                    }
                }
            }
            Action::TaskStart { id, parent, event } => {
                let parent_id = parent
                    .and_then(|id| match self.tasks.contains_key(&id) {
                        true => Some(id),
                        false => None,
                    })
                    .unwrap_or(TaskId::ROOT);

                let depth = self.task(&parent_id).depth + 1;
                let task = Task::new(depth, Some(parent_id), Some(event));

                self.tasks.insert(id, task);

                let p_task = self.tasks.get_mut(&parent_id).unwrap();

                p_task.subtasks.insert(id);
            }
            Action::TaskEnd { id } => {
                if let Some(task) = self.resolve_task(Some(id)) {
                    task.complete_task();
                }
            }
            Action::Exit => {}
        }
    }

    fn resolve_task<I: Into<TaskId>>(&mut self, id: Option<I>) -> Option<&mut Task<R>> {
        match id {
            Some(id) => self.tasks.get_mut(&id.into()),
            None => self.tasks.get_mut(&TaskId::ROOT),
        }
    }
}

#[derive(Clone)]
pub struct Task<R: Renderer> {
    pub(crate) depth: usize,
    pub(crate) completed: bool,
    pub(crate) parent: Option<TaskId>,
    pub(crate) data: Option<R::TaskData>,
    pub(crate) events: VecDeque<R::EventData>,
    pub(crate) subtasks: IndexSet<TaskId>,
}

impl<R: Renderer> std::fmt::Debug for Task<R>
where
    R::TaskData: std::fmt::Debug,
    R::EventData: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Task {{")?;
        writeln!(f, "  depth: {}", self.depth)?;
        writeln!(f, "  completed: {}", self.completed)?;
        writeln!(f, "  parent: {:?}", self.parent)?;
        writeln!(f, "  data: {:?}", self.data)?;
        writeln!(f, "  events: {:?}", self.events)?;
        writeln!(f, "  subtasks: {:?}", self.subtasks)?;
        writeln!(f, "}}")
    }
}

impl<R: Renderer> Task<R> {
    fn new(depth: usize, parent: Option<TaskId>, data: Option<R::TaskData>) -> Self {
        Self {
            data,
            depth,
            parent,
            completed: false,
            events: VecDeque::new(),
            subtasks: IndexSet::new(),
        }
    }

    pub(crate) fn events(&self) -> &VecDeque<R::EventData> {
        &self.events
    }

    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }

    pub(crate) fn subtasks(&self) -> &IndexSet<TaskId> {
        &self.subtasks
    }

    pub(crate) fn complete_task(&mut self) {
        self.completed = true;
    }
}
