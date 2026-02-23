use std::collections::VecDeque;
use std::io::Write;

use crate::task::{Task, TaskId, TaskRegistry};
use crate::{Action, EventRef, Renderer};

pub struct Target<'a> {
    target: &'a mut dyn Write,
    frame_lines: usize,
}

impl<'a> Target<'a> {
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

impl<'a> Write for Target<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let newlines = buf.iter().filter(|&&b| b == b'\n').count();
        self.frame_lines += newlines;
        self.target.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.target.flush()
    }
}

pub struct TaskView<'a, R: Renderer> {
    id: TaskId,
    tasks: &'a mut TaskRegistry<R>,
}

fn task_view<'a, R: Renderer>(tasks: &'a mut TaskRegistry<R>, id: TaskId) -> TaskView<'a, R> {
    TaskView::new(tasks, id)
}

impl<'a, R: Renderer> TaskView<'a, R> {
    pub fn new(tasks: &'a mut TaskRegistry<R>, id: TaskId) -> Self {
        Self { id, tasks }
    }
}

impl<'a, R: Renderer> TaskView<'a, R> {
    pub fn data(&self) -> &R::TaskData {
        self.tasks.task(&self.id).data.as_ref().unwrap()
    }

    pub fn depth(&self) -> usize {
        self.tasks.task(&self.id).depth
    }
}

pub struct EventView<'a, R: Renderer> {
    tasks: &'a mut TaskRegistry<R>,
    task: TaskId,
    id: EventRef,
}

fn event_view<'a, R: Renderer>(
    tasks: &'a mut TaskRegistry<R>, task: TaskId, id: usize,
) -> EventView<'a, R> {
    EventView {
        tasks,
        task,
        id: id.into(),
    }
}

impl<'a, R: Renderer> EventView<'a, R> {
    pub fn is_root(&self) -> bool {
        self.task == TaskId::ROOT
    }

    pub fn data(&self) -> &R::EventData {
        let task = self.task();
        task.events().get(self.id.0).unwrap()
    }

    pub fn depth(&self) -> usize {
        self.task().depth
    }

    fn task(&self) -> &Task<R> {
        self.tasks.task(&self.task)
    }
}

pub struct TaskRenderer<R: Renderer> {
    tasks: TaskRegistry<R>,
    frame_lines: usize,
    r: R,
}

impl<R: Renderer> Default for TaskRenderer<R>
where R: Default
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
    TaskRegistry<R>: Clone,
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
    pub fn new(renderer: R) -> Self {
        Self {
            tasks: TaskRegistry::new(),
            frame_lines: 0,
            r: renderer,
        }
    }

    pub fn update(&mut self, action: Action<R>) {
        self.tasks.apply_action(action);
    }

    pub fn render(&mut self, target: &mut dyn Write) -> Result<(), std::io::Error> {
        self.r.on_render_start();

        // Move the cursor to top of the active tasks frame
        let mut t = Target::new(target, self.frame_lines);
        t.clear_frame()?;

        // Render root task
        let (mut queue, completed) = self.render_task(&mut t, &TaskId::ROOT)?;
        self.tasks.root().clear_events();
        for id in completed {
            self.tasks.remove(id);
        }

        // Start active task frame
        let mut t = Target::new(target, 0);
        while let Some(task) = queue.pop_front() {
            // Render task events
            let (active, _) = self.render_task(&mut t, &task)?;
            queue.extend(active);
        }

        // Store the number of lines drawn in the active task frame
        self.frame_lines = t.frame_lines();

        t.flush()?;

        Ok(())
    }

    fn render_task(
        &mut self, target: &mut Target<'_>, task: &TaskId,
    ) -> Result<(VecDeque<TaskId>, VecDeque<TaskId>), std::io::Error> {
        let mut active = VecDeque::new();
        let mut completed = VecDeque::new();

        if self.tasks.task(task).data.is_some() {
            let view = task_view(&mut self.tasks, *task);
            self.r.task_start(target, view)?;
        }

        for i in 0..self.tasks.task(task).subtasks().len() {
            let task_id = *self.tasks.task(task).subtasks().get_index(i).unwrap();
            if self.tasks.task(&task_id).completed {
                let view = task_view(&mut self.tasks, task_id);
                self.r.task_end(target, view)?;
                completed.push_back(task_id);
                continue;
            } else {
                active.push_back(task_id);
            }
        }

        for event in 0..self.tasks.task(task).events().len() {
            let view = event_view(&mut self.tasks, *task, event);
            self.r.event(target, view)?;
        }

        Ok((active, completed))
    }
}
