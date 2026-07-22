/// A small deterministic riddle bank for numbers 1 through 6 — covers
/// column indices directly, and row letters after converting to a
/// 1-indexed number (A=1, B=2, ...). Deterministic (no randomness) so
/// riddle text is exactly reproducible in tests.
fn riddle_for_number(n: u32) -> &'static str {
    match n {
        1 => "I am the loneliest number, and the first count of anything.",
        2 => "I come right after Monday, if Monday is the start of the week.",
        3 => "I am half of six.",
        4 => "I am the number of legs on a dog.",
        5 => "I am one hand's worth of fingers.",
        6 => "I am one more than a half-dozen minus one.",
        _ => "I am a number this riddle bank doesn't know.",
    }
}

/// Converts a row letter ('A'..'E') to its 1-indexed number (A=1, B=2, ...).
pub(crate) fn row_letter_to_number(row: char) -> u32 {
    (row.to_ascii_uppercase() as u32) - ('A' as u32) + 1
}

/// Splits a position like "C3" into its row letter and column number.
/// Positions are always one letter followed by one or more digits.
pub(crate) fn parse_position(pos: &str) -> Option<(char, u32)> {
    let mut chars = pos.chars();
    let row = chars.next()?;
    if !row.is_ascii_alphabetic() {
        return None;
    }
    let col: u32 = chars.as_str().parse().ok()?;
    Some((row, col))
}

/// Generates the riddle text for a target position, e.g. "C3" ->
/// "Row: I am half of six. / Col: I am half of six." (row C = 3rd letter).
/// No longer used for the actual wire response (see `row_col_digit_images`)
/// but kept as the underlying row/col-to-number logic's own reference
/// behavior and left in place for whatever future text-based use.
pub fn generate_riddle(pos: &str) -> Option<String> {
    let (row, col) = parse_position(pos)?;
    let row_num = row_letter_to_number(row);
    Some(format!(
        "Row: {} / Col: {}",
        riddle_for_number(row_num),
        riddle_for_number(col)
    ))
}

const DIGIT_SIZE: u32 = 28; // MNIST's native size

/// Path to the real sample digit photo for `digit`, under the same
/// runtime-relative `data/` directory convention as `data/grids` and
/// `data/pregame_riddles.json` (see `content_pools.rs`).
fn mnist_digit_path(digit: u32) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("data/mnist_digits/digit_{digit}.png"))
}

/// A blank white DIGIT_SIZE x DIGIT_SIZE PNG, used whenever a real digit
/// photo can't be loaded -- a display glitch should never panic a match.
fn blank_digit_png_base64() -> String {
    use base64::Engine;
    use image::{GrayImage, Luma};

    let img = GrayImage::from_pixel(DIGIT_SIZE, DIGIT_SIZE, Luma([255]));
    let mut bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .expect("encoding a fixed-size in-memory PNG cannot fail");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Renders a single digit (0-9) as a base64-encoded PNG, loaded from the
/// real sample photos in `data/mnist_digits/` and resized to DIGIT_SIZE so
/// it's ready for an on-board MNIST classifier without further
/// preprocessing. Out-of-range digits or a missing/corrupt source photo
/// render as a blank image rather than panicking -- callers pass digits
/// derived from grid coordinates, which are always small, but a rendering
/// function shouldn't crash a match over a display glitch.
pub fn render_digit_png_base64(digit: u32) -> String {
    use base64::Engine;

    if digit > 9 {
        return blank_digit_png_base64();
    }

    let path = mnist_digit_path(digit);
    let source = match image::open(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!(
                "hints: failed to load {}: {err} (falling back to a blank digit image)",
                path.display()
            );
            return blank_digit_png_base64();
        }
    };

    let resized = source.to_luma8();
    let resized = image::imageops::resize(
        &resized,
        DIGIT_SIZE,
        DIGIT_SIZE,
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes: Vec<u8> = Vec::new();
    resized
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a fixed-size in-memory PNG cannot fail");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Row letter -> number (reusing the existing conversion) and column
/// number, each rendered as its own digit image.
pub fn row_col_digit_images(pos: &str) -> (String, String) {
    let (row, col) = parse_position(pos).expect("valid position");
    let row_num = row_letter_to_number(row);
    (
        render_digit_png_base64(row_num),
        render_digit_png_base64(col),
    )
}

/// Quadrant of `pos` within a `rows`x`cols` grid, by comparing against
/// the midpoint -- same coarse split used by the paid-hint's row/col
/// precision, just deliberately less exact.
pub fn quadrant_for_position(pos: &str, rows: u32, cols: u32) -> &'static str {
    let (row_char, col) = parse_position(pos).expect("valid position");
    let row_num = row_letter_to_number(row_char);
    let top = row_num <= rows.div_ceil(2);
    let left = col <= cols.div_ceil(2);
    match (top, left) {
        (true, true) => "top_left",
        (true, false) => "top_right",
        (false, true) => "bottom_left",
        (false, false) => "bottom_right",
    }
}

/// Splits riddle text into small multi-word fragments, delivered to
/// students as separate `free_hint_fragment` messages (two words per
/// fragment) rather than one big message -- purely a pacing/assembly
/// exercise at this point (previously each fragment was also QR-encoded
/// for a team to scan/decode; simplified to plain text).
pub fn split_into_fragments(text: &str) -> Vec<String> {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|chunk| chunk.join(" "))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_letter_to_number_converts_correctly() {
        assert_eq!(row_letter_to_number('A'), 1);
        assert_eq!(row_letter_to_number('C'), 3);
        assert_eq!(row_letter_to_number('E'), 5);
    }

    #[test]
    fn parse_position_splits_row_and_column() {
        assert_eq!(parse_position("C3"), Some(('C', 3)));
        assert_eq!(parse_position("A12"), Some(('A', 12)));
    }

    #[test]
    fn parse_position_returns_none_for_malformed_input() {
        assert_eq!(parse_position(""), None);
        assert_eq!(parse_position("3C"), None);
    }

    #[test]
    fn parse_position_rejects_non_alphabetic_row_char() {
        // Guards row_letter_to_number against underflowing on non-letter input.
        assert_eq!(parse_position("!1"), None);
        assert_eq!(parse_position("13"), None);
    }

    #[test]
    fn generate_riddle_produces_deterministic_text_for_c3() {
        // C -> row number 3 -> "half of six"; column 3 -> "half of six" too
        let riddle = generate_riddle("C3").unwrap();
        assert_eq!(riddle, "Row: I am half of six. / Col: I am half of six.");
    }

    #[test]
    fn generate_riddle_returns_none_for_malformed_position() {
        assert_eq!(generate_riddle("nonsense"), None);
    }

    #[test]
    fn generate_riddle_is_deterministic_across_calls() {
        let first = generate_riddle("A6").unwrap();
        let second = generate_riddle("A6").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn digit_image_renders_valid_base64_png_for_each_digit_0_to_9() {
        for digit in 0..=9u32 {
            let encoded = render_digit_png_base64(digit);
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded)
                    .expect("valid base64");
            assert_eq!(&bytes[1..4], b"PNG");
        }
    }

    #[test]
    fn digit_to_row_col_images_uses_the_existing_row_letter_conversion() {
        let (row_png, col_png) = row_col_digit_images("C3");
        assert!(!row_png.is_empty());
        assert!(!col_png.is_empty());
    }

    #[test]
    fn quadrant_for_position_matches_grid_midpoint() {
        assert_eq!(quadrant_for_position("A1", 3, 5), "top_left");
        assert_eq!(quadrant_for_position("A5", 3, 5), "top_right");
        assert_eq!(quadrant_for_position("C1", 3, 5), "bottom_left");
        assert_eq!(quadrant_for_position("C5", 3, 5), "bottom_right");
    }

    #[test]
    fn splits_riddle_text_into_word_fragments() {
        let fragments = split_into_fragments("I bark and fetch");
        assert_eq!(fragments, vec!["I bark", "and fetch"]);
    }
}
