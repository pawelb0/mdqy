//! mdqy: jq for markdown.
//!
//! Library first. The CLI in `src/bin/mdqy.rs` is a thin wrapper over
//! [`run_cli`].
//!
//! Typical flow:
//! ```no_run
//! use mdqy::Query;
//! let q = Query::compile("headings | .text").unwrap();
//! let tree = mdqy::parse("# Hi\n\n# There\n");
//! for v in q.run_tree(&tree) {
//!     println!("{v:?}");
//! }
//! ```

#![forbid(unsafe_code)]

pub mod ast;
pub mod emit;
pub mod error;
pub mod value;

pub(crate) mod aggregate;
pub(crate) mod analyze;
pub(crate) mod builtins;
pub(crate) mod cli;
pub(crate) mod eval;
pub(crate) mod events;
pub(crate) mod expr;
pub(crate) mod lex;
pub(crate) mod mutate;
pub(crate) mod parse;
pub(crate) mod stream;
pub(crate) mod walk;

pub use ast::{Node, NodeKind, Span};
pub use cli::{cli_command, run as run_cli};
pub use emit::OutputFormat;
pub use error::{CompileError, RunError};
pub use eval::Env;
pub use value::Value;

use pulldown_cmark::Event;

/// Compiled query. Lex, parse, and dispatch-mode selection all run
/// once in [`Query::compile`]; runners only walk the cached AST.
#[derive(Debug, Clone)]
pub struct Query {
    pub(crate) expr: expr::Expr,
    pub(crate) mode: analyze::Mode,
}

impl Query {
    /// Compile `source` into a `Query`.
    pub fn compile(source: &str) -> Result<Self, CompileError> {
        let tokens = lex::tokenize(source)?;
        let expr = parse::parse(&tokens)?;
        let mode = analyze::choose_mode(&expr);
        Ok(Self { expr, mode })
    }

    /// Run over an event iterator. Picks stream mode when the query
    /// qualifies, otherwise builds the tree and runs the interpreter.
    pub fn run<'a, I>(&self, events: I) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'a>
    where
        I: Iterator<Item = Event<'a>> + 'a,
    {
        match self.mode {
            analyze::Mode::Stream => stream::run(self.expr.clone(), events),
            analyze::Mode::Tree => self.run_value(Value::from(events::build_tree(events))),
        }
    }

    /// Run with pre-populated variable bindings. Used by the CLI to
    /// carry `--arg` / `--argjson` into evaluation.
    pub fn run_with_env(
        &self,
        input: Value,
        env: Env,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'static> {
        eval::eval(&self.expr, input, &env)
    }

    /// Run over an already-built tree. Skips stream dispatch.
    pub fn run_tree<'a>(
        &'a self,
        root: &'a Node,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'a> {
        self.run_value(Value::from(root.clone()))
    }

    /// Run over any `Value`. Used by `--slurp`, which binds `.` to an
    /// array of root nodes.
    pub fn run_value(
        &self,
        input: Value,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'static> {
        self.run_with_env(input, Env::default())
    }

    /// Transform markdown bytes. Parses, applies `|=` / `del(...)`,
    /// serialises back. Clean subtrees copy verbatim; only touched
    /// spans regenerate.
    pub fn transform_bytes(&self, source: &[u8]) -> Result<Vec<u8>, RunError> {
        mutate::transform_bytes(&self.expr, source)
    }

    /// `true` if the query has no `|=` / `del`.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        !analyze::has_mutation(&self.expr)
    }

    /// Name of the dispatch mode the compiler picked: `"stream"` or
    /// `"tree"`. Useful for `--explain-mode` and for tests.
    #[must_use]
    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            analyze::Mode::Stream => "stream",
            analyze::Mode::Tree => "tree",
        }
    }
}

/// Parse markdown into a `Node` tree.
#[must_use]
pub fn parse(source: &str) -> Node {
    events::build_tree_from_source(source)
}

/// The `pulldown_cmark::Options` set mdqy uses. Matches
/// `mdcat::markdown_options` under the `tty` feature so rendering
/// agrees with querying.
#[must_use]
pub fn markdown_options() -> pulldown_cmark::Options {
    events::options()
}
