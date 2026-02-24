use std::num::NonZeroUsize;
use std::{collections::VecDeque, time::Instant};

use indexmap::{IndexMap, IndexSet};

use crate::{Action, Renderer};

/// Unique identifier for a task in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(usize);

impl TaskId {
    /// A reserved ID for the virtual root task.
    pub const ROOT: Self = Self(0);

    /// Check if this ID refers to the root.
    pub fn is_root(&self) -> bool {
        self.0 == 0
    }

    /// Creates a new [`TaskId`] from a non-zero value.
    pub fn new(id: NonZeroUsize) -> Self {
        Self(id.get())
    }
}

impl From<usize> for TaskId {
    fn from(id: usize) -> Self {
        match id {
            0 => panic!("TaskId cannot be 0"),
            _ => Self(id),
        }
    }
}

/// Index reference to an event within a task's event buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct EventIndex(pub(crate) usize);

pub struct TaskStore<R: Renderer> {
    pub(crate) tasks: IndexMap<TaskId, Task<R>>,
    pub(crate) max_events: usize,
}

impl<R: Renderer> Clone for TaskStore<R>
where
    Task<R>: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
            max_events: self.max_events,
        }
    }
}

impl<R: Renderer> std::fmt::Debug for TaskStore<R>
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

impl<R: Renderer> TaskStore<R> {
    pub(crate) fn new() -> Self {
        Self::with_max_events(64)
    }

    pub(crate) fn with_max_events(max_events: usize) -> Self {
        let mut tasks = IndexMap::new();
        tasks.insert(TaskId::ROOT, Task::new(0, None, None));
        Self { tasks, max_events }
    }

    pub(crate) fn root(&mut self) -> &mut Task<R> {
        self.tasks.get_mut(&TaskId::ROOT).unwrap()
    }

    pub(crate) fn task(&self, id: &TaskId) -> &Task<R> {
        self.tasks.get(id).unwrap()
    }

    pub(crate) fn remove(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.shift_remove(&id) {
            if let Some(parent) = self.get_task_mut(task.parent) {
                parent.subtasks.shift_remove(&id);
            }
            for subtask in task.subtasks {
                self.remove(subtask);
            }
        }
    }

    pub(crate) fn apply(&mut self, action: Action<R>) {
        match action {
            Action::Event { parent, data } => {
                let max_events = self.max_events;
                if let Some(task) = self.get_task_mut(parent) {
                    task.events.push_back(data);
                    while task.events.len() > max_events {
                        task.events.pop_front();
                    }
                }
            }
            Action::TaskStart {
                id,
                parent,
                data: event,
            } => {
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
                if let Some(task) = self.get_task_mut(Some(id)) {
                    task.completed = true;
                }
            }
            Action::CancelAll => {
                let mut queue = self
                    .root()
                    .subtasks
                    .iter()
                    .copied()
                    .collect::<VecDeque<_>>();
                while let Some(id) = queue.pop_front() {
                    let task = self.tasks.get_mut(&id).unwrap();
                    task.cancelled = true;
                    queue.extend(task.subtasks.iter());
                }
            }
        }
    }

    fn get_task_mut<I: Into<TaskId>>(&mut self, id: Option<I>) -> Option<&mut Task<R>> {
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
    pub(crate) cancelled: bool,
    pub(crate) started_at: Instant,
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
            cancelled: false,
            started_at: Instant::now(),
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
}
