//! Cursor editing shared by every one-line text input (filters, searches,
//! the sniffer prompts, Config Editor fields, the save prompt). Only one
//! input is ever active, so `App::input_cursor` serves them all.

use crossterm::event::KeyCode;

/// Applies an editing key to `text` with the cursor at `cursor`, a char
/// index (`None` = after the last character, where typing appends). Returns
/// whether `code` was an editing key.
pub fn edit(text: &mut String, cursor: &mut Option<usize>, code: KeyCode) -> bool {
    let len = text.chars().count();
    let at = cursor.unwrap_or(len).min(len);
    let byte = |s: &str, i: usize| s.char_indices().nth(i).map_or(s.len(), |(b, _)| b);
    let new_at = match code {
        KeyCode::Char(c) if !c.is_control() => {
            text.insert(byte(text, at), c);
            at + 1
        }
        KeyCode::Backspace => {
            if at > 0 {
                text.remove(byte(text, at - 1));
            }
            at.saturating_sub(1)
        }
        KeyCode::Delete => {
            if at < len {
                text.remove(byte(text, at));
            }
            at
        }
        KeyCode::Left => at.saturating_sub(1),
        KeyCode::Right => (at + 1).min(len),
        KeyCode::Home => 0,
        KeyCode::End => len,
        _ => return false,
    };
    *cursor = (new_at < text.chars().count()).then_some(new_at);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(keys: &[KeyCode]) -> (String, Option<usize>) {
        let (mut text, mut cursor) = ("web-01".to_string(), None);
        for &k in keys {
            edit(&mut text, &mut cursor, k);
        }
        (text, cursor)
    }

    #[test]
    fn edits_at_the_cursor() {
        assert_eq!(apply(&[KeyCode::Char('x')]), ("web-01x".into(), None));
        assert_eq!(apply(&[KeyCode::Home, KeyCode::Char('d')]), ("dweb-01".into(), Some(1)));
        assert_eq!(apply(&[KeyCode::Home, KeyCode::Delete, KeyCode::Delete, KeyCode::Delete]), ("-01".into(), Some(0)));
        assert_eq!(apply(&[KeyCode::Left, KeyCode::Left, KeyCode::Backspace]), ("web01".into(), Some(3)));
        assert_eq!(apply(&[KeyCode::Home, KeyCode::Right, KeyCode::End, KeyCode::Right]), ("web-01".into(), None));
        assert_eq!(apply(&[KeyCode::Home, KeyCode::Backspace, KeyCode::Left]), ("web-01".into(), Some(0)));
        assert!(!edit(&mut String::new(), &mut None, KeyCode::Enter));
    }

    #[test]
    fn handles_multibyte_characters() {
        let (mut text, mut cursor) = ("a…b".to_string(), None);
        edit(&mut text, &mut cursor, KeyCode::Left);
        edit(&mut text, &mut cursor, KeyCode::Backspace);
        assert_eq!((text.as_str(), cursor), ("ab", Some(1)));
    }
}
