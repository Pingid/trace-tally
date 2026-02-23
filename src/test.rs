use std::io::Write;

use crate::{Action, Renderer, TaskId, TaskRenderer};

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

    fn render_task(
        &mut self, target: &mut crate::Target<'_>, task: crate::TaskView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        writeln!(target, "{}{}", indent, task.data())
    }

    fn render_event(
        &mut self, target: &mut crate::Target<'_>, task: crate::EventView<'_, Self>,
    ) -> Result<(), std::io::Error> {
        let indent = " ".repeat(task.depth());
        writeln!(target, "{}{}", indent, task.data())
    }
}

struct TestEnv {
    term: VirtualTerm,
    writer: TaskRenderer<TestRenderer>,
    task: Option<TaskId>,
    counter: usize,
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
            event: message.to_string(),
        });
    }

    fn span(&mut self, name: &str, f: impl FnOnce(&mut Self)) -> TaskId {
        let id = TaskId::new(self.counter);
        self.counter += 1;
        self.writer.update(Action::TaskStart {
            id,
            parent: self.task,
            event: name.to_string(),
        });
        self.task = Some(id);
        f(self);
        self.writer.update(Action::TaskEnd { id });
        self.task = None;
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
