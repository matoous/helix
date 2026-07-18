use helix_core::doc_formatter::FormattedGrapheme;
use helix_core::text_annotations::LineAnnotation;
use helix_core::Position;

use crate::Document;

pub(crate) struct MultiBufferHeaders {
    anchors: Vec<usize>,
    current: usize,
}

impl MultiBufferHeaders {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(doc: &Document) -> Box<dyn LineAnnotation + '_> {
        let text = doc.text();
        let mut anchors: Vec<_> = doc
            .multibuffer()
            .into_iter()
            .flat_map(|multibuffer| {
                multibuffer
                    .segments
                    .iter()
                    .map(|segment| segment.projection_range.start.min(text.len_chars()))
            })
            .collect();
        anchors.sort_unstable();
        anchors.dedup();

        Box::new(Self {
            anchors,
            current: 0,
        })
    }
}

impl LineAnnotation for MultiBufferHeaders {
    fn reset_pos(&mut self, char_idx: usize) -> usize {
        self.current = self.anchors.partition_point(|anchor| *anchor < char_idx);
        self.anchors
            .get(self.current)
            .copied()
            .unwrap_or(usize::MAX)
    }

    fn process_anchor(&mut self, _grapheme: &FormattedGrapheme) -> usize {
        self.current += 1;
        self.anchors
            .get(self.current)
            .copied()
            .unwrap_or(usize::MAX)
    }

    fn insert_virtual_lines_before(
        &mut self,
        char_idx: usize,
        _visual_pos: Position,
        _doc_line: usize,
    ) -> Position {
        if self
            .anchors
            .get(self.current)
            .is_some_and(|anchor| *anchor == char_idx)
        {
            Position::new(1, 0)
        } else {
            Position::new(0, 0)
        }
    }

    fn insert_virtual_lines(
        &mut self,
        _line_end_char_idx: usize,
        _line_end_visual_pos: Position,
        _doc_line: usize,
    ) -> Position {
        Position::new(0, 0)
    }
}
