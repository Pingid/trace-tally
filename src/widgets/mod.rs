//! Rendering utilities for task output.
//!
//! # Progress bar
//!
//! [`ProgressBar`] renders a configurable inline bar with percentage:
//!
//! ```rust,ignore
//! let bar = ProgressBar::new(45, 100).width(30);
//! writeln!(f, "{} {bar}", task.data())?;
//! // => my_task [██████████████░░░░░░░░░░░░░░░░]  45%
//!
//! // Custom fill characters:
//! let bar = ProgressBar::new(3, 10).chars('#', '.');
//! // => [######..............] 30%
//! ```
//!
//! # Spinner
//!
//! [`Spinner`] cycles through animation frames on each [`tick`](Spinner::tick):
//!
//! ```rust,ignore
//! let mut spinner = Spinner::dots(); // ⠋ ⠙ ⠹ ...
//! spinner.tick();
//! write!(f, "{} working...", spinner.frame())?;
//!
//! // Other presets:
//! let s = Spinner::line();  // | / - \
//! let s = Spinner::arrow(); // ← ↖ ↑ ↗ → ↘ ↓ ↙
//!
//! // Custom frames:
//! let s = Spinner::custom(&["🌑", "🌒", "🌓", "🌔", "🌕"]);
//! ```
//!
//! # Tree indent
//!
//! [`TreeIndent`] produces box-drawing prefixes for hierarchical task trees:
//!
//! ```rust,ignore
//! fn render_task_line(&mut self, f: &mut Frame<'_>, task: &TaskView<'_, String, String>) -> io::Result<()> {
//!     write!(f, "{}", TreeIndent::of(task))?;
//!     writeln!(f, "{}", task.data())
//! }
//! ```
//!
//! Output:
//!
//! ```text
//! root task
//! ├── child 1
//! │   ├── grandchild a
//! │   └── grandchild b
//! └── child 2
//! ```

mod progress_bar;
mod spinner;
mod tree_indent;

pub use progress_bar::*;
pub use spinner::*;
pub use tree_indent::*;
