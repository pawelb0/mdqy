use thiserror::Error;

/// Something went wrong turning source text into a [`Query`](crate::Query).
#[derive(Debug, Clone, Error)]
pub enum CompileError {
    #[error("lex error at offset {offset}: {message}")]
    Lex { offset: usize, message: String },
    #[error("parse error at offset {offset}: expected {expected}, found {found}")]
    Parse {
        offset: usize,
        expected: String,
        found: String,
    },
    #[error("selector error at offset {offset}: {message}")]
    Selector { offset: usize, message: String },
    #[error("unknown builtin: {name}")]
    UnknownBuiltin { name: String },
}

/// Something went wrong running a compiled query.
#[derive(Debug, Clone, Error)]
pub enum RunError {
    #[error("type error: expected {expected}, got {got}")]
    Type { expected: String, got: String },
    #[error("index out of range: {index}")]
    Index { index: i64 },
    #[error("regex error: {0}")]
    Regex(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("not implemented: {feature}")]
    NotImplemented { feature: &'static str },
    #[error("runtime error: {0}")]
    Other(String),
}

impl CompileError {
    /// Byte offset into the expression source where the error happened.
    #[must_use]
    pub fn offset(&self) -> usize {
        match self {
            Self::Lex { offset, .. }
            | Self::Parse { offset, .. }
            | Self::Selector { offset, .. } => *offset,
            Self::UnknownBuiltin { .. } => 0,
        }
    }

    /// Human-readable multi-line rendering that underlines the offending
    /// column. `expr` is the original source passed to `compile`.
    #[must_use]
    pub fn render(&self, expr: &str) -> String {
        let offset = self.offset().min(expr.len());
        let (line_no, col, line_text) = locate(expr, offset);
        let gutter = format!("{line_no:>4}");
        let blank = " ".repeat(gutter.len());
        let caret = " ".repeat(col) + "^";
        format!("error: {self}\n {blank} |\n {gutter} | {line_text}\n {blank} | {caret}")
    }
}

fn locate(source: &str, offset: usize) -> (usize, usize, &str) {
    let (mut line, mut line_start) = (1usize, 0usize);
    for (i, c) in source.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |n| line_start + n);
    (line, offset - line_start, &source[line_start..line_end])
}

impl From<std::io::Error> for RunError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<regex::Error> for RunError {
    fn from(e: regex::Error) -> Self {
        Self::Regex(e.to_string())
    }
}
