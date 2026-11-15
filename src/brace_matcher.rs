/// Zero-dependency delimiter and curly-brace matcher for Rust source code.
/// Properly ignores delimiters inside string literals (`"..."`, `r#"..."#`),
/// char literals (`'{'`), line comments (`//`), and block comments (`/* ... */`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BraceMatch {
    pub open_byte: usize,
    pub close_byte: usize,
    pub open_char: char,
    pub close_char: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenDelimiter {
    Open(char, usize),
    Close(char, usize),
}

/// Scans the source code into a flat list of structural delimiters (ignoring strings and comments)
/// and checks if the cursor (in char index or byte index) is adjacent to a delimiter.
pub fn find_matching_brace(text: &str, cursor_char_idx: usize) -> Option<BraceMatch> {
    if text.is_empty() {
        return None;
    }

    // Convert cursor_char_idx to byte offset safely
    let cursor_byte = char_to_byte_idx(text, cursor_char_idx);

    let delimiters = extract_structural_delimiters(text);
    if delimiters.is_empty() {
        return None;
    }

    // Check if the character at cursor_byte or right before cursor_byte is a structural delimiter
    let prev_byte = prev_char_boundary(text, cursor_byte);

    let target_idx = delimiters
        .iter()
        .position(|d| match *d {
            TokenDelimiter::Open(_, b) | TokenDelimiter::Close(_, b) => b == cursor_byte,
        })
        .or_else(|| {
            prev_byte.and_then(|pb| {
                delimiters.iter().position(|d| match *d {
                    TokenDelimiter::Open(_, b) | TokenDelimiter::Close(_, b) => b == pb,
                })
            })
        })?;

    match delimiters[target_idx] {
        TokenDelimiter::Open(open_ch, open_b) => {
            let expected_close = matching_close(open_ch);
            let mut depth = 0usize;
            for item in &delimiters[target_idx..] {
                match *item {
                    TokenDelimiter::Open(c, _) if c == open_ch => depth += 1,
                    TokenDelimiter::Close(c, close_b) if c == expected_close => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return Some(BraceMatch {
                                open_byte: open_b,
                                close_byte: close_b,
                                open_char: open_ch,
                                close_char: expected_close,
                            });
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        TokenDelimiter::Close(close_ch, close_b) => {
            let expected_open = matching_open(close_ch);
            let mut depth = 0usize;
            for item in delimiters[..=target_idx].iter().rev() {
                match *item {
                    TokenDelimiter::Close(c, _) if c == close_ch => depth += 1,
                    TokenDelimiter::Open(c, open_b) if c == expected_open => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return Some(BraceMatch {
                                open_byte: open_b,
                                close_byte: close_b,
                                open_char: expected_open,
                                close_char: close_ch,
                            });
                        }
                    }
                    _ => {}
                }
            }
            None
        }
    }
}

fn matching_close(open: char) -> char {
    match open {
        '{' => '}',
        '(' => ')',
        '[' => ']',
        _ => open,
    }
}

fn matching_open(close: char) -> char {
    match close {
        '}' => '{',
        ')' => '(',
        ']' => '[',
        _ => close,
    }
}

pub fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn prev_char_boundary(text: &str, byte_idx: usize) -> Option<usize> {
    if byte_idx == 0 || byte_idx > text.len() {
        return None;
    }
    let mut b = byte_idx - 1;
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    Some(b)
}

/// First-principles lexical scan that strips out comments, strings, and char literals,
/// emitting only structural `{`, `}`, `(`, `)`, `[`, `]`.
fn extract_structural_delimiters(text: &str) -> Vec<TokenDelimiter> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // 1. Line comment `//`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // 2. Block comment `/* ... */` (supports nested block comments in Rust!)
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < len && depth > 0 {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // 3. String literal `"..."`
        if b == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // 4. Character literal `'x'` or `'\\'` vs lifetime `'a`
        if b == b'\'' {
            if i + 2 < len && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                i += 3;
                continue;
            } else if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                i += 4;
                continue;
            }
        }

        // 5. Structural delimiters
        match b {
            b'{' | b'(' | b'[' => out.push(TokenDelimiter::Open(b as char, i)),
            b'}' | b')' | b']' => out.push(TokenDelimiter::Close(b as char, i)),
            _ => {}
        }

        i += 1;
    }

    out
}

/// Helper to convert byte offset to 1-based (line, column)
pub fn byte_to_line_col(text: &str, byte_offset: usize) -> (usize, usize) {
    let clamped = byte_offset.min(text.len());
    let slice = &text[..clamped];
    let mut line = 1usize;
    let mut col = 1usize;
    for ch in slice.chars() {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
