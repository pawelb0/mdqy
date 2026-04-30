//! mdqy: jq for markdown.
//!
//! Library first; the CLI in `src/bin/mdqy.rs` wraps [`run_cli`].

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

/// Compiled query. Lex, parse, and dispatch-mode selection happen in
/// [`Query::compile`]; runners only walk the cached AST.
#[derive(Debug, Clone)]
pub struct Query {
    pub(crate) expr: expr::Expr,
    pub(crate) mode: analyze::Mode,
}

impl Query {
    pub fn compile(source: &str) -> Result<Self, CompileError> {
        let tokens = lex::tokenize(source)?;
        let expr = parse::parse(&tokens)?;
        let mode = analyze::choose_mode(&expr);
        Ok(Self { expr, mode })
    }

    pub fn run<'a, I>(&self, events: I) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'a>
    where
        I: Iterator<Item = Event<'a>> + 'a,
    {
        match self.mode {
            analyze::Mode::Stream => stream::run(self.expr.clone(), events),
            analyze::Mode::Tree => self.run_value(Value::from(events::build_tree(events))),
        }
    }

    pub fn run_with_env(
        &self,
        input: Value,
        env: Env,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'static> {
        eval::eval(&self.expr, input, &env)
    }

    pub fn run_tree<'a>(
        &'a self,
        root: &'a Node,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'a> {
        self.run_value(Value::from(root.clone()))
    }

    pub fn run_value(
        &self,
        input: Value,
    ) -> Box<dyn Iterator<Item = Result<Value, RunError>> + 'static> {
        self.run_with_env(input, Env::default())
    }

    pub fn transform_bytes(&self, source: &[u8]) -> Result<Vec<u8>, RunError> {
        mutate::transform_bytes(&self.expr, source)
    }

    #[must_use]
    pub fn is_read_only(&self) -> bool {
        !analyze::has_mutation(&self.expr)
    }

    #[must_use]
    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            analyze::Mode::Stream => "stream",
            analyze::Mode::Tree => "tree",
        }
    }
}

#[must_use]
pub fn parse(source: &str) -> Node {
    events::build_tree_from_source(source)
}

#[must_use]
pub fn markdown_options() -> pulldown_cmark::Options {
    events::options()
}
