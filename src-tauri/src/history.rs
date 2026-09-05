use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub timestamp_secs: u64,
    pub cluster: String,
    pub job_name: String,
    pub job_id: String,
    pub from_state: String,
    pub to_state: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    entries: VecDeque<HistoryEntry>,
}

impl History {
    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.push_back(entry);
        if self.entries.len() > MAX_HISTORY {
            self.entries.pop_front();
        }
    }

    pub fn recent(&self, n: usize) -> Vec<HistoryEntry> {
        self.entries.iter().rev().take(n).cloned().collect()
    }
}

pub type SharedHistory = Arc<Mutex<History>>;
