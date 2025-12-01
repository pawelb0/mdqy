//! Directory walk that honours `.gitignore` and `.ignore`.
//!
//! Built on the `ignore` crate (same one ripgrep and fd use), so the
//! rules are the ones users already know. Output is sorted by
//! filename per directory, so runs are reproducible and diffs stay
//! clean.

use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Knobs for [`walk_inputs`]. All default off.
#[derive(Debug, Clone, Copy, Default)]
pub struct WalkOptions {
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub no_ignore: bool,
}

/// Yield every markdown file reachable from `paths`.
///
/// Files pass through as-is; directories recurse. Ordering is stable.
pub fn walk_inputs(
    paths: &[PathBuf],
    opts: WalkOptions,
) -> Box<dyn Iterator<Item = io::Result<PathBuf>>> {
    if paths.is_empty() {
        return Box::new(std::iter::empty());
    }

    // Split file args from directory args. Files go out verbatim;
    // directories feed the `ignore::WalkBuilder`. Keeping them apart
    // avoids paying the builder cost on file-only invocations.
    let mut direct_files: Vec<io::Result<PathBuf>> = Vec::new();
    let mut dirs: Vec<&Path> = Vec::new();

    for p in paths {
        if p.is_file() {
            direct_files.push(Ok(p.clone()));
        } else if p.is_dir() {
            dirs.push(p.as_path());
        } else {
            direct_files.push(Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("path not found: {}", p.display()),
            )));
        }
    }

    if dirs.is_empty() {
        return Box::new(direct_files.into_iter());
    }

    let mut builder = WalkBuilder::new(dirs[0]);
    for extra in &dirs[1..] {
        builder.add(extra);
    }
    builder
        .standard_filters(!opts.no_ignore)
        .hidden(!opts.include_hidden)
        .follow_links(opts.follow_symlinks)
        .sort_by_file_name(std::cmp::Ord::cmp);

    let walker = builder.build();
    let dir_stream = walker.filter_map(|entry| match entry {
        Ok(e) => {
            if !e.file_type().is_some_and(|t| t.is_file()) {
                return None;
            }
            let path = e.into_path();
            is_markdown_ext(&path).then_some(Ok(path))
        }
        Err(err) => Some(Err(io::Error::other(err.to_string()))),
    });

    Box::new(direct_files.into_iter().chain(dir_stream))
}

/// `true` if `path` has an extension mdqy knows how to parse.
#[must_use]
pub fn is_markdown_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn walks_only_markdown_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        fs::write(dir.path().join("b.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("c.markdown"), "# C").unwrap();
        let paths = vec![dir.path().to_path_buf()];
        let found: Vec<String> = walk_inputs(&paths, WalkOptions::default())
            .map(|r| r.unwrap().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["a.md", "c.markdown"]);
    }

    #[test]
    fn respects_gitignore() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "hidden.md\n").unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        fs::write(dir.path().join("hidden.md"), "# nope").unwrap();
        // `ignore` requires a `.git` marker for `.gitignore` to apply; use `.ignore` for direct.
        fs::write(dir.path().join(".ignore"), "hidden.md\n").unwrap();
        let paths = vec![dir.path().to_path_buf()];
        let found: Vec<String> = walk_inputs(&paths, WalkOptions::default())
            .map(|r| r.unwrap().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["a.md"]);
    }

    #[test]
    fn positional_files_pass_through() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        let paths = vec![dir.path().join("a.md")];
        let found: Vec<_> = walk_inputs(&paths, WalkOptions::default()).collect();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn deterministic_order() {
        let dir = tempdir().unwrap();
        for name in ["z.md", "a.md", "m.md"] {
            fs::write(dir.path().join(name), "# x").unwrap();
        }
        let paths = vec![dir.path().to_path_buf()];
        let found: Vec<String> = walk_inputs(&paths, WalkOptions::default())
            .map(|r| r.unwrap().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["a.md", "m.md", "z.md"]);
    }
}
