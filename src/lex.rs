//! Hand-rolled lexer. Tokens carry byte offsets for source-caret errors.

use std::borrow::Cow;

use crate::error::CompileError;

/// Token plus its starting byte offset in the source.
#[derive(Debug, Clone)]
pub struct Spanned<'a> {
    pub tok: Tok<'a>,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub enum Tok<'a> {
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
    Plus,
    Minus,
    Star,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    /// `#`..`######` for heading selector. Carries level 1..=6.
    Hash(u8),
    /// `:first`, `:last`, `:nth`, `:text`, `:lang`.
    ColonIdent(&'a str),
    Ident(&'a str),
    Str(Cow<'a, str>),
    Num(f64),
    DollarIdent(&'a str),
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

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

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

        if let Some(tok) = single_char_tok(c) {
            out.push(Spanned { tok, offset: start });
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
            let tok = if next.is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                i += 1;
                let id_start = i;
                while i < bytes.len() && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                Tok::ColonIdent(&source[id_start..i])
            } else {
                i += 1;
                Tok::Colon
            };
            out.push(Spanned { tok, offset: start });
            continue;
        }

        // `@format` filter. Emit as a regular Ident with the `@`
        // preserved so the builtin registry dispatches on the full name.
        if c == b'@' {
            i = scan_ident(bytes, i + 1);
            if i == start + 1 {
                return Err(CompileError::Lex {
                    offset: start,
                    message: "bare `@` with no identifier".into(),
                });
            }
            out.push(Spanned {
                tok: Tok::Ident(&source[start..i]),
                offset: start,
            });
            continue;
        }

        // Dollar-prefixed variable reference.
        if c == b'$' {
            i += 1;
            let id_start = i;
            i = scan_ident(bytes, i);
            if id_start == i {
                return Err(CompileError::Lex {
                    offset: start,
                    message: "bare `$` with no identifier".into(),
                });
            }
            out.push(Spanned {
                tok: Tok::DollarIdent(&source[id_start..i]),
                offset: start,
            });
            continue;
        }

        if c == b'"' {
            let (value, end) = lex_string(source, i)?;
            out.push(Spanned {
                tok: Tok::Str(value),
                offset: start,
            });
            i = end;
            continue;
        }

        if c.is_ascii_digit() {
            let (num, end) = lex_number(source, i)?;
            out.push(Spanned {
                tok: Tok::Num(num),
                offset: start,
            });
            i = end;
            continue;
        }

        if is_ident_start(c) {
            let id_start = i;
            i = scan_ident(bytes, i + 1);
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

fn scan_ident(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_ident_continue(bytes[i]) {
        i += 1;
    }
    i
}

fn single_char_tok(c: u8) -> Option<Tok<'static>> {
    Some(match c {
        b'.' => Tok::Dot,
        b'(' => Tok::LParen,
        b')' => Tok::RParen,
        b'[' => Tok::LBracket,
        b']' => Tok::RBracket,
        b'{' => Tok::LBrace,
        b'}' => Tok::RBrace,
        b',' => Tok::Comma,
        b';' => Tok::Semicolon,
        b'|' => Tok::Pipe,
        b'=' => Tok::Eq,
        b'/' => Tok::Slash,
        b'?' => Tok::Question,
        b'+' => Tok::Plus,
        b'-' => Tok::Minus,
        b'*' => Tok::Star,
        b'%' => Tok::Percent,
        b'<' => Tok::Lt,
        b'>' => Tok::Gt,
        _ => return None,
    })
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
    // Paren depth inside `\(...)`. Nonzero means `"` opens a nested
    // string instead of terminating this one.
    let mut depth = 0usize;

    while i < bytes.len() {
        if depth > 0 {
            match bytes[i] {
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b')' => {
                    depth -= 1;
                    i += 1;
                }
                b'"' => {
                    let (_inner, end) = lex_string(source, i)?;
                    i = end;
                }
                _ => i += 1,
            }
            continue;
        }
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
                    // `\(expr)` stays raw for the parser to re-tokenise.
                    b'(' => {
                        buf.push_str("\\(");
                        depth = 1;
                    }
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
