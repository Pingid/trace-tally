use std::io::Write;

/// Write target passed to [`Renderer`](crate::Renderer) callbacks.
///
/// Use `write!` / `writeln!` to produce output. When the frame belongs to a
/// [`LiveRegion`], it counts the terminal rows written (including soft-wrapped
/// rows when a width is set) so the region can be erased next frame.
pub struct Frame<'a> {
    target: &'a mut dyn Write,
    rows: Option<&'a mut usize>,
    width: Option<usize>,
    col: usize,
    escape: Escape,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    None,
    Esc,
    Csi,
}

impl<'a> Frame<'a> {
    /// Creates a frame that writes to `target` without tracking rows.
    pub fn new(target: &'a mut dyn Write) -> Self {
        Self {
            target,
            rows: None,
            width: None,
            col: 0,
            escape: Escape::None,
        }
    }

    fn tracked(target: &'a mut dyn Write, rows: &'a mut usize, width: Option<usize>) -> Self {
        Self {
            target,
            rows: Some(rows),
            width,
            col: 0,
            escape: Escape::None,
        }
    }

    fn track(&mut self, buf: &[u8]) {
        let Some(rows) = self.rows.as_deref_mut() else {
            return;
        };
        for &b in buf {
            match (self.escape, b) {
                (Escape::None, 0x1b) => self.escape = Escape::Esc,
                (Escape::Esc, b'[') => self.escape = Escape::Csi,
                (Escape::Esc, _) => self.escape = Escape::None,
                (Escape::Csi, 0x40..=0x7e) => self.escape = Escape::None,
                (Escape::Csi, _) => {}
                (Escape::None, b'\n') => {
                    *rows += 1;
                    self.col = 0;
                }
                (Escape::None, b'\r') => self.col = 0,
                // UTF-8 continuation bytes don't advance the cursor.
                (Escape::None, 0x80..=0xbf) => {}
                (Escape::None, b) if b < 0x20 => {}
                (Escape::None, _) => {
                    if self.width.is_some_and(|w| w > 0 && self.col == w) {
                        *rows += 1;
                        self.col = 0;
                    }
                    self.col += 1;
                }
            }
        }
    }
}

impl Write for Frame<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.target.write(buf)?;
        self.track(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.target.flush()
    }
}

/// An erasable region at the bottom of an inline terminal.
///
/// Each frame, call [`clear`](Self::clear) to erase what was drawn last
/// time, write permanent output with [`committed`](Self::committed), then
/// redraw the region with [`live`](Self::live).
///
/// ```rust
/// use std::io::Write;
/// use trace_tally::LiveRegion;
///
/// let mut out = Vec::new();
/// let mut region = LiveRegion::new();
/// region.clear(&mut out)?;
/// writeln!(region.committed(&mut out), "finished: build")?;
/// writeln!(region.live(&mut out), "running: test")?;
/// assert_eq!(region.rows(), 1);
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct LiveRegion {
    rows: usize,
    width: Option<usize>,
}

impl LiveRegion {
    pub fn new() -> Self {
        Self::default()
    }

    /// Terminal width in columns, used to count soft-wrapped rows.
    ///
    /// Without a width, lines longer than the terminal leave stale output
    /// behind when the region is cleared. Characters are counted as one
    /// column each, so wide glyphs (CJK, emoji) are under-counted.
    pub fn set_width(&mut self, width: Option<usize>) {
        self.width = width;
    }

    /// Number of rows drawn in the region since the last [`clear`](Self::clear).
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Erases the region drawn by the previous frame.
    pub fn clear(&mut self, target: &mut dyn Write) -> std::io::Result<()> {
        if self.rows > 0 {
            write!(target, "\r\x1b[{}A\x1b[2K\x1b[J", self.rows)?;
            target.flush()?;
        }
        self.rows = 0;
        Ok(())
    }

    /// Frame for permanent output written above the region.
    pub fn committed<'a>(&self, target: &'a mut dyn Write) -> Frame<'a> {
        Frame::new(target)
    }

    /// Frame for output that will be erased by the next [`clear`](Self::clear).
    pub fn live<'a>(&'a mut self, target: &'a mut dyn Write) -> Frame<'a> {
        Frame::tracked(target, &mut self.rows, self.width)
    }
}
