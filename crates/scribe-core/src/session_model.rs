//! Which speech model each dictation uses, decided when it starts.
//!
//! Choosing another model on the Models page while you dictate applies from
//! your next dictation: the one in progress keeps the model it started with
//! for every live segment and for the final transcription, so a dictation is
//! never split across two models and the new model doesn't load in the
//! middle of it. The new model loads in the background once the dictation
//! is done (see `WarmTranscriber::sync_with_settings` and
//! `dictation::transcribe_recording_inner`).

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Recent sessions kept, so a cancelled recording that never releases its
/// entry can't grow the list.
const KEEP: usize = 8;

#[derive(Default)]
pub struct SessionModels {
    pins: Mutex<VecDeque<(String, String)>>,
}

impl SessionModels {
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, String)>> {
        self.pins
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Remembers the model a dictation started with.
    pub fn pin(&self, session_id: &str, model_id: Option<&str>) {
        let Some(model_id) = model_id else {
            return;
        };
        let mut pins = self.lock();
        pins.retain(|(session, _)| session != session_id);
        pins.push_back((session_id.to_string(), model_id.to_string()));
        while pins.len() > KEEP {
            pins.pop_front();
        }
    }

    /// The model `session_id` started with, if it was pinned.
    pub fn pinned(&self, session_id: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|(session, _)| session == session_id)
            .map(|(_, model)| model.clone())
    }

    /// The model a dictation uses: the one it started with, else the one
    /// selected now (a dictation that started before any model was chosen).
    pub fn model_for(&self, session_id: &str, selected_now: Option<&str>) -> Option<String> {
        self.pinned(session_id)
            .or_else(|| selected_now.map(str::to_string))
    }

    /// Forgets a finished dictation.
    pub fn release(&self, session_id: &str) {
        self.lock().retain(|(session, _)| session != session_id);
    }
}

/// The app's one registry.
pub fn global() -> &'static SessionModels {
    static GLOBAL: OnceLock<SessionModels> = OnceLock::new();
    GLOBAL.get_or_init(SessionModels::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dictation_keeps_the_model_it_started_with() {
        let models = SessionModels::default();
        models.pin("s1", Some("base.en"));
        // Picking Parakeet mid-dictation changes the settings, not the pin.
        assert_eq!(models.pinned("s1").as_deref(), Some("base.en"));
        models.release("s1");
        assert_eq!(models.pinned("s1"), None);
    }

    /// The owner's report: switching from Base to Parakeet in the middle of
    /// a dictation. Before this fix the dictation read the selected model
    /// when its first live segment was transcribed and again at the end, so
    /// it took the new model and stalled on cold loads.
    #[test]
    fn switching_mid_dictation_applies_from_the_next_one() {
        let models = SessionModels::default();
        models.pin("first", Some("base.en"));
        // The user picks Parakeet while "first" is still recording.
        let selected_now = Some("parakeet-tdt-0.6b-v3");
        assert_eq!(
            models.model_for("first", selected_now).as_deref(),
            Some("base.en")
        );
        models.release("first");
        // The next dictation starts with the new model.
        models.pin("second", selected_now);
        assert_eq!(
            models.model_for("second", selected_now).as_deref(),
            Some("parakeet-tdt-0.6b-v3")
        );
        // A session that was never pinned follows the selection.
        assert_eq!(
            models.model_for("unknown", selected_now).as_deref(),
            selected_now
        );
    }

    #[test]
    fn no_model_selected_pins_nothing() {
        let models = SessionModels::default();
        models.pin("s1", None);
        assert_eq!(models.pinned("s1"), None);
    }

    #[test]
    fn old_pins_are_dropped() {
        let models = SessionModels::default();
        for index in 0..(KEEP + 3) {
            models.pin(&format!("s{index}"), Some("tiny.en"));
        }
        assert_eq!(models.pinned("s0"), None);
        assert_eq!(
            models.pinned(&format!("s{}", KEEP + 2)).as_deref(),
            Some("tiny.en")
        );
    }
}
