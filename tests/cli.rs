//! Black-box tests for the `mdqy` binary. Drives the CLI through
//! `assert_cmd` so scripts that depend on flag semantics get early
//! warning when behaviour drifts.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

const TINY: &str = "tests/fixtures/tiny.md";

fn mdqy() -> Command {
    Command::cargo_bin("mdqy").expect("binary built")
}

#[test]
fn output_json_contains_node_schema_keys() {
    mdqy()
        .args(["--output", "json", "headings | .text", TINY])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tiny"))
        .stdout(predicate::str::contains("Second heading"));
}

#[test]
fn null_input_runs_without_a_file() {
    mdqy()
        .args(["-n", "1 + 2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3"));
}

#[test]
fn raw_input_binds_source_string() {
    mdqy()
        .args(["-R", "--stdin", "."])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn slurp_binds_array_of_roots() {
    mdqy()
        .args(["--slurp", "length", TINY])
        .assert()
        .success()
        .stdout(predicate::str::contains("1"));
}

#[test]
fn dry_run_prints_diff_does_not_write() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    fs::write(tmp.path(), b"See [x](http://e.com).\n").unwrap();
    let before = fs::read(tmp.path()).unwrap();
    mdqy()
        .args([
            "--dry-run",
            r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#,
            tmp.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("-See"))
        .stdout(predicate::str::contains("+See"));
    assert_eq!(fs::read(tmp.path()).unwrap(), before);
}

#[test]
fn in_place_rewrites_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    fs::write(tmp.path(), b"[x](http://e.com)\n").unwrap();
    mdqy()
        .args([
            "-U",
            r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#,
            tmp.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let after = fs::read_to_string(tmp.path()).unwrap();
    assert!(after.contains("https://e.com"), "got: {after}");
    assert!(!after.contains("http://e.com"), "got: {after}");
}

#[test]
fn with_path_tags_json_output() {
    mdqy()
        .args(["--output", "json", "--with-path", "headings | .text", TINY])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"path\""))
        .stdout(predicate::str::contains(TINY));
}

#[test]
fn compile_error_exits_with_caret() {
    mdqy()
        .args([". ~ garbage", TINY])
        .assert()
        .failure()
        .stderr(predicate::str::contains("^"));
}

#[test]
fn workers_preserves_sequential_output() {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [
        ("a.md", "# A\n## A-sub\n"),
        ("b.md", "# B\n## B-sub\n"),
        ("c.md", "# C\n## C-sub\n"),
        ("d.md", "# D\n## D-sub\n"),
    ] {
        fs::write(dir.path().join(name), body).unwrap();
    }
    let serial = mdqy()
        .args(["headings | .text", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parallel = mdqy()
        .args(["--workers", "4", "headings | .text", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serial, parallel);
}

#[test]
fn arg_and_argjson_bind_variables() {
    mdqy()
        .args(["--arg", "name", "World", "-n", r#""hi " + $name"#])
        .assert()
        .success()
        .stdout(predicate::str::contains("hi World"));

    mdqy()
        .args(["--argjson", "n", "41", "-n", "$n + 1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}
