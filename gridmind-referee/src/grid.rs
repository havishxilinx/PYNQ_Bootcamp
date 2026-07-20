use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Deserialize)]
struct GridFile {
    positions: HashMap<String, String>,
}

/// Loads the {position -> object_class} golden answer key from a JSON file,
/// e.g. {"positions": {"A1": "dog", "A2": "boat"}}.
pub fn load_grid(path: &str) -> Result<HashMap<String, String>> {
    let data =
        fs::read_to_string(path).with_context(|| format!("failed to read grid file at {path}"))?;
    let parsed: GridFile = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse grid JSON at {path}"))?;
    Ok(parsed.positions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_positions_from_grid_file() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, r#"{{"positions": {{"A1": "dog", "A2": "boat"}}}}"#).unwrap();
        let grid = load_grid(file.path().to_str().unwrap()).unwrap();
        assert_eq!(grid.get("A1"), Some(&"dog".to_string()));
        assert_eq!(grid.get("A2"), Some(&"boat".to_string()));
    }

    #[test]
    fn errors_on_missing_file() {
        let result = load_grid("/nonexistent/path/grid.json");
        assert!(result.is_err());
    }
}
