use std::io::Write;

use crate::{Action, EventView, Frame, Renderer, TaskId, TaskRenderer, TaskTree, TaskView};

pub struct VirtualTerm {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    buf: Vec<u8>,
}

impl VirtualTerm {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            buf: Vec::new(),
        }
    }

    pub fn render(&self) -> String {
        self.lines.join("\n")
    }

    fn ensure_row(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(String::new());
        }
    }

    fn process(&mut self, s: &str) {
        if s.contains("\x1b[") {
            if let Some(pos) = s.find('A') {
                let num_str = &s[s.find('[').unwrap() + 1..pos];
                if let Ok(n) = num_str.parse::<usize>() {
                    self.cursor_row = self.cursor_row.saturating_sub(n);
                }
            }
            if s.contains("\x1b[2K") {
                self.ensure_row(self.cursor_row);
                self.lines[self.cursor_row].clear();
            }
            if s.contains("\x1b[J") {
                self.lines.truncate(self.cursor_row + 1);
            }
        } else {
            for c in s.chars() {
                match c {
                    '\n' => {
                        self.cursor_row += 1;
                        self.ensure_row(self.cursor_row);
                    }
                    _ => {
                        self.ensure_row(self.cursor_row);
                        self.lines[self.cursor_row].push(c);
                    }
                }
            }
        }
    }
}

impl std::io::Write for VirtualTerm {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let s = String::from_utf8(std::mem::take(&mut self.buf)).unwrap();
            self.process(&s);
        }
        Ok(())
    }
}

#[derive(Default)]
struct TestRenderer;

impl Renderer for TestRenderer {
    type EventData = String;
    type TaskData = String;

    fn render_task_line(
        &mut self,
        target: &mut Frame<'_>,
        task: &TaskView<'_, String, String>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        writeln!(target, "{}{}", indent, task.data())
    }

    fn render_event_line(
        &mut self,
        target: &mut Frame<'_>,
        task: &EventView<'_, String, String>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        writeln!(target, "{}{}", indent, task.data())
    }
}

struct TestEnv {
    term: VirtualTerm,
    writer: TaskRenderer<TestRenderer>,
    task: Option<TaskId>,
    counter: u64,
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            term: VirtualTerm::new(),
            writer: TaskRenderer::new(TestRenderer),
            task: None,
            counter: 1,
        }
    }

    fn event(&mut self, message: &str) {
        self.writer.update(Action::Event {
            parent: self.task,
            data: message.to_string(),
        });
    }

    fn span(&mut self, name: &str, f: impl FnOnce(&mut Self)) -> TaskId {
        let prev = self.task;
        let id = TaskId::new(self.counter);
        self.counter += 1;
        self.writer.update(Action::TaskStart {
            id,
            parent: self.task,
            data: name.to_string(),
        });
        self.task = Some(id);
        f(self);
        self.writer.update(Action::TaskEnd { id });
        self.task = prev;
        id
    }

    fn render(&mut self) -> String {
        self.writer.render(&mut self.term).unwrap();
        self.term.render()
    }
}

#[test]
fn test_virtual_term() {
    let mut env = TestEnv::new();
    env.event("test 1");
    assert_eq!(env.render(), "test 1\n");
}

#[test]
fn test_span_with_events() {
    let mut env = TestEnv::new();
    env.span("my-span", |env| {
        env.event("inside");
        assert_eq!(env.render(), " my-span\n inside\n");
    });
}

#[test]
fn test_nested_spans() {
    let mut env = TestEnv::new();
    env.span("outer", |env| {
        env.span("inner", |env| {
            env.event("deep");
            assert_eq!(env.render(), " outer\n  inner\n  deep\n");
        });
    });
}

#[test]
fn test_completed_span_removal() {
    let mut env = TestEnv::new();
    env.span("done", |_| {});
    env.render();
    env.event("after");
    assert_eq!(env.render(), " done\nafter\n");
}

#[test]
fn test_multiple_render_cycles() {
    let mut env = TestEnv::new();
    env.event("root");
    env.span("s", |env| {
        env.event("a");
        assert_eq!(env.render(), "root\n s\n a\n");
        env.event("b");
        assert_eq!(env.render(), "root\n s\n a\n b\n");
    });
}

#[test]
fn test_event_overflow() {
    let mut env = TestEnv::new();
    env.span("s", |env| {
        env.event("e1");
        env.event("e2");
        env.event("e3");
        env.event("e4");
        assert_eq!(env.render(), " s\n e2\n e3\n e4\n");
    });
}

#[test]
fn test_cancelled_tasks_are_committed() {
    let mut env = TestEnv::new();
    env.writer.update(Action::TaskStart {
        id: TaskId::new(1),
        parent: None,
        data: "pending".into(),
    });
    assert_eq!(env.render(), " pending\n");
    env.writer.update(Action::CancelAll);
    env.render();
    assert!(env.writer.tree().is_empty());
}

#[test]
fn test_task_update_replaces_data() {
    let mut tree = TaskTree::<&str, ()>::new();
    tree.apply(Action::TaskStart {
        id: TaskId::new(1),
        parent: None,
        data: "before",
    });
    tree.apply(Action::TaskUpdate {
        id: TaskId::new(1),
        data: "after",
    });
    assert_eq!(*tree.get(TaskId::new(1)).unwrap().data(), "after");
}

#[test]
fn test_views_on_top_level_tasks() {
    let mut tree = TaskTree::<&str, &str>::new();
    for (id, parent) in [(1, None), (2, None), (3, Some(1))] {
        tree.apply(Action::TaskStart {
            id: TaskId::new(id),
            parent: parent.map(TaskId::new),
            data: "t",
        });
    }
    tree.apply(Action::Event {
        parent: None,
        data: "root",
    });

    let first = tree.get(TaskId::new(1)).unwrap();
    let second = tree.get(TaskId::new(2)).unwrap();
    let child = tree.get(TaskId::new(3)).unwrap();
    assert!(first.parent().is_none());
    assert_eq!((first.index(), first.is_last()), (0, false));
    assert_eq!((second.index(), second.is_last()), (1, true));
    assert_eq!(child.parent().unwrap().id(), TaskId::new(1));
    assert!(child.is_last());

    let event = tree.root_events().next().unwrap();
    assert!(event.is_root() && event.task().is_none());
    assert_eq!(event.depth(), 0);
    assert!(tree.get(TaskId::new(99)).is_none());
}

#[test]
fn test_remove_detaches_subtree() {
    let mut tree = TaskTree::<(), ()>::new();
    for (id, parent) in [(1, None), (2, Some(1)), (3, Some(2))] {
        tree.apply(Action::TaskStart {
            id: TaskId::new(id),
            parent: parent.map(TaskId::new),
            data: (),
        });
    }
    tree.remove(TaskId::new(2));
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.get(TaskId::new(1)).unwrap().subtasks().len(), 0);
}

#[test]
fn test_actions_clone_without_renderer() {
    fn assert_clone<T: Clone>() {}
    assert_clone::<Action<String, String>>();
}

#[test]
fn test_live_region_counts_wrapped_rows() {
    let mut out = Vec::new();
    let mut region = crate::LiveRegion::new();
    region.set_width(Some(4));
    {
        let mut f = region.live(&mut out);
        writeln!(f, "abcd").unwrap(); // exactly fills one row
        writeln!(f, "abcdefghi").unwrap(); // wraps onto three rows
        writeln!(f, "\x1b[32mab\x1b[0m").unwrap(); // escapes take no columns
        write!(f, "é").unwrap(); // multi-byte char, no trailing newline
    }
    assert_eq!(region.rows(), 5);
    region.clear(&mut out).unwrap();
    assert_eq!(region.rows(), 0);
}

#[cfg(feature = "tracing")]
mod layer {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::SubscriberExt;

    use crate::{Action, TaskId, TaskLayer, TraceMapper};

    struct NameMapper;

    struct Fields<'a>(&'a mut String);
    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            *self.0 = format!("{}={:?}", field.name(), value);
        }
    }

    impl TraceMapper for NameMapper {
        type TaskData = String;
        type EventData = String;

        fn map_span(&self, attrs: &tracing::span::Attributes<'_>) -> String {
            attrs.metadata().name().to_string()
        }
        fn map_event(&self, event: &tracing::Event<'_>) -> String {
            let mut s = String::new();
            event.record(&mut Fields(&mut s));
            s
        }
        fn map_record(
            &self,
            previous: &String,
            values: &tracing::span::Record<'_>,
        ) -> Option<String> {
            let mut s = String::new();
            values.record(&mut Fields(&mut s));
            Some(format!("{previous} {s}"))
        }
    }

    #[test]
    fn test_custom_sink_receives_actions() {
        let actions = Arc::new(Mutex::new(Vec::new()));
        let sink = actions.clone();
        let layer = TaskLayer::new(NameMapper, move |a| sink.lock().unwrap().push(a));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("work", n = tracing::field::Empty);
            span.in_scope(|| tracing::info!("hi"));
            span.record("n", 5);
        });

        let id = TaskId::new(1);
        assert_eq!(
            *actions.lock().unwrap(),
            vec![
                Action::TaskStart {
                    id,
                    parent: None,
                    data: "work".into()
                },
                Action::Event {
                    parent: Some(id),
                    data: "message=hi".into()
                },
                Action::TaskUpdate {
                    id,
                    data: "work n=5".into()
                },
                Action::TaskEnd { id },
            ]
        );
    }
}
