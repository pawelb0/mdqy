//! Hand-rolled lexer. One pass, one `Vec<Spanned>` out.
//!
//! Tokens carry source byte offsets so the parser can point errors at
//! the exact column. `Hash` and `ColonIdent` tokens are emitted for
//! selector shortcuts (`# Title`, `:first`); the parser decides how
//! to use them.

use std::borrow::Cow;

use crate::error::CompileError;

/// Token plus its starting byte offset in the source.
#[derive(Debug, Clone)]
pub struct Spanned<'a> {
    pub tok: Tok<'a>,
    pub offset: usize,
}

/// A single token.
#[derive(Debug, Clone)]
pub enum Tok<'a> {
    // Punctuation
    Dot,
    DotDot,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semicolon,
    Pipe,
    PipeEq,
    Eq,
    EqEq,
    NotEq,
    Slash,
    SlashSlash,
    Question,

    // Operators
    Plus,
    Minus,
    Star,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,

    /// `#`..`######`. The `u8` is the heading level (1..=6).
    #[allow(dead_code)]
    Hash(u8),
    /// `:first`, `:last`, `:nth`, `:text`, `:lang`. Ident after the colon.
    #[allow(dead_code)]
    ColonIdent(&'a str),

    // Literals & identifiers
    Ident(&'a str),
    Str(Cow<'a, str>),
    Num(f64),
    DollarIdent(&'a str),

    // Keywords
    KwIf,
    KwThen,
    KwElif,
    KwElse,
    KwEnd,
    KwAs,
    KwDef,
    KwTrue,
    KwFalse,
    KwNull,
    KwAnd,
    KwOr,
    KwNot,

    Eof,
}

/// Tokenise `source`. Output always ends with `Tok::Eof`.
pub fn tokenize(source: &str) -> Result<Vec<Spanned<'_>>, CompileError> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();

    while i < bytes.len() {
        let start = i;
        let c = bytes[i];

        // Skip whitespace.
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // `#` opens a heading selector. mdqy has no line comments.

        // Two-char punct first; otherwise a trailing `=` would get eaten.
        let two = [c, bytes.get(i + 1).copied().unwrap_or(0)];
        if let Some(tok) = match &two {
            b".." => Some(Tok::DotDot),
            b"|=" => Some(Tok::PipeEq),
            b"==" => Some(Tok::EqEq),
            b"!=" => Some(Tok::NotEq),
            b"<=" => Some(Tok::Le),
            b">=" => Some(Tok::Ge),
            b"//" => Some(Tok::SlashSlash),
            _ => None,
        } {
            out.push(Spanned { tok, offset: start });
            i += 2;
            continue;
        }

        // Single-char punctuation.
        let single = match c {
            b'.' => Some(Tok::Dot),
            b'(' => Some(Tok::LParen),
            b')' => Some(Tok::RParen),
            b'[' => Some(Tok::LBracket),
            b']' => Some(Tok::RBracket),
            b'{' => Some(Tok::LBrace),
            b'}' => Some(Tok::RBrace),
            b',' => Some(Tok::Comma),
            b';' => Some(Tok::Semicolon),
            b'|' => Some(Tok::Pipe),
            b'=' => Some(Tok::Eq),
            b'/' => Some(Tok::Slash),
            b'?' => Some(Tok::Question),
            b'+' => Some(Tok::Plus),
            b'-' => Some(Tok::Minus),
            b'*' => Some(Tok::Star),
            b'%' => Some(Tok::Percent),
            b'<' => Some(Tok::Lt),
            b'>' => Some(Tok::Gt),
            _ => None,
        };
        if let Some(t) = single {
            out.push(Spanned {
                tok: t,
                offset: start,
            });
            i += 1;
            continue;
        }

        // Heading selector `#`..`######`.
        if c == b'#' {
            let mut count = 0u8;
            while i < bytes.len() && bytes[i] == b'#' && count < 6 {
                i += 1;
                count += 1;
            }
            if i < bytes.len() && bytes[i] == b'#' {
                return Err(CompileError::Lex {
                    offset: start,
                    message: format!("too many `#` (max 6), got at least {}", count + 1),
                });
            }
            out.push(Spanned {
                tok: Tok::Hash(count),
                offset: start,
            });
            continue;
        }

        // Colon-prefixed pseudo: `:first`, or bare `:`.
        if c == b':' {
            let next = bytes.get(i + 1).copied();
            if next.is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                i += 1;
                let id_start = i;
                while i < bytes.len() && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                let name = &source[id_start..i];
                out.push(Spanned {
                    tok: Tok::ColonIdent(name),
                    offset: start,
                });
            } else {
                out.push(Spanned {
                    tok: Tok::Colon,
                    offset: start,
                });
                i += 1;
            }
            continue;
        }

        // Dollar-prefixed variable reference.
        if c == b'$' {
            i += 1;
            let id_start = i;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if id_start == i {
                return Err(CompileError::Lex {
                    offset: start,
                    message: "bare `$` with no identifier".into(),
                });
            }
            let name = &source[id_start..i];
            out.push(Spanned {
                tok: Tok::DollarIdent(name),
                offset: start,
            });
            continue;
        }

        // String literal.
        if c == b'"' {
            let (value, end) = lex_string(source, i)?;
            out.push(Spanned {
                tok: Tok::Str(value),
                offset: start,
            });
            i = end;
            continue;
        }

        // Number literal (simple floating-point).
        if c.is_ascii_digit() {
            let (num, end) = lex_number(source, i)?;
            out.push(Spanned {
                tok: Tok::Num(num),
                offset: start,
            });
            i = end;
            continue;
        }

        // Identifier / keyword.
        if is_ident_start(c) {
            let id_start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let name = &source[id_start..i];
            let tok = keyword_for(name).unwrap_or(Tok::Ident(name));
            out.push(Spanned { tok, offset: start });
            continue;
        }

        return Err(CompileError::Lex {
            offset: start,
            message: format!("unexpected character: {}", c as char),
        });
    }

    out.push(Spanned {
        tok: Tok::Eof,
        offset: source.len(),
    });
    Ok(out)
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn keyword_for(name: &str) -> Option<Tok<'_>> {
    Some(match name {
        "if" => Tok::KwIf,
        "then" => Tok::KwThen,
        "elif" => Tok::KwElif,
        "else" => Tok::KwElse,
        "end" => Tok::KwEnd,
        "as" => Tok::KwAs,
        "def" => Tok::KwDef,
        "true" => Tok::KwTrue,
        "false" => Tok::KwFalse,
        "null" => Tok::KwNull,
        "and" => Tok::KwAnd,
        "or" => Tok::KwOr,
        "not" => Tok::KwNot,
        _ => return None,
    })
}

fn lex_string(source: &str, start: usize) -> Result<(Cow<'_, str>, usize), CompileError> {
    let bytes = source.as_bytes();
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    let mut owned: Option<String> = None;
    let mut raw_start = i;

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let tail = &source[raw_start..i];
                let value = match owned {
                    Some(mut s) => {
                        s.push_str(tail);
                        Cow::Owned(s)
                    }
                    None => Cow::Borrowed(&source[start + 1..i]),
                };
                return Ok((value, i + 1));
            }
            b'\\' => {
                // Escape sequence. Flush any prior raw slice.
                let mut buf = owned.take().unwrap_or_default();
                buf.push_str(&source[raw_start..i]);
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'"' | b'\\' | b'/' => buf.push(bytes[i] as char),
                    b'n' => buf.push('\n'),
                    b't' => buf.push('\t'),
                    b'r' => buf.push('\r'),
                    b'0' => buf.push('\0'),
                    // `\(expr)` interpolation: pass through raw for now.
                    b'(' => buf.push_str("\\("),
                    other => {
                        return Err(CompileError::Lex {
                            offset: i - 1,
                            message: format!("unknown escape `\\{}`", other as char),
                        });
                    }
                }
                i += 1;
                raw_start = i;
                owned = Some(buf);
            }
            b'\n' => {
                return Err(CompileError::Lex {
                    offset: start,
                    message: "unterminated string literal (newline before close)".into(),
                });
            }
            _ => i += 1,
        }
    }
    Err(CompileError::Lex {
        offset: start,
        message: "unterminated string literal".into(),
    })
}

fn lex_number(source: &str, start: usize) -> Result<(f64, usize), CompileError> {
    let bytes = source.as_bytes();
    let mut i = start;
    let consume_digits = |bytes: &[u8], i: &mut usize| {
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
    };
    consume_digits(bytes, &mut i);
    if i < bytes.len() && bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        consume_digits(bytes, &mut i);
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        consume_digits(bytes, &mut i);
    }
    let text = &source[start..i];
    let value: f64 = text.parse().map_err(|_| CompileError::Lex {
        offset: start,
        message: format!("invalid number literal `{text}`"),
    })?;
    Ok((value, i))
}

#[cfg(test)]
mod tests {
    //! Lexer behavior is exercised transitively by `tests/queries.rs`
    //! (every query compiled there passes through this module). The
    //! only error we want to pin down here is unterminated strings,
    //! because compile-error surface is part of the public contract.

    use super::*;

    #[test]
    fn unterminated_string_is_an_error() {
        assert!(tokenize(r#""oops"#).is_err());
        assert!(tokenize("\"line\nwrap\"").is_err());
    }
}
