/// A frame-based spinner animation.
///
/// Call [`Spinner::tick`] once per render frame (typically in [`crate::Renderer::on_render_start`])
/// and [`Spinner::frame`] to get the current character.
///
/// ```rust,ignore
/// struct MyRenderer {
///     spinner: Spinner,
/// }
///
/// impl Renderer for MyRenderer {
///     // ...
///     fn on_render_start(&mut self) { self.spinner.tick(); }
///
///     fn render_task_line(&mut self, f: &mut FrameWriter<'_>, task: &TaskView<'_, Self>) -> io::Result<()> {
///         if task.active() {
///             write!(f, "{} ", self.spinner)?;
///         }
///         writeln!(f, "{}", task.data())
///     }
/// }
/// ```
pub struct Spinner {
    frames: &'static [&'static str],
    index: usize,
}

impl Spinner {
    /// Braille dot spinner (the most common choice).
    pub fn dots() -> Self {
        Self {
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            index: 0,
        }
    }

    /// Classic line spinner.
    pub fn line() -> Self {
        Self {
            frames: &["|", "/", "-", "\\"],
            index: 0,
        }
    }

    /// Arrow spinner.
    pub fn arrow() -> Self {
        Self {
            frames: &["←", "↖", "↑", "↗", "→", "↘", "↓", "↙"],
            index: 0,
        }
    }

    /// Custom frames.
    pub fn custom(frames: &'static [&'static str]) -> Self {
        Self { frames, index: 0 }
    }

    /// Advance to the next frame.
    pub fn tick(&mut self) {
        self.index = (self.index + 1) % self.frames.len();
    }

    /// Current frame string.
    pub fn frame(&self) -> &'static str {
        self.frames[self.index]
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::dots()
    }
}

impl std::fmt::Display for Spinner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.frame())
    }
}
