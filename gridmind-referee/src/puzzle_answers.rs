use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The current puzzle-race riddle's real answer, per arena -- deliberately
/// kept OUT of `MasterState`/`ScoreboardState`, since that gets broadcast
/// wholesale to every websocket client including the public scoreboard and
/// arena pages. Exposing the answer there would let any team just read it
/// off the projector. Fetched instead via a dedicated `/api/admin/*`
/// endpoint that only the operator console calls. Same pattern as
/// `JoinRegistry` being kept out of `MasterState` for an analogous reason.
#[derive(Clone)]
pub struct PuzzleAnswers {
    entries: Arc<Mutex<HashMap<u32, String>>>,
}

impl PuzzleAnswers {
    pub fn new() -> Self {
        PuzzleAnswers {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Records (or overwrites) the current answer for `arena` -- called
    /// every time a fresh riddle is picked (including on Resend/Restart).
    pub fn set(&self, arena: u32, answer: &str) {
        self.entries
            .lock()
            .expect("puzzle answers lock poisoned")
            .insert(arena, answer.to_string());
    }

    pub fn get(&self, arena: u32) -> Option<String> {
        self.entries
            .lock()
            .expect("puzzle answers lock poisoned")
            .get(&arena)
            .cloned()
    }
}

impl Default for PuzzleAnswers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_returns_the_recorded_answer() {
        let answers = PuzzleAnswers::new();
        answers.set(1, "honey");
        assert_eq!(answers.get(1), Some("honey".to_string()));
    }

    #[test]
    fn get_is_none_for_an_arena_with_nothing_recorded() {
        let answers = PuzzleAnswers::new();
        assert_eq!(answers.get(1), None);
    }

    #[test]
    fn setting_again_overwrites_the_previous_answer_for_that_arena() {
        let answers = PuzzleAnswers::new();
        answers.set(1, "honey");
        answers.set(1, "night");
        assert_eq!(answers.get(1), Some("night".to_string()));
    }

    #[test]
    fn each_arena_tracks_its_own_answer_independently() {
        let answers = PuzzleAnswers::new();
        answers.set(1, "honey");
        answers.set(2, "river");
        assert_eq!(answers.get(1), Some("honey".to_string()));
        assert_eq!(answers.get(2), Some("river".to_string()));
    }
}
