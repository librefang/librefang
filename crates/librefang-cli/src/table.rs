//! Table renderer for CLI output.
//!
//! Two rendering modes:
//! - [`Table::render`] — unicode box-drawing borders, bold headers via
//!   `colored`. Suitable for interactive TTY output.
//! - [`Table::render_ascii`] — pure ASCII (`+ - |`), no ANSI escapes,
//!   safe for log pipes, CI capture, and terminals that mis-render
//!   `LANG=C` unicode boxes.
//!
//! Most call sites should use [`Table::print`], which auto-selects unicode
//! when stdout is a TTY and ASCII otherwise.

use std::io::IsTerminal;

use colored::Colorize;

/// Column alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// A table builder that collects headers and rows, then renders to a
/// Unicode box-drawing string.
pub struct Table {
    headers: Vec<String>,
    alignments: Vec<Align>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Create a new table with the given column headers.
    /// All columns default to left-alignment.
    pub fn new(headers: &[&str]) -> Self {
        let headers: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
        let alignments = vec![Align::Left; headers.len()];
        Self {
            headers,
            alignments,
            rows: Vec::new(),
        }
    }

    /// Override the alignment for a specific column (0-indexed).
    /// Out-of-range indices are silently ignored.
    pub fn align(mut self, col: usize, alignment: Align) -> Self {
        if col < self.alignments.len() {
            self.alignments[col] = alignment;
        }
        self
    }

    /// Add a row. Extra cells are truncated; missing cells are filled with "".
    pub fn add_row(&mut self, cells: &[&str]) {
        let row: Vec<String> = (0..self.headers.len())
            .map(|i| cells.get(i).unwrap_or(&"").to_string())
            .collect();
        self.rows.push(row);
    }

    /// Compute the display width of each column (max of header and all cells).
    fn column_widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.len()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.len());
                }
            }
        }
        widths
    }

    /// Pad a string to the given width according to alignment.
    fn pad(text: &str, width: usize, alignment: Align) -> String {
        let len = text.len();
        if len >= width {
            return text.to_string();
        }
        let diff = width - len;
        match alignment {
            Align::Left => format!("{text}{}", " ".repeat(diff)),
            Align::Right => format!("{}{text}", " ".repeat(diff)),
            Align::Center => {
                let left = diff / 2;
                let right = diff - left;
                format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
            }
        }
    }

    /// Build a horizontal border line.
    /// `left`, `mid`, `right` are the corner/junction characters.
    fn border(widths: &[usize], left: &str, mid: &str, right: &str) -> String {
        let segments: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(w + 2)).collect();
        format!("{left}{}{right}", segments.join(mid))
    }

    /// Render the table to a string with Unicode box-drawing borders.
    ///
    /// Layout:
    /// ```text
    /// ┌──────┬───────┐
    /// │ Name │ Value │
    /// ├──────┼───────┤
    /// │ foo  │ bar   │
    /// └──────┴───────┘
    /// ```
    pub fn render(&self) -> String {
        let widths = self.column_widths();

        let top = Self::border(&widths, "\u{250c}", "\u{252c}", "\u{2510}");
        let sep = Self::border(&widths, "\u{251c}", "\u{253c}", "\u{2524}");
        let bot = Self::border(&widths, "\u{2514}", "\u{2534}", "\u{2518}");

        let mut lines = Vec::new();

        // Top border
        lines.push(top);

        // Header row (bold)
        let header_cells: Vec<String> = self
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!(" {} ", Self::pad(h, widths[i], self.alignments[i]).bold()))
            .collect();
        lines.push(format!("\u{2502}{}\u{2502}", header_cells.join("\u{2502}")));

        // Separator
        lines.push(sep);

        // Data rows
        for row in &self.rows {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| format!(" {} ", Self::pad(cell, widths[i], self.alignments[i])))
                .collect();
            lines.push(format!("\u{2502}{}\u{2502}", cells.join("\u{2502}")));
        }

        // Bottom border
        lines.push(bot);

        lines.join("\n")
    }

    /// Render the table using ASCII glyphs only (`+ - |`), with no ANSI
    /// escapes. Use for pipe / CI / dumb-terminal output where unicode
    /// box-drawing or bold escape sequences would be noise.
    ///
    /// ```text
    /// +------+-------+
    /// | Name | Value |
    /// +------+-------+
    /// | foo  | bar   |
    /// +------+-------+
    /// ```
    pub fn render_ascii(&self) -> String {
        let widths = self.column_widths();

        let border: String = {
            let segs: Vec<String> = widths.iter().map(|w| "-".repeat(w + 2)).collect();
            format!("+{}+", segs.join("+"))
        };

        let mut lines = Vec::with_capacity(self.rows.len() + 4);
        lines.push(border.clone());

        // Header row — no bold/ANSI in ASCII mode.
        let header_cells: Vec<String> = self
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!(" {} ", Self::pad(h, widths[i], self.alignments[i])))
            .collect();
        lines.push(format!("|{}|", header_cells.join("|")));

        lines.push(border.clone());

        for row in &self.rows {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| format!(" {} ", Self::pad(cell, widths[i], self.alignments[i])))
                .collect();
            lines.push(format!("|{}|", cells.join("|")));
        }

        lines.push(border);

        lines.join("\n")
    }

    /// Render using unicode borders on a TTY, ASCII otherwise.
    ///
    /// Detection looks at stdout — that's where the rendered string is most
    /// commonly written. Callers writing to stderr should pick `render` or
    /// `render_ascii` explicitly.
    pub fn render_auto(&self) -> String {
        if std::io::stdout().is_terminal() {
            self.render()
        } else {
            self.render_ascii()
        }
    }

    /// Render the table and print it to stdout.
    ///
    /// Auto-selects unicode vs. ASCII based on whether stdout is a TTY so
    /// piped output (`librefang agents | grep …`) stays clean.
    pub fn print(&self) {
        println!("{}", self.render_auto());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_table() {
        let mut t = Table::new(&["Name", "Age", "City"]);
        t.add_row(&["Alice", "30", "London"]);
        t.add_row(&["Bob", "25", "Paris"]);

        let rendered = t.render();
        let lines: Vec<&str> = rendered.lines().collect();

        // 5 lines: top, header, sep, 2 rows, bottom = 6
        assert_eq!(lines.len(), 6);

        // Top border uses box-drawing
        assert!(lines[0].starts_with('\u{250c}'));
        assert!(lines[0].ends_with('\u{2510}'));

        // Bottom border
        assert!(lines[5].starts_with('\u{2514}'));
        assert!(lines[5].ends_with('\u{2518}'));

        // Header line contains column names (ignore ANSI codes for bold)
        assert!(lines[1].contains("Name"));
        assert!(lines[1].contains("Age"));
        assert!(lines[1].contains("City"));

        // Data rows contain cell values
        assert!(lines[3].contains("Alice"));
        assert!(lines[3].contains("30"));
        assert!(lines[3].contains("London"));
        assert!(lines[4].contains("Bob"));
        assert!(lines[4].contains("25"));
        assert!(lines[4].contains("Paris"));
    }

    #[test]
    fn right_alignment() {
        let mut t = Table::new(&["Item", "Count"]);
        t = t.align(1, Align::Right);
        t.add_row(&["apples", "5"]);
        t.add_row(&["oranges", "123"]);

        let rendered = t.render();
        // The "5" should be right-padded on the left within its column
        // Find the data line with "5"
        let line = rendered.lines().find(|l| l.contains("apples")).unwrap();
        // After the second box char, the number should be right-aligned
        assert!(line.contains("   5"));
    }

    #[test]
    fn center_alignment() {
        let pad = Table::pad("hi", 6, Align::Center);
        assert_eq!(pad, "  hi  ");

        let pad_odd = Table::pad("hi", 7, Align::Center);
        assert_eq!(pad_odd, "  hi   ");
    }

    #[test]
    fn empty_table() {
        let t = Table::new(&["A", "B"]);
        let rendered = t.render();
        let lines: Vec<&str> = rendered.lines().collect();
        // top, header, sep, bottom = 4 lines (no data rows)
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn missing_cells_filled() {
        let mut t = Table::new(&["X", "Y", "Z"]);
        t.add_row(&["only-one"]);

        let rendered = t.render();
        // Row should still have 3 columns; missing ones are empty
        let data_line = rendered.lines().nth(3).unwrap();
        // Count box-drawing vertical bars in data line
        let bars = data_line.matches('\u{2502}').count();
        assert_eq!(bars, 4); // left + 2 inner + right
    }

    #[test]
    fn wide_cells_auto_width() {
        let mut t = Table::new(&["ID", "Description"]);
        t.add_row(&["1", "A very long description string"]);

        let rendered = t.render();
        assert!(rendered.contains("A very long description string"));
        // The top border should be wide enough to contain the description
        let top = rendered.lines().next().unwrap();
        // At minimum: 2 padding + description length for second column
        assert!(top.len() > 30);
    }

    #[test]
    fn ascii_render_uses_only_ascii_glyphs() {
        // The ASCII path must be safe for any encoding / dumb terminal:
        // no unicode box-drawing chars, no ANSI escapes from `colored`.
        let mut t = Table::new(&["Name", "Value"]);
        t.add_row(&["foo", "bar"]);
        let rendered = t.render_ascii();

        for ch in rendered.chars() {
            assert!(
                ch.is_ascii(),
                "non-ASCII glyph {:?} leaked into render_ascii output",
                ch
            );
        }
        // No ESC byte (start of an ANSI escape sequence).
        assert!(!rendered.contains('\u{1b}'), "ANSI escape leaked");

        // Borders use `+` corners and `-` edges.
        let top = rendered.lines().next().unwrap();
        assert!(top.starts_with('+') && top.ends_with('+'));
        assert!(top.chars().all(|c| c == '+' || c == '-'));
    }

    #[test]
    fn ascii_render_aligns_columns() {
        // Right-align a numeric column and confirm the digit lands at the
        // right edge of its cell (one space padding after the digit before
        // the closing `|`).
        let mut t = Table::new(&["Item", "Count"]);
        t = t.align(1, Align::Right);
        t.add_row(&["apples", "5"]);
        t.add_row(&["oranges", "123"]);

        let rendered = t.render_ascii();
        let line = rendered.lines().find(|l| l.contains("apples")).unwrap();
        // "Count" header is 5 chars wide; "5" right-aligned ⇒ "    5".
        assert!(
            line.contains("|     5 |"),
            "expected right-aligned 5 in {line}",
        );
    }

    #[test]
    fn ascii_and_unicode_render_have_same_row_count() {
        // The two renderers should agree on layout (top, header, sep, rows,
        // bottom) — only the glyphs differ.
        let mut t = Table::new(&["A", "B", "C"]);
        t.add_row(&["1", "2", "3"]);
        t.add_row(&["4", "5", "6"]);

        let u = t.render();
        let a = t.render_ascii();
        assert_eq!(u.lines().count(), a.lines().count());
    }
}
