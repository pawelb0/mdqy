//! Output paths: JSON, markdown, tty. Each path is a separate submodule.

pub mod json;
pub mod md;

#[cfg(feature = "tty")]
pub mod tty;

/// Output format. Also the `--output` CLI enum.
///
/// `Auto` is the default. The CLI resolves it per invocation: when
/// stdout is a terminal and the `tty` feature is compiled in, it
/// renders through mdcat; otherwise it emits markdown. This keeps
/// interactive use nice while leaving scripted pipes untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pick between `md` and `tty` based on stdout.
    #[default]
    Auto,
    /// Markdown. Non-Node results fall back to JSON since they have
    /// no markdown form.
    Md,
    /// JSON. One result per line.
    Json,
    /// Render through mdcat. Requires the `tty` cargo feature.
    Tty,
    /// Plain text. Only useful for string results.
    Text,
}
