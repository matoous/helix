use std::collections::HashMap;

use crate::registry::DebugAdapterId;

#[derive(Debug)]
pub enum ProgressStatus {
    Started { title: String },
}

#[derive(Default, Debug)]
pub struct DapProgressMap(HashMap<DebugAdapterId, HashMap<String, ProgressStatus>>);

impl DapProgressMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_progressing(&self, id: DebugAdapterId) -> bool {
        self.0.get(&id).map(|it| !it.is_empty()).unwrap_or_default()
    }

    pub fn title(&self, id: DebugAdapterId, progress_id: &str) -> Option<&String> {
        self.0.get(&id).and_then(|values| {
            values.get(progress_id).map(|status| match status {
                ProgressStatus::Started { title } => title,
            })
        })
    }

    pub fn start(&mut self, id: DebugAdapterId, progress_id: String, title: String) {
        self.0
            .entry(id)
            .or_default()
            .insert(progress_id, ProgressStatus::Started { title });
    }

    pub fn update(&mut self, id: DebugAdapterId, progress_id: String) {
        self.0
            .entry(id)
            .or_default()
            .entry(progress_id)
            .or_insert_with(|| ProgressStatus::Started {
                title: String::new(),
            });
    }

    pub fn end(&mut self, id: DebugAdapterId, progress_id: &str) -> Option<ProgressStatus> {
        self.0
            .get_mut(&id)
            .and_then(|values| values.remove(progress_id))
    }
}
