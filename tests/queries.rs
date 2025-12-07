//! End-to-end query tests. Drives the public library API only.

use mdqy::{parse, Query, Value};
use pulldown_cmark::Parser;

const SRC: &str = include_str!("fixtures/tiny.md");

fn compile(expr: &str) -> Query {
    Query::compile(expr).unwrap_or_else(|e| panic!("compile {expr}: {e}"))
}

fn run(expr: &str) -> Vec<Value> {
    compile(expr)
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .collect()
}

fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Number(n) if n.fract() == 0.0 => (*n as i64).to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        _ => format!("{v:?}"),
    }
}

fn strings(expr: &str) -> Vec<String> {
    run(expr).iter().map(render).collect()
}

/// Table-driven smoke test: expression -> expected stringified results.
#[test]
fn query_cases() {
    let cases: &[(&str, &[&str])] = &[
        ("headings | .text", &["Tiny", "Second heading"]),
        ("headings | select(.level == 2) | .text", &["Second heading"]),
        ("codeblocks | .lang", &["rust"]),
        ("links | .href", &["https://example.com"]),
        ("[headings] | length", &["2"]),
        (
            ".. | select(type == \"heading\") | .text",
            &["Tiny", "Second heading"],
        ),
        ("headings | first(.text)", &["Tiny", "Second heading"]),
        ("h1 | .text", &["Tiny"]),
        ("h1:first | .text", &["Tiny"]),
        ("h2:first | .text", &["Second heading"]),
        ("headings:nth(1) | .text", &["Second heading"]),
        ("headings:nth(-1) | .text", &["Second heading"]),
        ("headings:last | .text", &["Second heading"]),
        (r#"codeblocks:lang("rust") | .lang"#, &["rust"]),
    ];
    for (expr, want) in cases {
        assert_eq!(&strings(expr), want, "query: {expr}");
    }
}

/// Object construction returns the right type + shape.
#[test]
fn object_ctor_shape() {
    let out = run("headings | {level: .level, text: .text}");
    assert_eq!(out.len(), 2);
    let Value::Object(map) = &out[0] else {
        panic!("expected object")
    };
    assert!(matches!(map.get("level"), Some(Value::Number(n)) if (*n - 1.0).abs() < 1e-9));
    assert!(matches!(map.get("text"), Some(Value::String(s)) if s.as_ref() == "Tiny"));
}

/// One row per transformation case.
/// `(source, expression, must_contain, must_not_contain)`.
type MutationCase<'a> = (&'a [u8], &'a str, &'a [&'a str], &'a [&'a str]);

/// Run `transform_bytes` on each case and check what's in and what's
/// out of the resulting bytes.
#[test]
fn mutation_cases() {
    let cases: &[MutationCase] = &[
        // (source, expression, must_contain, must_not_contain)
        (
            b"See [docs](http://example.com).\n",
            r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#,
            &["https://example.com", "See "],
            &["http://"],
        ),
        (
            b"See [docs](https://example.com).\n",
            r#"(.. | select(type == "link") | select(.href | startswith("http:"))).href |= sub("http:"; "https:")"#,
            &["https://example.com"],
            &[],
        ),
        (
            b"[docs](https://example.com \"My Title\")\n",
            r#"del((.. | select(type == "link")).title)"#,
            &["docs", "https://example.com"],
            &["My Title"],
        ),
    ];
    for (source, expr, must_contain, must_not_contain) in cases {
        let out = compile(expr)
            .transform_bytes(source)
            .unwrap_or_else(|e| panic!("transform {expr}: {e}"));
        let text = String::from_utf8(out).unwrap();
        for needle in *must_contain {
            assert!(text.contains(needle), "{expr}: missing `{needle}`\n{text}");
        }
        for needle in *must_not_contain {
            assert!(
                !text.contains(needle),
                "{expr}: should exclude `{needle}`\n{text}"
            );
        }
    }
}

/// Builtins added after the initial set. Each row is a query that
/// should compile and yield the listed stringified outputs when fed
/// `null` via `--null-input` semantics.
#[test]
fn extra_builtins() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(expr)
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(run_null("[range(3)] | length"), ["3"]);
    assert_eq!(run_null("[limit(2; range(100))] | length"), ["2"]);
    assert_eq!(run_null("nth(1; range(10))"), ["1"]);
    assert_eq!(run_null("\"a,b,c\" | split(\",\") | join(\"-\")"), ["a-b-c"]);
    assert_eq!(
        run_null("[{n:3},{n:1},{n:2}] | min_by(.n) | .n"),
        ["1"]
    );
    assert_eq!(run_null("\"hello world\" | contains(\"world\")"), ["true"]);
    assert_eq!(
        run_null(r#"{a:{b:1}} | setpath(["a","b"]; 99) | getpath(["a","b"])"#),
        ["99"]
    );
}

/// `$foo` resolves through `Env` when the caller pre-populates it.
#[test]
fn env_bindings_thread_through() {
    let q = compile("$greet + \" \" + $name");
    let env = mdqy::Env::default()
        .with("greet", Value::from("hi"))
        .with("name", Value::from("world"));
    let out: Vec<_> = q.run_with_env(Value::Null, env).map(Result::unwrap).collect();
    assert_eq!(render(&out[0]), "hi world");
}

/// `def`, `reduce`, `foreach`, and `as $x` all compile and run.
#[test]
fn control_constructs() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null("[1,2,3] as $xs | [$xs[] | . + 10] | length"), "3");
    assert_eq!(run_null("reduce range(5) as $x (0; . + $x)"), "10");
    assert_eq!(
        run_null("[foreach range(4) as $x (0; . + $x; .)] | length"),
        "4"
    );
    assert_eq!(run_null("def inc: . + 1; 3 | inc"), "4");
    assert_eq!(run_null("def pick(f): . | f; {x:1,y:2} | pick(.y)"), "2");
    assert_eq!(run_null("def add(a; b): a + b; 0 | add(10; 20)"), "30");
}

/// YAML + TOML frontmatter parse into the root `frontmatter` attr
/// and are reachable via the builtin.
#[test]
fn frontmatter_parses() {
    let yaml = "---\ntitle: Hi\ncount: 3\n---\n\n# Body\n";
    let out: Vec<_> = compile("frontmatter | .title")
        .run_tree(&parse(yaml))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["Hi"]);

    let toml_src = "+++\ntitle = \"Hi\"\ncount = 3\n+++\n\n# Body\n";
    let out: Vec<_> = compile("frontmatter | .count")
        .run_tree(&parse(toml_src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["3"]);
}

/// `walk(f)` with `|=` inside `f` mutates attrs on matching nodes.
#[test]
fn walk_mutation_bumps_heading_levels() {
    let src = b"# one\n\n## two\n";
    let out = compile(r#"walk(if type == "heading" then .level |= (. + 1) else . end)"#)
        .transform_bytes(src)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("## one"), "{text}");
    assert!(text.contains("### two"), "{text}");
}

/// `node(obj)` constructs a fresh Node from a shape object.
#[test]
fn node_constructor_round_trips_kind() {
    let q = compile("node({kind:\"heading\", level:4}) | .kind");
    let out: Vec<_> = q
        .run_with_env(Value::Null, mdqy::Env::default())
        .map(Result::unwrap)
        .collect();
    assert_eq!(render(&out[0]), "heading");
}

/// `>` combinator scopes into the current heading's section and
/// doesn't hijack the `>` comparison operator.
#[test]
fn selector_combinator_and_gt_both_work() {
    let src = "# Install\n\n## Linux\n\n```sh\napt\n```\n\n## Macos\n\n```sh\nbrew\n```\n";
    // Combinator picks the right code block.
    let out: Vec<_> = compile("# Install > codeblocks:first | .literal")
        .run_tree(&parse(src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["apt\n"]);

    // `>` between scalars still means greater-than.
    let out: Vec<_> = compile("5 > 3")
        .run_with_env(Value::Null, mdqy::Env::default())
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["true"]);
}

/// `# Title` sugar desugars to `section("Title")`.
#[test]
fn hash_selector_matches_section() {
    let out: Vec<_> = compile("# \"Second heading\" | .kind")
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["section"]);
}

/// Stream and tree runners agree for every stream-eligible query.
/// If they drift, one of them has regressed.
#[test]
fn stream_and_tree_agree() {
    let queries = [
        "headings | .text",
        "headings | .level",
        "h1 | .text",
        "h2 | .anchor",
        "codeblocks | .lang",
        "codeblocks | .literal",
        "links | .href",
        "headings | select(.level == 2) | .text",
    ];
    for expr in queries {
        let q = compile(expr);
        let tree: Vec<_> = q
            .run_tree(&parse(SRC))
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect();
        let stream: Vec<_> = q
            .run(Parser::new_ext(SRC, mdqy::markdown_options()))
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect();
        assert_eq!(tree, stream, "divergence on `{expr}`");
    }
}
