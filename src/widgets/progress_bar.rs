// A configurable progress bar renderer.
///
/// ```rust,ignore
/// let bar = ProgressBar::new(45, 100).width(30);
/// writeln!(f, "{} {bar}", task.data())?;
/// // => my_task [██████████████░░░░░░░░░░░░░░░░]  45%
/// ```
pub struct ProgressBar {
    done: u64,
    total: u64,
    width: usize,
    filled: char,
    empty: char,
}

impl ProgressBar {
    pub fn new(done: u64, total: u64) -> Self {
        Self {
            done,
            total,
            width: 20,
            filled: '█',
            empty: '░',
        }
    }

    pub fn width(mut self, w: usize) -> Self {
        self.width = w;
        self
    }

    pub fn chars(mut self, filled: char, empty: char) -> Self {
        self.filled = filled;
        self.empty = empty;
        self
    }

    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64).clamp(0.0, 1.0)
    }
}

impl std::fmt::Display for ProgressBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ratio = self.ratio();
        let filled = (ratio * self.width as f64) as usize;
        let empty = self.width - filled;
        write!(
            f,
            "[{}{}] {:3.0}%",
            std::iter::repeat_n(self.filled, filled).collect::<String>(),
            std::iter::repeat_n(self.empty, empty).collect::<String>(),
            ratio * 100.0,
        )
    }
}
