//! A small, frontend-agnostic action log -- narrates "an action
//! succeeded/failed" the same way for every UI, so `ui::cli` and
//! `ui::wasm` share the exact same type and push the exact same
//! messages, diverging only in how each one renders `Entry` (`ui::cli`
//! flushes to stderr once at the end of a run; `ui::wasm` holds
//! entries for JS to drain into the page's log view). Deliberately
//! *not* a field on `Ctx` -- this is session/UI state, not persisted
//! domain data (a fresh `Logs` starts empty every run/page-load,
//! unlike everything `Ctx` holds), so each `ui` adapter owns its own
//! instance alongside its `Ctx`, the same way it owns its own
//! persistence choice (see `ctx.rs`'s own doc).
//!
//! Not every message a UI shows goes through here: a value the user
//! needs to see because there's nowhere else to show it (a freshly
//! generated signing key, an exported blob) stays a direct
//! println!/DOM write in each `ui` adapter -- same reasoning as
//! `ui::wasm`'s own "this file is pure boilerplate" rule: `Logs` is
//! for narration, not for data this crate has no other way to
//! surface.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Level {
    Info,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub level: Level,
    pub message: String,
}

/// Oldest-dropped cap, so a long-lived browser session doesn't grow
/// this unboundedly.
const CAP: usize = 200;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Logs {
    entries: Vec<Entry>,
}

impl Logs {
    pub fn push(&mut self, level: Level, message: impl Into<String>) {
        self.entries.push(Entry {
            level,
            message: message.into(),
        });
        if self.entries.len() > CAP {
            self.entries.remove(0);
        }
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.push(Level::Info, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.push(Level::Error, message);
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_at_oldest_dropped() {
        let mut logs = Logs::default();
        for i in 0..(CAP + 10) {
            logs.info(format!("entry {i}"));
        }
        assert_eq!(logs.entries().len(), CAP);
        assert_eq!(logs.entries()[0].message, "entry 10");
    }

    #[test]
    fn clear_empties() {
        let mut logs = Logs::default();
        logs.error("oops");
        assert_eq!(logs.entries().len(), 1);
        logs.clear();
        assert!(logs.entries().is_empty());
    }
}
