//! Argument parsing and the `mdqy` binary entry point.

use std::fs;
use std::io;
use std::io::Write as _;
#[cfg(feature = "tty")]
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::Parser;
use tempfile::NamedTempFile;

use crate::aggregate::Aggregation;
use crate::emit::json::{self, JsonOptions};
use crate::emit::OutputFormat;
use crate::value::Value;
use crate::walk::{walk_inputs, WalkOptions};

/// mdqy: jq for markdown.
#[derive(Debug, Parser)]
#[command(name = "mdqy", version, about = "jq for markdown: query and transform Markdown")]
#[allow(clippy::struct_excessive_bools)] // CLI flags map 1:1 to bools.
pub struct Args {
    /// mdqy expression (required unless --from-file is set).
    #[arg(required_unless_present = "from_file")]
    pub expr: Option<String>,

    /// Input markdown files or directories.
    pub paths: Vec<PathBuf>,

    /// Start evaluation with `null` instead of reading input.
    #[arg(short = 'n', long = "null-input")]
    pub null_input: bool,

    /// Read input as a raw string rather than parsing as markdown.
    #[arg(short = 'R', long = "raw-input")]
    pub raw_input: bool,

    /// Collect every input document into a single array bound to `.`.
    #[arg(short = 's', long)]
    pub slurp: bool,

    /// Concatenate event streams into one virtual document.
    #[arg(long, conflicts_with = "slurp")]
    pub merge: bool,

    /// Run the query once per input file (default).
    #[arg(long = "per-file", conflicts_with_all = ["slurp", "merge"])]
    pub per_file: bool,

    /// Walk directories recursively (implied when any path is a directory).
    #[arg(short = 'r', long)]
    pub recursive: bool,

    /// Follow symlinks during directory walk.
    #[arg(long)]
    pub follow: bool,

    /// Include hidden files in the walk.
    #[arg(long)]
    pub hidden: bool,

    /// Do not honor `.gitignore` or `.ignore` during the walk.
    #[arg(long = "no-ignore")]
    pub no_ignore: bool,

    /// Force reading from stdin even when PATHS are set.
    #[arg(long)]
    pub stdin: bool,

    /// Bind `$NAME` to a string value (jq-compatible).
    #[arg(long = "arg", num_args = 2, value_names = ["NAME", "VALUE"])]
    pub arg: Vec<String>,

    /// Bind `$NAME` to a JSON value (jq-compatible).
    #[arg(long = "argjson", num_args = 2, value_names = ["NAME", "JSON"])]
    pub argjson: Vec<String>,

    /// Output format. `auto` renders to TTY when stdout is a
    /// terminal and the `tty` feature is compiled in; otherwise
    /// emits markdown.
    #[arg(short = 'o', long, value_enum, default_value_t = OutputFormat::Auto)]
    pub output: OutputFormat,

    /// Emit strings without JSON quoting.
    #[arg(long)]
    pub raw: bool,

    /// Single-line JSON per result.
    #[arg(short = 'c', long)]
    pub compact: bool,

    /// Tag each JSON result with its source path.
    #[arg(long = "with-path")]
    pub with_path: bool,

    /// Include span information on every Node in JSON output.
    #[arg(long = "with-spans")]
    pub with_spans: bool,

    /// Disable colour output.
    #[arg(long = "no-color")]
    pub no_color: bool,

    /// Atomically overwrite each input file with the transform result.
    #[arg(short = 'U', long = "in-place")]
    pub in_place: bool,

    /// Print a unified diff instead of writing (implies --in-place).
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Before an --in-place overwrite, copy `foo.md` to `foo.md.EXT`.
    #[arg(long, value_name = "EXT")]
    pub backup: Option<String>,

    /// Read expression from FILE instead of the positional argument.
    #[arg(short = 'f', long = "from-file", value_name = "FILE")]
    pub from_file: Option<PathBuf>,

    /// Compile the expression and exit. No input is read.
    #[arg(short = 'p', long = "compile-only")]
    pub compile_only: bool,

    /// Number of worker threads for per-file query dispatch. `0`
    /// picks a thread per CPU. `1` (default) runs sequentially.
    #[arg(long, default_value_t = 1)]
    pub workers: usize,

    /// Re-run the query whenever the input file changes. Requires
    /// the `watch` cargo feature and a single file path.
    #[arg(long)]
    pub watch: bool,

    /// Print the dispatch mode (`stream` or `tree`) the compiler
    /// picked and exit.
    #[arg(long = "explain-mode")]
    pub explain_mode: bool,
}

/// The built `clap::Command` for the `mdqy` binary. Exposed so
/// helper binaries (completion / man-page generators) can drive the
/// same parser without forking its spec.
#[must_use]
pub fn cli_command() -> clap::Command {
    <Args as clap::CommandFactory>::command()
}

/// Main entry called from `src/bin/mdqy.rs`.
pub fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.no_color {
        // Honoured by `mdcat` under the `tty` feature and by any
        // ANSI-aware crate downstream that reads `NO_COLOR`.
        std::env::set_var("NO_COLOR", "1");
    }

    let expression = if let Some(path) = &args.from_file {
        fs::read_to_string(path)?
    } else {
        args.expr.clone().unwrap_or_default()
    };

    let format = resolve_format(args.output);
    let aggregation = if args.slurp {
        Aggregation::Slurp
    } else if args.merge {
        Aggregation::Merge
    } else {
        Aggregation::PerFile
    };

    let trimmed = expression.trim();
    let query = crate::Query::compile(trimmed)
        .map_err(|e| anyhow::anyhow!("{}", e.render(trimmed)))?;

    if args.explain_mode {
        println!("mode: {}", query.mode_name());
        return Ok(());
    }
    if args.compile_only {
        return Ok(());
    }

    let env = build_env(&args)?;

    if args.watch {
        return run_watch(&query, &args, env, format);
    }

    let walk_opts = WalkOptions {
        follow_symlinks: args.follow,
        include_hidden: args.hidden,
        no_ignore: args.no_ignore,
    };

    let inputs: Vec<PathBuf> = if args.stdin || args.null_input {
        Vec::new()
    } else {
        walk_inputs(&args.paths, walk_opts).collect::<Result<Vec<_>, _>>()?
    };

    let mut stdout = io::BufWriter::new(io::stdout().lock());

    if args.null_input {
        return emit_stream(query.run_with_env(Value::Null, env), "", None, format, &args, &mut stdout);
    }
    if args.stdin {
        let mut buf = String::new();
        io::Read::read_to_string(&mut io::stdin(), &mut buf)?;
        if !query.is_read_only() && !args.raw_input {
            let new_bytes = query.transform_bytes(buf.as_bytes())
                .map_err(|e| anyhow::anyhow!("transform: {e}"))?;
            stdout.write_all(&new_bytes)?;
            return Ok(());
        }
        let input = if args.raw_input { Value::from(buf.clone()) } else { Value::from(crate::events::build_tree_from_source(&buf)) };
        return emit_stream(query.run_with_env(input, env), &buf, None, format, &args, &mut stdout);
    }

    if args.in_place || args.dry_run || !query.is_read_only() {
        for path in &inputs {
            run_transform(&query, path, &args, &mut stdout)?;
        }
        return Ok(());
    }

    match aggregation {
        _ if args.raw_input => {
            for path in &inputs {
                let source = fs::read_to_string(path)?;
                emit_stream(query.run_with_env(Value::from(source.clone()), env.clone()), &source, Some(path), format, &args, &mut stdout)?;
            }
        }
        Aggregation::PerFile if args.workers != 1 && !matches!(format, OutputFormat::Tty) => {
            run_per_file_parallel(&query, &inputs, &env, format, &args, &mut stdout)?;
        }
        Aggregation::PerFile => {
            for path in &inputs {
                let source = fs::read_to_string(path)?;
                let root = crate::events::build_tree_from_source(&source);
                emit_stream(query.run_with_env(Value::from(root), env.clone()), &source, Some(path), format, &args, &mut stdout)?;
            }
        }
        Aggregation::Slurp => {
            let input = Value::Array(std::sync::Arc::new(read_all_roots(&inputs)?));
            emit_stream(query.run_with_env(input, env), "", None, format, &args, &mut stdout)?;
        }
        Aggregation::Merge => {
            let mut virt = crate::ast::Node::new(crate::ast::NodeKind::Root);
            for path in &inputs {
                let source = fs::read_to_string(path)?;
                virt.children.extend(crate::events::build_tree_from_source(&source).children);
            }
            emit_stream(query.run_with_env(Value::from(virt), env), "", None, format, &args, &mut stdout)?;
        }
    }
    Ok(())
}

/// Collect `--arg` / `--argjson` into an `Env`.
fn build_env(args: &Args) -> anyhow::Result<crate::Env> {
    let mut env = crate::Env::default();
    for pair in args.arg.chunks_exact(2) {
        env = env.with(pair[0].clone(), Value::from(pair[1].clone()));
    }
    for pair in args.argjson.chunks_exact(2) {
        let json: serde_json::Value = serde_json::from_str(&pair[1])
            .map_err(|e| anyhow::anyhow!("--argjson {}: {e}", pair[0]))?;
        env = env.with(pair[0].clone(), json::value_from_json(json));
    }
    Ok(env)
}

/// Drive the result iterator into the writer. `path` is set when the
/// stream came from a specific file; tagged onto JSON output under
/// `--with-path`.
fn emit_stream<W: io::Write>(
    stream: Box<dyn Iterator<Item = Result<Value, crate::RunError>>>,
    source: &str,
    path: Option<&Path>,
    format: OutputFormat,
    args: &Args,
    out: &mut W,
) -> anyhow::Result<()> {
    for r in stream {
        let value = r.map_err(|e| match path {
            Some(p) => anyhow::anyhow!("runtime error in {}: {e}", p.display()),
            None => anyhow::anyhow!("runtime error: {e}"),
        })?;
        let tagged;
        let out_value = if args.with_path && matches!(format, OutputFormat::Json) {
            tagged = tag_with_path(&value, path);
            &tagged
        } else {
            &value
        };
        emit_value(out, out_value, format, args, source)?;
    }
    Ok(())
}

/// Wrap a value as `{ "path": "...", "value": <value> }` so
/// `--with-path --output json` still emits one object per result.
fn tag_with_path(value: &Value, path: Option<&Path>) -> Value {
    use std::collections::BTreeMap;
    let path_str = path.map(|p| p.display().to_string()).unwrap_or_default();
    let obj: BTreeMap<String, Value> = [("path".into(), Value::from(path_str)), ("value".into(), value.clone())].into();
    Value::Object(std::sync::Arc::new(obj))
}

/// Parallel per-file query dispatch. Each worker parses, runs, and
/// serialises results into a per-file buffer. Buffers flush in input
/// order so output matches the serial path.
fn run_per_file_parallel(
    query: &crate::Query,
    inputs: &[PathBuf],
    env: &crate::Env,
    format: OutputFormat,
    args: &Args,
    stdout: &mut io::BufWriter<io::StdoutLock<'_>>,
) -> anyhow::Result<()> {
    use rayon::prelude::*;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.workers)
        .build()
        .map_err(|e| anyhow::anyhow!("thread pool: {e}"))?;
    let bufs: Vec<anyhow::Result<Vec<u8>>> = pool.install(|| {
        inputs.par_iter().map(|path| {
            let source = fs::read_to_string(path)?;
            let root = crate::events::build_tree_from_source(&source);
            let mut buf: Vec<u8> = Vec::new();
            emit_stream(query.run_with_env(Value::from(root), env.clone()), &source, Some(path), format, args, &mut buf)?;
            Ok(buf)
        }).collect()
    });
    for buf in bufs {
        stdout.write_all(&buf?)?;
    }
    Ok(())
}

#[cfg(feature = "watch")]
fn run_watch(
    query: &crate::Query,
    args: &Args,
    env: crate::Env,
    format: OutputFormat,
) -> anyhow::Result<()> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    if args.paths.len() != 1 || !args.paths[0].is_file() {
        anyhow::bail!("--watch expects exactly one file path");
    }
    let path = args.paths[0].clone();

    let render = || -> anyhow::Result<()> {
        let source = fs::read_to_string(&path)?;
        let root = crate::events::build_tree_from_source(&source);
        let mut stdout = io::BufWriter::new(io::stdout().lock());
        write!(stdout, "\x1b[2J\x1b[H")?;
        emit_stream(
            query.run_with_env(Value::from(root), env.clone()),
            &source,
            Some(&path),
            format,
            args,
            &mut stdout,
        )
    };

    render()?;

    let (tx, rx) = channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(&path, RecursiveMode::NonRecursive)?;

    let debounce = Duration::from_millis(150);
    loop {
        match rx.recv() {
            Ok(Ok(ev)) if matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) => {
                loop {
                    match rx.recv_timeout(debounce) {
                        Ok(_) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                if let Err(e) = render() {
                    eprintln!("mdqy: {e}");
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("mdqy: watch error: {e}"),
            Err(_) => return Ok(()),
        }
    }
}

#[cfg(not(feature = "watch"))]
fn run_watch(
    _query: &crate::Query,
    _args: &Args,
    _env: crate::Env,
    _format: OutputFormat,
) -> anyhow::Result<()> {
    anyhow::bail!("--watch requires the `watch` cargo feature")
}

fn read_all_roots(inputs: &[PathBuf]) -> anyhow::Result<Vec<Value>> {
    inputs
        .iter()
        .map(|p| fs::read_to_string(p).map(|s| Value::from(crate::events::build_tree_from_source(&s))))
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

fn run_transform(
    query: &crate::Query,
    path: &Path,
    args: &Args,
    stdout: &mut impl io::Write,
) -> anyhow::Result<()> {
    let source = fs::read(path)?;
    let new_bytes = query.transform_bytes(&source).map_err(|e| anyhow::anyhow!("transform: {e}"))?;
    if args.dry_run {
        let old = String::from_utf8_lossy(&source);
        let new = String::from_utf8_lossy(&new_bytes);
        let diff = similar::TextDiff::from_lines(old.as_ref(), new.as_ref());
        write!(stdout, "--- {p}\n+++ {p}\n", p = path.display())?;
        for change in diff.iter_all_changes() {
            let tag = match change.tag() {
                similar::ChangeTag::Delete => "-",
                similar::ChangeTag::Insert => "+",
                similar::ChangeTag::Equal => " ",
            };
            write!(stdout, "{tag}{}", change.value())?;
        }
    } else if args.in_place {
        apply_in_place(path, &new_bytes, args.backup.as_deref())?;
    } else {
        stdout.write_all(&new_bytes)?;
    }
    Ok(())
}


fn emit_value<W: io::Write>(
    out: &mut W,
    value: &Value,
    format: OutputFormat,
    args: &Args,
    source: &str,
) -> anyhow::Result<()> {
    let emit_json = |out: &mut W, compact: bool| -> anyhow::Result<()> {
        json::emit(out, value, JsonOptions { compact, include_spans: args.with_spans })?;
        Ok(())
    };
    let emit_line = |out: &mut W, s: &str| -> anyhow::Result<()> {
        out.write_all(s.as_bytes())?;
        out.write_all(b"\n")?;
        Ok(())
    };
    match (format, value) {
        // `Auto` is resolved before emit_value runs. It still needs a
        // branch to keep the match exhaustive, so pair it with Md.
        (OutputFormat::Auto | OutputFormat::Md, Value::Node(n)) => {
            crate::emit::md::serialize(out, source.as_bytes(), n)
                .map_err(|e| anyhow::anyhow!("md emit: {e}"))?;
        }
        (OutputFormat::Auto | OutputFormat::Md, Value::String(s)) if args.raw => emit_line(out, s)?,
        (OutputFormat::Auto | OutputFormat::Md | OutputFormat::Json, _) => {
            emit_json(out, args.compact)?;
        }
        (OutputFormat::Text, Value::String(s)) => emit_line(out, s)?,
        (OutputFormat::Text, Value::Node(n)) => emit_line(out, &crate::events::plain_text(&n.children))?,
        (OutputFormat::Text, _) => emit_json(out, true)?,
        #[cfg(feature = "tty")]
        (OutputFormat::Tty, _) => crate::emit::tty::emit(out, std::iter::once(value.clone()))
            .map_err(|e| anyhow::anyhow!("tty emit: {e}"))?,
        #[cfg(not(feature = "tty"))]
        (OutputFormat::Tty, _) => {
            anyhow::bail!("--output tty requires the `tty` cargo feature")
        }
    }
    Ok(())
}

/// Resolve `OutputFormat::Auto` based on where stdout points. On a
/// terminal with `tty` support compiled in we render; elsewhere we
/// emit markdown so pipes stay clean.
fn resolve_format(requested: OutputFormat) -> OutputFormat {
    if !matches!(requested, OutputFormat::Auto) {
        return requested;
    }
    #[cfg(feature = "tty")]
    if io::stdout().is_terminal() {
        return OutputFormat::Tty;
    }
    OutputFormat::Md
}

fn apply_in_place(path: &Path, new_bytes: &[u8], backup: Option<&str>) -> anyhow::Result<()> {
    if let Some(ext) = backup {
        let backup_path = match path.extension() {
            Some(orig) => {
                let mut bp = path.to_path_buf();
                bp.set_extension(format!("{}.{ext}", orig.to_string_lossy()));
                bp
            }
            None => path.with_extension(ext),
        };
        fs::copy(path, &backup_path)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(new_bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}
