//! Proof that every example in `docs/quickstart.md` still runs.
//!
//! One test per commanded example. The fixtures live under
//! `tests/fixtures/quickstart/` so the outputs don't drift with
//! project docs.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

const GUIDE: &str = "tests/fixtures/quickstart/guide.md";
const DOCS: &str = "tests/fixtures/quickstart/docs";

fn mdqy() -> Command {
    Command::cargo_bin("mdqy").expect("binary built")
}

fn copy_guide(dir: &Path) -> std::path::PathBuf {
    let dst = dir.join("guide.md");
    fs::copy(GUIDE, &dst).unwrap();
    dst
}

// Step 0: the shape of a query

#[test]
fn identity_byte_exact_roundtrip() {
    let out = mdqy().args([".", GUIDE]).assert().success().get_output().stdout.clone();
    assert_eq!(out, fs::read(GUIDE).unwrap());
}

#[test]
fn stdin_feeds_the_query() {
    let body = fs::read_to_string(GUIDE).unwrap();
    mdqy()
        .args(["--stdin", "."])
        .write_stdin(body.clone())
        .assert()
        .success()
        .stdout(predicate::eq(body));
}

// Step 1: pull something out

#[test]
fn headings_text_lists_every_heading() {
    mdqy()
        .args(["headings | .text", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("Guide"))
        .stdout(predicate::str::contains("Install"))
        .stdout(predicate::str::contains("Usage"))
        .stdout(predicate::str::contains("Query examples"))
        .stdout(predicate::str::contains("Output format"))
        .stdout(predicate::str::contains("Notes"));
}

#[test]
fn links_href_yields_http_targets() {
    mdqy()
        .args(["links | .href", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("http://example.com"));
}

#[test]
fn codeblocks_lang_yields_fence_tags() {
    mdqy()
        .args(["codeblocks | .lang", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("sh"))
        .stdout(predicate::str::contains("rust"))
        .stdout(predicate::str::contains("bash"));
}

// Step 2: filter

#[test]
fn select_level_two_drops_h1_and_h3() {
    mdqy()
        .args(["headings | select(.level == 2) | .text", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("Install"))
        .stdout(predicate::str::contains("Usage"))
        .stdout(predicate::str::contains("Notes"))
        .stdout(predicate::str::contains("Query examples").not())
        .stdout(predicate::str::contains("Guide").not());
}

#[test]
fn h2_shorthand_matches_longhand() {
    let short = mdqy()
        .args(["h2 | .text", GUIDE])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let long = mdqy()
        .args(["headings | select(.level == 2) | .text", GUIDE])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(short, long);
}

#[test]
fn code_lang_rust_filters_by_fence_tag() {
    mdqy()
        .args(["code:lang(rust) | .literal", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("let x = 1"))
        .stdout(predicate::str::contains("fn first()"))
        .stdout(predicate::str::contains("echo hello").not())
        .stdout(predicate::str::contains("cargo install").not());
}

// Step 3: drill into a section

#[test]
fn hash_install_returns_install_section() {
    mdqy()
        .args(["# Install", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo install guide"))
        .stdout(predicate::str::contains("let x = 1"))
        .stdout(predicate::str::contains("echo hello").not());
}

#[test]
fn combinator_scopes_codeblocks_to_install() {
    mdqy()
        .args(["# Install > codeblocks | .literal", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo install guide"))
        .stdout(predicate::str::contains("let x = 1"))
        .stdout(predicate::str::contains("fn first").not());
}

#[test]
fn nested_combinator_reaches_query_examples() {
    mdqy()
        .args([r#"# Usage > ## "Query examples" > codeblocks:first | .literal"#, GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("fn first"))
        .stdout(predicate::str::contains("echo hello").not());
}

// Step 4: shape the output

#[test]
fn object_ctor_projects_attrs() {
    mdqy()
        .args(["headings | {level, text, anchor}", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"level\""))
        .stdout(predicate::str::contains("\"text\""))
        .stdout(predicate::str::contains("\"anchor\""))
        .stdout(predicate::str::contains("\"install\""));
}

#[test]
fn array_ctor_wraps_stream() {
    let output = mdqy()
        .args(["[headings | .text]", GUIDE])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let arr = parsed.as_array().expect("array");
    let texts: Vec<&str> = arr.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(
        texts,
        vec!["Guide", "Install", "Usage", "Query examples", "Output format", "Notes"]
    );
}

#[test]
fn reduce_counts_codeblock_languages() {
    let output = mdqy()
        .args([
            r#"reduce (codeblocks | .lang // "plain") as $l ({}; setpath([$l]; (getpath([$l]) // 0) + 1))"#,
            GUIDE,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["rust"], 2);
    assert_eq!(parsed["bash"], 1);
    assert_eq!(parsed["sh"], 1);
}

// Step 5: switch output format

#[test]
fn output_json_emits_node_schema() {
    mdqy()
        .args(["--output", "json", "headings", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"heading\""))
        .stdout(predicate::str::contains("\"level\": 1"));
}

#[test]
fn output_md_preserves_section_heading_marker() {
    mdqy()
        .args(["--output", "md", "# Install", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Install"))
        .stdout(predicate::str::contains("cargo install guide"));
}

#[test]
fn output_text_strips_json_quoting() {
    mdqy()
        .args(["--output", "text", "headings | .text", GUIDE])
        .assert()
        .success()
        .stdout(predicate::str::contains("Guide\n"))
        .stdout(predicate::str::contains("\"Guide\"").not());
}

// Step 6: many files at once

#[test]
fn directory_walk_scans_every_md() {
    mdqy()
        .args(["headings | .text", DOCS])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha"))
        .stdout(predicate::str::contains("A-sub"))
        .stdout(predicate::str::contains("Beta"))
        .stdout(predicate::str::contains("B-sub"));
}

#[test]
fn with_path_tags_multi_file_results() {
    mdqy()
        .args(["--with-path", "--output", "json", "headings | .text", DOCS])
        .assert()
        .success()
        .stdout(predicate::str::contains("a.md"))
        .stdout(predicate::str::contains("b.md"))
        .stdout(predicate::str::contains("\"path\""))
        .stdout(predicate::str::contains("\"value\""));
}

#[test]
fn merge_runs_query_over_concatenated_stream() {
    mdqy()
        .args(["--merge", "code:lang(rust) | .literal", DOCS])
        .assert()
        .success()
        .stdout(predicate::str::contains("fn alpha"))
        .stdout(predicate::str::contains("echo b").not());
}

#[test]
fn slurp_exposes_array_of_roots() {
    let output = mdqy()
        .args(["--slurp", "length", DOCS])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let n: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(n, 2);
}

// Step 7: rewrite in place

#[test]
fn dry_run_prints_diff_and_leaves_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let target = copy_guide(dir.path());
    let before = fs::read(&target).unwrap();

    mdqy()
        .args([
            "--dry-run",
            r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#,
            target.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("-").and(predicate::str::contains("http://example.com")))
        .stdout(predicate::str::contains("+").and(predicate::str::contains("https://example.com")));

    assert_eq!(fs::read(&target).unwrap(), before);
}

#[test]
fn in_place_rewrites_links() {
    let dir = tempfile::tempdir().unwrap();
    let target = copy_guide(dir.path());
    mdqy()
        .args([
            "-U",
            r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#,
            target.to_str().unwrap(),
        ])
        .assert()
        .success();
    let after = fs::read_to_string(&target).unwrap();
    assert!(after.contains("https://example.com"), "got: {after}");
    assert!(!after.contains("http://example.com"), "got: {after}");
}

#[test]
fn walk_bumps_heading_levels() {
    let dir = tempfile::tempdir().unwrap();
    let target = copy_guide(dir.path());
    mdqy()
        .args([
            "-U",
            r#"walk(if type == "heading" then .level |= (. + 1) else . end)"#,
            target.to_str().unwrap(),
        ])
        .assert()
        .success();
    let after = fs::read_to_string(&target).unwrap();
    assert!(after.contains("## Guide"), "H1 became H2 missing: {after}");
    assert!(after.contains("### Install"), "H2 became H3 missing: {after}");
    assert!(
        !after.lines().any(|l| l.starts_with("# Guide")),
        "stale H1 left: {after}"
    );
}

#[test]
fn walk_strips_image_titles() {
    let dir = tempfile::tempdir().unwrap();
    let target = copy_guide(dir.path());
    assert!(fs::read_to_string(&target).unwrap().contains("\"caption\""));
    mdqy()
        .args([
            "-U",
            r#"walk(if type == "image" then del(.title) else . end)"#,
            target.to_str().unwrap(),
        ])
        .assert()
        .success();
    let after = fs::read_to_string(&target).unwrap();
    assert!(!after.contains("\"caption\""), "title survived: {after}");
    assert!(after.contains("diagram"), "alt dropped: {after}");
    assert!(after.contains("img.png"), "src dropped: {after}");
}

// Step 8: pipe into jq, ripgrep, whatever

#[test]
fn json_output_is_machine_parseable_per_result() {
    let output = mdqy()
        .args(["--output", "json", "codeblocks", GUIDE])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    let langs: Vec<String> = serde_json::Deserializer::from_str(&text)
        .into_iter::<serde_json::Value>()
        .map(|v| v.unwrap()["lang"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(langs, vec!["sh", "rust", "rust", "bash"]);
}

#[test]
fn raw_output_strips_quotes_for_shell_loops() {
    mdqy()
        .args(["--raw", "headings | .text", DOCS])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha\n"))
        .stdout(predicate::str::contains("\"Alpha\"").not());
}

// External pipe tip: markdown output must be valid for mdcat / glow.

#[test]
fn output_md_for_external_renderers_is_parseable_markdown() {
    let out = mdqy()
        .args(["--output", "md", "# Install", GUIDE])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let md = String::from_utf8(out).unwrap();
    assert!(md.contains("## Install"));
    assert!(md.contains("```sh"));
    assert!(md.contains("cargo install guide"));
}
