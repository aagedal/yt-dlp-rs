//! Pure subtitle inventory rendering; never selects or inspects resources.
use crate::VideoInfo;

fn visible(value: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut count = 0;
    let mut last_token_bytes = 0;
    for character in value.chars() {
        let token = if character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            format!("\\u{{{:x}}}", character as u32)
        } else {
            character.to_string()
        };
        let length = token.chars().count();
        if count + length > limit {
            if count == limit {
                result.truncate(result.len() - last_token_bytes);
            }
            result.push('…');
            return result;
        }
        result.push_str(&token);
        count += length;
        last_token_bytes = token.len();
    }
    result
}

/// Render declared language/format metadata without resource URLs or selection.
/// Cells are capped at 128 characters, the caption at 256 and rows at 1024.
pub fn render_subtitles(info: &VideoInfo) -> String {
    let prefix = "[info] Available subtitles for ";
    let mut output = format!(
        "{prefix}{}:\nLANGUAGE  FORMATS\n",
        visible(&info.id, 256 - prefix.len() - 1)
    );
    for (language, formats) in info.subtitles.entries.iter().take(1024) {
        let language = visible(language.as_deref().unwrap_or("null"), 128);
        let mut extensions = String::new();
        for format in formats.iter().take(1024) {
            if !extensions.is_empty() {
                extensions.push_str(", ");
            }
            extensions.push_str(&visible(&format.ext, 128));
            if extensions.chars().count() > 128 {
                break;
            }
        }
        output.push_str(&language);
        output.push_str("  ");
        output.push_str(&visible(&extensions, 128));
        output.push('\n');
    }
    if info.subtitles.entries.len() > 1024 {
        output.push_str("…\n");
    }
    output
}
