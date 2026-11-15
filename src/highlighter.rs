use crate::brace_matcher::BraceMatch;
use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Color32, FontId, TextFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    TypeKeyword,
    StringLiteral,
    Comment,
    Number,
    MacroOrAttribute,
    Lifetime,
    OperatorOrPunct,
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32",
    "u64", "u128", "usize", "str", "String", "Vec", "Option", "Result", "Some", "None", "Ok",
    "Err", "Box", "Rc", "Arc", "HashMap", "HashSet", "Path", "PathBuf",
];

/// Builds an egui `LayoutJob` with syntax highlighting and optional matched-brace highlight boxes.
pub fn highlight_rust_code(
    text: &str,
    font_id: FontId,
    brace_match: Option<BraceMatch>,
    dark_mode: bool,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    if text.is_empty() {
        return job;
    }

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let start = i;
        let b = bytes[i];

        // 1. Line Comment `//...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            append_span(
                &mut job,
                text,
                start,
                i,
                TokenKind::Comment,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 2. Block Comment `/* ... */`
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
            append_span(
                &mut job,
                text,
                start,
                i,
                TokenKind::Comment,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 3. Attribute `#[...]` or `#![...]`
        if b == b'#'
            && i + 1 < len
            && (bytes[i + 1] == b'['
                || (bytes[i + 1] == b'!' && i + 2 < len && bytes[i + 2] == b'['))
        {
            while i < len && bytes[i] != b']' && bytes[i] != b'\n' {
                i += 1;
            }
            if i < len && bytes[i] == b']' {
                i += 1;
            }
            append_span(
                &mut job,
                text,
                start,
                i,
                TokenKind::MacroOrAttribute,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 4. String Literal `"..."`
        if b == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            append_span(
                &mut job,
                text,
                start,
                i,
                TokenKind::StringLiteral,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 5. Char literal or Lifetime `'a`
        if b == b'\'' {
            if i + 2 < len && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                i += 3;
                append_span(
                    &mut job,
                    text,
                    start,
                    i,
                    TokenKind::StringLiteral,
                    &font_id,
                    brace_match,
                    dark_mode,
                );
                continue;
            } else if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                i += 4;
                append_span(
                    &mut job,
                    text,
                    start,
                    i,
                    TokenKind::StringLiteral,
                    &font_id,
                    brace_match,
                    dark_mode,
                );
                continue;
            } else if i + 1 < len && is_ident_start(bytes[i + 1]) {
                i += 2;
                while i < len && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                append_span(
                    &mut job,
                    text,
                    start,
                    i,
                    TokenKind::Lifetime,
                    &font_id,
                    brace_match,
                    dark_mode,
                );
                continue;
            }
        }

        // 6. Numeric Literal
        if b.is_ascii_digit() {
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            append_span(
                &mut job,
                text,
                start,
                i,
                TokenKind::Number,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 7. Identifier, Keyword, Type, or Macro (`println!`)
        if is_ident_start(b) {
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            let kind = if i < len && bytes[i] == b'!' {
                i += 1; // Include the `!` in macro invocation
                TokenKind::MacroOrAttribute
            } else if RUST_KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if RUST_TYPES.contains(&word)
                || word
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
            {
                TokenKind::TypeKeyword
            } else {
                TokenKind::Plain
            };
            append_span(
                &mut job,
                text,
                start,
                i,
                kind,
                &font_id,
                brace_match,
                dark_mode,
            );
            continue;
        }

        // 8. Single-byte punctuation / whitespace (handle brace matching per character!)
        let ch_len = text[start..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        i = start + ch_len;
        let kind = if b.is_ascii_punctuation() {
            TokenKind::OperatorOrPunct
        } else {
            TokenKind::Plain
        };
        append_span(
            &mut job,
            text,
            start,
            i,
            kind,
            &font_id,
            brace_match,
            dark_mode,
        );
    }

    job
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn append_span(
    job: &mut LayoutJob,
    text: &str,
    start: usize,
    end: usize,
    kind: TokenKind,
    font_id: &FontId,
    brace_match: Option<BraceMatch>,
    dark_mode: bool,
) {
    let fg = token_color(kind, dark_mode);

    // Check if this span is a single character that matches an active brace pair
    let is_matched_brace = brace_match
        .map(|bm| (start == bm.open_byte || start == bm.close_byte) && (end - start == 1))
        .unwrap_or(false);

    let mut format = TextFormat {
        font_id: font_id.clone(),
        color: if is_matched_brace {
            if dark_mode {
                Color32::from_rgb(255, 235, 59)
            } else {
                Color32::from_rgb(180, 0, 0)
            }
        } else {
            fg
        },
        ..Default::default()
    };

    if is_matched_brace {
        format.background = if dark_mode {
            Color32::from_rgba_unmultiplied(90, 80, 20, 200)
        } else {
            Color32::from_rgba_unmultiplied(255, 235, 130, 220)
        };
        format.underline = egui::Stroke::new(1.5_f32, Color32::from_rgb(255, 180, 0));
    }

    job.append(&text[start..end], 0.0, format);
}

fn token_color(kind: TokenKind, dark_mode: bool) -> Color32 {
    if dark_mode {
        match kind {
            TokenKind::Plain => Color32::from_rgb(220, 223, 228),
            TokenKind::Keyword => Color32::from_rgb(255, 123, 114), // Coral red/orange
            TokenKind::TypeKeyword => Color32::from_rgb(121, 192, 255), // Crisp cyan/blue
            TokenKind::StringLiteral => Color32::from_rgb(165, 214, 255), // Soft sky blue / green
            TokenKind::Comment => Color32::from_rgb(110, 130, 110), // Muted sage green
            TokenKind::Number => Color32::from_rgb(210, 168, 255),  // Lavender
            TokenKind::MacroOrAttribute => Color32::from_rgb(255, 166, 87), // Amber
            TokenKind::Lifetime => Color32::from_rgb(230, 192, 123), // Warm gold
            TokenKind::OperatorOrPunct => Color32::from_rgb(190, 195, 205),
        }
    } else {
        // PSPad classic light-theme inspired palette
        match kind {
            TokenKind::Plain => Color32::from_rgb(20, 20, 25),
            TokenKind::Keyword => Color32::from_rgb(0, 0, 180), // Classic Navy Blue
            TokenKind::TypeKeyword => Color32::from_rgb(0, 110, 130), // Teal
            TokenKind::StringLiteral => Color32::from_rgb(163, 21, 21), // Crimson
            TokenKind::Comment => Color32::from_rgb(0, 128, 0), // Classic Forest Green
            TokenKind::Number => Color32::from_rgb(150, 0, 150), // Purple
            TokenKind::MacroOrAttribute => Color32::from_rgb(180, 80, 0), // Rust Orange
            TokenKind::Lifetime => Color32::from_rgb(140, 90, 0),
            TokenKind::OperatorOrPunct => Color32::from_rgb(60, 60, 70),
        }
    }
}
