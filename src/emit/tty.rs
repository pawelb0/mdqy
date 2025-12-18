//! Render query results to a TTY through `mdcat::push_tty`.
//! Feature-gated behind `tty`.
//!
//! Node results turn into a synthesised event stream and go straight
//! into `push_tty`; no markdown serialisation, no re-parse. Non-Node
//! values fall back to one line of text each, since mdcat speaks
//! markdown and doesn't know about arbitrary jq values.

use std::io;

use mdcat::resources::NoopResourceHandler;
use mdcat::{push_tty, Environment, Multiplexer, Settings, TerminalProgram, TerminalSize, Theme};
use syntect::parsing::SyntaxSet;

use crate::error::RunError;
use crate::events::node_to_events_owned;
use crate::value::Value;

/// Render a stream of Values to `writer`.
pub fn emit<W: io::Write>(
    writer: &mut W,
    values: impl IntoIterator<Item = Value>,
) -> Result<(), RunError> {
    let (events, scalars) = split_events_and_scalars(values);
    if !events.is_empty() {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let settings = Settings {
            terminal_capabilities: TerminalProgram::detect().capabilities(),
            terminal_size: TerminalSize::detect().unwrap_or_default(),
            multiplexer: Multiplexer::detect(),
            syntax_set: &syntax_set,
            theme: Theme::default(),
            wrap_code: true,
        };
        let cwd = std::env::current_dir().map_err(|e| RunError::Io(e.to_string()))?;
        let env = Environment::for_local_directory(&cwd).map_err(|e| RunError::Io(e.to_string()))?;
        push_tty(&settings, &env, &NoopResourceHandler, writer, events.into_iter())
            .map_err(|e| RunError::Other(format!("push_tty: {e}")))?;
    }
    for v in scalars {
        writer.write_all(value_to_line(&v).as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn split_events_and_scalars(
    values: impl IntoIterator<Item = Value>,
) -> (Vec<pulldown_cmark::Event<'static>>, Vec<Value>) {
    let mut events = Vec::new();
    let mut scalars = Vec::new();
    for v in values {
        if let Value::Node(n) = v {
            events.extend(node_to_events_owned(&n));
        } else {
            scalars.push(v);
        }
    }
    (events, scalars)
}

fn value_to_line(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if n.fract() == 0.0 && n.is_finite() => format!("{}", *n as i64),
        Value::Number(n) => n.to_string(),
        _ => {
            let json = crate::emit::json::value_to_json(
                v,
                crate::emit::json::JsonOptions::COMPACT,
            );
            serde_json::to_string(&json).unwrap_or_default()
        }
    }
}
