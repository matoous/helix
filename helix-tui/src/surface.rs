use helix_core::unicode::width::UnicodeWidthStr;
use helix_view::graphics::Style;
use ratatui::{buffer::Buffer, text::Line};
use unicode_segmentation::UnicodeSegmentation;

/// Editor-specific operations on Ratatui's screen buffer.
pub trait BufferExt {
    fn in_bounds(&self, x: u16, y: u16) -> bool;
    fn set_spans(&mut self, x: u16, y: u16, spans: &Line<'_>, width: u16) -> (u16, u16);
    fn set_grapheme(&mut self, x: u16, y: u16, grapheme: &str, width: usize, style: Style);
    fn set_tab(&mut self, x: u16, y: u16, tab: &str, style: Style);

    #[allow(clippy::too_many_arguments)]
    fn set_string_anchored(
        &mut self,
        x: u16,
        y: u16,
        truncate_start: bool,
        truncate_end: bool,
        string: &str,
        width: usize,
        style: impl Fn(usize) -> Style,
    ) -> (u16, u16);

    #[allow(clippy::too_many_arguments)]
    fn set_string_truncated(
        &mut self,
        x: u16,
        y: u16,
        string: &str,
        width: usize,
        style: impl Fn(usize) -> Style,
        ellipsis: bool,
        truncate_start: bool,
    ) -> (u16, u16);

    fn clear_with(&mut self, area: ratatui::layout::Rect, style: Style);
}

impl BufferExt for Buffer {
    fn in_bounds(&self, x: u16, y: u16) -> bool {
        self.cell((x, y)).is_some()
    }

    fn set_spans(&mut self, x: u16, y: u16, spans: &Line<'_>, width: u16) -> (u16, u16) {
        self.set_line(x, y, spans, width)
    }

    #[inline]
    fn set_grapheme(&mut self, x: u16, y: u16, grapheme: &str, width: usize, style: Style) {
        let index = self.index_of(x, y);
        self.content[index].set_symbol(grapheme).set_style(style);
        for cell in &mut self.content[index + 1..index + width] {
            cell.reset();
        }
    }

    #[inline]
    fn set_tab(&mut self, x: u16, y: u16, tab: &str, style: Style) {
        let mut index = self.index_of(x, y);
        for ch in tab.chars() {
            self.content[index].set_char(ch).set_style(style);
            index += 1;
        }
    }

    fn set_string_anchored(
        &mut self,
        x: u16,
        y: u16,
        truncate_start: bool,
        truncate_end: bool,
        string: &str,
        width: usize,
        style: impl Fn(usize) -> Style,
    ) -> (u16, u16) {
        if self.cell((x, y)).is_none() || width == 0 {
            return (x, y);
        }

        let mut index = self.index_of(x, y);
        let mut rendered_width = 0;
        let mut graphemes = string.grapheme_indices(true);

        if truncate_start {
            for _ in 0..graphemes.next().map(|(_, g)| g.width()).unwrap_or_default() {
                self.content[index].set_symbol("…");
                index += 1;
                rendered_width += 1;
            }
        }

        for (byte_offset, grapheme) in graphemes {
            let grapheme_width = grapheme.width();
            if truncate_end && rendered_width + grapheme_width >= width {
                break;
            }
            if grapheme_width == 0 {
                continue;
            }
            self.content[index]
                .set_symbol(grapheme)
                .set_style(style(byte_offset));
            for cell in &mut self.content[index + 1..index + grapheme_width] {
                cell.reset();
            }
            index += grapheme_width;
            rendered_width += grapheme_width;
        }

        if truncate_end {
            for cell in &mut self.content[index..index + width.saturating_sub(rendered_width)] {
                cell.set_symbol("…");
            }
        }
        (x, y)
    }

    fn set_string_truncated(
        &mut self,
        x: u16,
        y: u16,
        string: &str,
        width: usize,
        style: impl Fn(usize) -> Style,
        ellipsis: bool,
        truncate_start: bool,
    ) -> (u16, u16) {
        if self.cell((x, y)).is_none() || width == 0 {
            return (x, y);
        }

        let width = width.min(self.area.right().saturating_sub(x) as usize);
        let content_width = string.width();
        let available = width.saturating_sub(usize::from(ellipsis));
        let truncated = content_width > available;
        let mut x_offset = x;

        let graphemes: Vec<_> = string.grapheme_indices(true).collect();
        let iter: Box<dyn Iterator<Item = &(usize, &str)> + '_> = if truncate_start {
            Box::new(graphemes.iter().rev())
        } else {
            Box::new(graphemes.iter())
        };

        if truncate_start {
            let min_x = if truncated && ellipsis {
                self[(x, y)].set_symbol("…");
                x.saturating_add(1)
            } else {
                x
            };
            let end = x.saturating_add(if truncated {
                width as u16
            } else {
                content_width as u16
            });
            let mut cursor = end;
            for (byte_offset, grapheme) in iter {
                let grapheme_width = grapheme.width() as u16;
                let Some(start) = cursor.checked_sub(grapheme_width) else {
                    break;
                };
                if start < min_x {
                    break;
                }
                self[(start, y)]
                    .set_symbol(grapheme)
                    .set_style(style(*byte_offset));
                for column in start + 1..cursor {
                    self[(column, y)].reset();
                }
                cursor = start;
                x_offset += grapheme_width;
            }
        } else {
            for (byte_offset, grapheme) in iter {
                let grapheme_width = grapheme.width() as u16;
                if x_offset.saturating_sub(x) as usize + grapheme_width as usize > available {
                    break;
                }
                self[(x_offset, y)]
                    .set_symbol(grapheme)
                    .set_style(style(*byte_offset));
                for column in x_offset + 1..x_offset + grapheme_width {
                    self[(column, y)].reset();
                }
                x_offset += grapheme_width;
            }
            if truncated && ellipsis {
                self[(x_offset, y)].set_symbol("…");
            }
        }
        (x_offset, y)
    }

    fn clear_with(&mut self, area: ratatui::layout::Rect, style: Style) {
        let area = self.area.intersection(area);
        for position in area.positions() {
            self[position].reset();
            self[position].set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn symbols(buffer: &Buffer) -> String {
        buffer.content.iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn truncates_the_end_with_an_ellipsis() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));

        buffer.set_string_truncated(0, 0, "abcdef", 5, |_| Style::default(), true, false);

        assert_eq!(symbols(&buffer), "abcd…");
    }

    #[test]
    fn truncates_the_start_with_an_ellipsis() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));

        let end = buffer.set_string_truncated(0, 0, "abcdef", 5, |_| Style::default(), true, true);

        assert_eq!(symbols(&buffer), "…cdef");
        assert_eq!(end, (4, 0));
    }

    #[test]
    fn start_truncation_keeps_short_text_left_aligned() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));

        let end = buffer.set_string_truncated(0, 0, "abc", 5, |_| Style::default(), true, true);

        assert_eq!(symbols(&buffer), "abc  ");
        assert_eq!(end, (3, 0));
    }

    #[test]
    fn clears_only_the_intersection_with_the_buffer() {
        let mut buffer = Buffer::with_lines(["abcdef"]);

        buffer.clear_with(Rect::new(4, 0, 5, 1), Style::default());

        assert_eq!(symbols(&buffer), "abcd  ");
    }
}
