/// PSPad-inspired Clip / Template Completion engine (`Ctrl+J` or `Ctrl+Space`).
/// Looks up the identifier immediately preceding the cursor, expands it with
/// proper indentation matching the current line, and positions the caret at `|`.
use crate::brace_matcher::char_to_byte_idx;

#[derive(Debug, Clone)]
pub struct ClipTemplate {
    pub trigger: &'static str,
    pub description: &'static str,
    pub body: &'static str, // Uses `|` to mark where the cursor should land
}

pub const BUILTIN_TEMPLATES: &[ClipTemplate] = &[
    ClipTemplate {
        trigger: "fn",
        description: "Function definition",
        body: "fn |() {\n    \n}",
    },
    ClipTemplate {
        trigger: "pfn",
        description: "Public function definition",
        body: "pub fn |() {\n    \n}",
    },
    ClipTemplate {
        trigger: "main",
        description: "Main entry point",
        body: "fn main() {\n    |\n}",
    },
    ClipTemplate {
        trigger: "for",
        description: "For-in range loop",
        body: "for i in 0..| {\n    \n}",
    },
    ClipTemplate {
        trigger: "if",
        description: "If conditional block",
        body: "if | {\n    \n}",
    },
    ClipTemplate {
        trigger: "ifel",
        description: "If / Else block",
        body: "if | {\n    \n} else {\n    \n}",
    },
    ClipTemplate {
        trigger: "match",
        description: "Match expression",
        body: "match | {\n    _ => {}\n}",
    },
    ClipTemplate {
        trigger: "struct",
        description: "Struct with Debug & Clone",
        body: "#[derive(Debug, Clone)]\npub struct | {\n    \n}",
    },
    ClipTemplate {
        trigger: "enum",
        description: "Enum with Debug, Clone, Copy, PartialEq",
        body: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum | {\n    \n}",
    },
    ClipTemplate {
        trigger: "impl",
        description: "Implementation block",
        body: "impl | {\n    \n}",
    },
    ClipTemplate {
        trigger: "test",
        description: "Unit test module & function",
        body: "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn test_|() {\n        \n    }\n}",
    },
    ClipTemplate {
        trigger: "pr",
        description: "println! macro",
        body: "println!(\"|{}\", );",
    },
    ClipTemplate {
        trigger: "epr",
        description: "eprintln! macro",
        body: "eprintln!(\"|{}\", );",
    },
];

/// Attempts to expand a template trigger word immediately preceding `cursor_char_idx`.
/// Returns `Some((new_cursor_char_idx, trigger_name))` if a template was matched and expanded.
pub fn try_expand_template(
    text: &mut String,
    cursor_char_idx: usize,
) -> Option<(usize, &'static str)> {
    let cursor_byte = char_to_byte_idx(text, cursor_char_idx);
    if cursor_byte == 0 {
        return None;
    }

    // Walk backward from cursor_byte to extract the word before the cursor
    let bytes = text.as_bytes();
    let mut word_start = cursor_byte;
    while word_start > 0
        && (bytes[word_start - 1].is_ascii_alphanumeric() || bytes[word_start - 1] == b'_')
    {
        word_start -= 1;
    }

    if word_start == cursor_byte {
        return None;
    }

    let word = &text[word_start..cursor_byte];
    let template = BUILTIN_TEMPLATES.iter().find(|t| t.trigger == word)?;

    // Determine the indentation of the current line so multi-line snippets align cleanly
    let line_start = text[..word_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent: String = text[line_start..word_start]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();

    // Indent subsequent lines of the template body
    let mut expanded = String::new();
    for (idx, line) in template.body.lines().enumerate() {
        if idx > 0 {
            expanded.push('\n');
            expanded.push_str(&indent);
        }
        expanded.push_str(line);
    }

    // Locate the `|` cursor placeholder and remove it
    let pipe_byte_offset = expanded.find('|').unwrap_or(expanded.len());
    let clean_expanded = expanded.replacen('|', "", 1);

    // Replace `word` in `text` with `clean_expanded`
    text.replace_range(word_start..cursor_byte, &clean_expanded);

    // Compute the new cursor position in `char` coordinates
    let new_cursor_byte = word_start + pipe_byte_offset;
    let new_cursor_char = text[..new_cursor_byte].chars().count();

    Some((new_cursor_char, template.trigger))
}

/// Inserts a specific template at the current cursor position (used when clicking from the Clips sidebar/menu)
pub fn insert_template_at_cursor(
    text: &mut String,
    cursor_char_idx: usize,
    template: &ClipTemplate,
) -> usize {
    let cursor_byte = char_to_byte_idx(text, cursor_char_idx);
    let line_start = text[..cursor_byte].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent: String = text[line_start..cursor_byte]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();

    let mut expanded = String::new();
    for (idx, line) in template.body.lines().enumerate() {
        if idx > 0 {
            expanded.push('\n');
            expanded.push_str(&indent);
        }
        expanded.push_str(line);
    }

    let pipe_byte_offset = expanded.find('|').unwrap_or(expanded.len());
    let clean_expanded = expanded.replacen('|', "", 1);

    text.insert_str(cursor_byte, &clean_expanded);
    let new_cursor_byte = cursor_byte + pipe_byte_offset;
    text[..new_cursor_byte].chars().count()
}
