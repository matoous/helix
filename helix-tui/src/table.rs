use helix_core::unicode::width::UnicodeWidthStr;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::Style,
    text::{Span, Text},
    widgets::{
        Cell as RatatuiCell, Row as RatatuiRow, StatefulWidget, Table as RatatuiTable, TableState,
    },
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell<'a> {
    pub content: Text<'a>,
}

impl<'a, T> From<T> for Cell<'a>
where
    T: Into<Text<'a>>,
{
    fn from(content: T) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row<'a> {
    pub cells: Vec<Cell<'a>>,
    style: Style,
}

impl<'a> Row<'a> {
    pub fn new<T>(cells: T) -> Self
    where
        T: IntoIterator,
        T::Item: Into<Cell<'a>>,
    {
        Self {
            cells: cells.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }

    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }
}

impl<'a, T: Into<Cell<'a>>> From<T> for Row<'a> {
    fn from(cell: T) -> Self {
        Self::new([cell.into()])
    }
}

#[derive(Debug, Clone)]
pub struct Table<'a> {
    rows: Vec<Row<'a>>,
    widths: Vec<Constraint>,
    style: Style,
    highlight_style: Style,
    highlight_symbol: Option<&'a str>,
    column_spacing: u16,
    header: Option<Row<'a>>,
}

impl<'a> Table<'a> {
    pub fn new<T>(rows: T) -> Self
    where
        T: IntoIterator<Item = Row<'a>>,
    {
        Self {
            rows: rows.into_iter().collect(),
            widths: Vec::new(),
            style: Style::default(),
            highlight_style: Style::default(),
            highlight_symbol: None,
            column_spacing: 1,
            header: None,
        }
    }

    pub fn widths(mut self, widths: &[Constraint]) -> Self {
        self.widths = widths.to_vec();
        self
    }

    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    pub fn highlight_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.highlight_style = style.into();
        self
    }

    pub fn highlight_symbol(mut self, symbol: &'a str) -> Self {
        self.highlight_symbol = Some(symbol);
        self
    }

    pub fn column_spacing(mut self, spacing: u16) -> Self {
        self.column_spacing = spacing;
        self
    }

    pub fn header(mut self, header: Row<'a>) -> Self {
        self.header = Some(header);
        self
    }

    pub fn render_table(
        self,
        area: Rect,
        buffer: &mut Buffer,
        state: &mut TableState,
        truncate_start: bool,
    ) {
        let widths = self.widths.clone();
        let rows = self
            .rows
            .into_iter()
            .map(|row| row.into_ratatui(&widths, truncate_start));
        let mut table = RatatuiTable::new(rows, self.widths)
            .style(self.style)
            .row_highlight_style(self.highlight_style)
            .column_spacing(self.column_spacing);
        if let Some(symbol) = self.highlight_symbol {
            table = table.highlight_symbol(symbol);
        }
        if let Some(header) = self.header {
            table = table.header(header.into_ratatui(&widths, false));
        }

        StatefulWidget::render(table, area, buffer, state);
    }
}

impl<'a> Row<'a> {
    fn into_ratatui(self, widths: &[Constraint], truncate_start: bool) -> RatatuiRow<'a> {
        let cells = self.cells.into_iter().enumerate().map(|(index, mut cell)| {
            if truncate_start {
                if let Some(Constraint::Length(width)) = widths.get(index) {
                    truncate_text_start(&mut cell.content, *width as usize);
                }
            }
            RatatuiCell::new(cell.content)
        });
        RatatuiRow::new(cells).style(self.style)
    }
}

fn truncate_text_start(text: &mut Text<'_>, width: usize) {
    for line in &mut text.lines {
        if line.width() <= width || width == 0 {
            continue;
        }

        let mut remaining = width.saturating_sub(1);
        let mut spans = Vec::new();
        for span in line.spans.iter().rev() {
            let mut content = String::new();
            for grapheme in span.content.graphemes(true).rev() {
                let grapheme_width = grapheme.width();
                if grapheme_width > remaining {
                    break;
                }
                content.insert_str(0, grapheme);
                remaining -= grapheme_width;
            }
            if !content.is_empty() {
                spans.push(Span::styled(content, span.style));
            }
            if remaining == 0 {
                break;
            }
        }
        spans.reverse();
        spans.insert(0, Span::raw("…"));
        line.spans = spans;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_ratatui_and_preserves_start_truncation() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));
        let mut state = TableState::default();
        let table = Table::new([Row::new(["abcdef"])]).widths(&[Constraint::Length(5)]);

        table.render_table(buffer.area, &mut buffer, &mut state, true);

        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(rendered, "…cdef");
    }
}
