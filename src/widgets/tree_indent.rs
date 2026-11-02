use crate::TaskView;

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
/// fn render_task_line(&mut self, f: &mut Frame<'_>, task: &TaskView<'_, String, String>) -> io::Result<()> {
///     write!(f, "{}", TreeIndent::of(task))?;
///     writeln!(f, "{}", task.data())
/// }
/// ```
pub struct TreeIndent(String);

impl TreeIndent {
    pub fn of<T, E>(task: &TaskView<'_, T, E>) -> Self {
        // At each level, record whether the node is the last sibling.
        let mut segments: Vec<bool> = Vec::with_capacity(task.depth());
        let mut current = Some(*task);
        while let Some(t) = current {
            segments.push(t.is_last());
            current = t.parent();
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
