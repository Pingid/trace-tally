use crate::{Renderer, TaskView};

/// Renders tree-drawing characters for a task's position in the hierarchy.
///
/// ```text
/// root task
/// ├── child 1
/// │   ├── grandchild a
/// │   └── grandchild b
/// └── child 2
/// ```
///
/// ```rust,ignore
/// fn render_task_line(&mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>) -> io::Result<()> {
///     write!(f, "{}", TreeIndent::of(task))?;
///     writeln!(f, "{}", task.data())
/// }
/// ```
pub struct TreeIndent(String);

impl TreeIndent {
    pub fn of<R: Renderer>(task: &TaskView<'_, R>) -> Self {
        if task.depth() == 0 {
            return Self(String::new());
        }

        // Walk up the tree by TaskId to avoid borrow conflicts.
        // At each level, record whether the node is the last sibling.
        let mut segments: Vec<bool> = Vec::with_capacity(task.depth());
        let mut current_id = task.id();

        loop {
            let current = task.view(current_id);
            match current.parent() {
                Some(parent) => {
                    let is_last = current.index() == parent.subtasks().len() - 1;
                    segments.push(is_last);
                    if parent.depth() == 0 {
                        break;
                    }
                    current_id = parent.id();
                }
                None => break,
            }
        }

        segments.reverse();

        let mut out = String::new();
        for (i, &is_last) in segments.iter().enumerate() {
            let is_self = i == segments.len() - 1;
            match (is_self, is_last) {
                (true, true) => out.push_str("└── "),
                (true, false) => out.push_str("├── "),
                (false, true) => out.push_str("    "),
                (false, false) => out.push_str("│   "),
            }
        }

        Self(out)
    }
}

impl std::fmt::Display for TreeIndent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
