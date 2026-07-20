use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PregameRiddle {
    pub riddle: String,
    pub answer: String,
}

/// Reads the riddle pool fresh from disk every call -- this is the whole
/// "live hot-reload" mechanism: edit the file, the next call sees the
/// change. No caching, no file-watcher thread.
pub fn load_pregame_riddles(path: &str) -> Result<Vec<PregameRiddle>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading pregame riddle pool at {path}"))?;
    serde_json::from_str(&text).with_context(|| format!("parsing pregame riddle pool at {path}"))
}

/// Picks a riddle not yet in `used` (by answer, tracked in-memory per
/// tournament) and marks it used. Resets `used` and picks again once the
/// whole pool has been exhausted, rather than ever returning `None` for a
/// non-empty pool.
pub fn pick_unused<'a>(
    pool: &'a [PregameRiddle],
    used: &mut HashSet<String>,
) -> Option<&'a PregameRiddle> {
    if pool.is_empty() {
        return None;
    }
    if used.len() >= pool.len() {
        used.clear();
    }
    use rand::seq::SliceRandom;
    let available: Vec<&PregameRiddle> =
        pool.iter().filter(|r| !used.contains(&r.answer)).collect();
    let chosen = *available.choose(&mut rand::thread_rng())?;
    used.insert(chosen.answer.clone());
    Some(chosen)
}

pub fn load_object_riddle(path: &str, class_name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let bank: HashMap<String, String> = serde_json::from_str(&text).ok()?;
    bank.get(class_name).cloned()
}

pub fn load_quadrant_riddle(path: &str, quadrant: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let bank: HashMap<String, String> = serde_json::from_str(&text).ok()?;
    bank.get(quadrant).cloned()
}

/// Lists every `.json` grid file in the pool directory, as full relative
/// paths ready to pass straight to `grid::load_grid`. Errors loudly on an
/// empty or missing directory -- a missing grid pool the night before the
/// event should be caught immediately at Close Registration time, not
/// discovered when the first match tries to start.
pub fn list_grid_pool(dir: &str) -> Result<Vec<String>> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading grid pool directory {dir}"))?;
    let grids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|p| p.to_str().map(String::from))
        .collect();
    if grids.is_empty() {
        anyhow::bail!("grid pool directory {dir} contains no .json files");
    }
    Ok(grids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_pregame_riddles_from_disk() {
        let riddles = load_pregame_riddles("data/pregame_riddles.json").unwrap();
        assert!(riddles.len() >= 8);
        assert!(riddles
            .iter()
            .all(|r| !r.riddle.is_empty() && !r.answer.is_empty()));
    }

    #[test]
    fn picks_unused_riddle_and_tracks_it() {
        let pool = vec![
            PregameRiddle {
                riddle: "a".into(),
                answer: "1".into(),
            },
            PregameRiddle {
                riddle: "b".into(),
                answer: "2".into(),
            },
        ];
        let mut used = HashSet::new();
        let first = pick_unused(&pool, &mut used).unwrap().clone();
        assert_eq!(used.len(), 1);
        let second = pick_unused(&pool, &mut used).unwrap().clone();
        assert_ne!(first.answer, second.answer);
        assert_eq!(used.len(), 2);
        // pool exhausted -- resets and picks again rather than returning None
        let third = pick_unused(&pool, &mut used).unwrap();
        assert_eq!(used.len(), 1);
        assert!(third.answer == "1" || third.answer == "2");
    }

    #[test]
    fn loads_object_riddle_by_class_name() {
        let riddle = load_object_riddle("data/object_riddles.json", "dog").unwrap();
        assert!(riddle.contains("bark"));
    }

    #[test]
    fn object_riddle_returns_none_for_unknown_class() {
        assert!(load_object_riddle("data/object_riddles.json", "spaceship").is_none());
    }

    #[test]
    fn loads_quadrant_riddle() {
        let riddle = load_quadrant_riddle("data/quadrant_riddles.json", "top_left").unwrap();
        assert!(riddle.contains("first page"));
    }

    #[test]
    fn lists_grid_files_in_the_pool_directory() {
        let grids = list_grid_pool("data/grids").unwrap();
        assert!(grids.len() >= 2);
        assert!(grids.iter().all(|g| g.ends_with(".json")));
    }

    #[test]
    fn empty_grid_pool_directory_returns_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = list_grid_pool(dir.path().to_str().unwrap());
        assert!(result.is_err());
    }
}
