pub mod backend;
mod surface;
mod table;
pub mod terminal;

pub mod buffer {
    pub use crate::surface::BufferExt;
    pub use ratatui::buffer::*;
}

pub mod layout {
    pub use ratatui::layout::*;
}

pub mod symbols {
    pub use ratatui::symbols::*;
}

pub mod style {
    pub use ratatui::style::*;
}

pub mod text {
    pub use ratatui::text::*;
}

pub mod widgets {
    pub use crate::table::{Cell, Row, Table};
    pub use ratatui::widgets::{
        Block, BorderType, Borders, Paragraph, StatefulWidget, TableState, Widget, Wrap,
    };
}

pub use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
