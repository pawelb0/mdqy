//! End-to-end query tests. Drives the public library API only.

use mdqy::{parse, Query, Value};
use pulldown_cmark::Parser;

const SRC: &str = include_str!("fixtures/tiny.md");
const STRESS: &str = include_str!("fixtures/stress.md");

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

fn stress_strings(expr: &str) -> Vec<String> {
    compile(expr)
        .run_tree(&parse(STRESS))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect()
}

/// Table-driven smoke test: expression -> expected stringified results.
#[test]
fn query_cases() {
    let cases: &[(&str, &[&str])] = &[
        ("headings | .text", &["Tiny", "Second heading"]),
        (
            "headings | select(.level == 2) | .text",
            &["Second heading"],
        ),
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
    assert_eq!(
        run_null("\"a,b,c\" | split(\",\") | join(\"-\")"),
        ["a-b-c"]
    );
    assert_eq!(run_null("[{n:3},{n:1},{n:2}] | min_by(.n) | .n"), ["1"]);
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
    let out: Vec<_> = q
        .run_with_env(Value::Null, env)
        .map(Result::unwrap)
        .collect();
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

/// `rows`, `cells`, `headers` cover table projection.
#[test]
fn table_builtins_project_rows_and_cells() {
    let src = std::fs::read_to_string("tests/fixtures/table.md").unwrap();
    let root = parse(&src);

    let headers: Vec<String> = compile("headers | .text")
        .run_tree(&root)
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(headers, ["Name", "Role", "Since"]);

    let row_kinds: Vec<String> = compile("rows | .kind")
        .run_tree(&root)
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(row_kinds.len(), 4);
    assert!(row_kinds.iter().all(|k| k == "row"));

    let cells: Vec<String> = compile("rows | cells | .text")
        .run_tree(&root)
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(cells.len(), 12);
    assert_eq!(cells[3], "Ada");
    assert_eq!(cells[11], "2024");
}

/// `"\(expr)"` interpolation.
#[test]
fn string_interpolation() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null(r#""hello \(1 + 2)!""#), "hello 3!");
    assert_eq!(run_null(r#""\(42)""#), "42");
    assert_eq!(run_null(r#""a\(1)b\(2)c""#), "a1b2c");
    assert_eq!(run_null(r#""plain""#), "plain");

    let headings: Vec<String> = compile(r#"headings | "h\(.level): \(.text)""#)
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(headings, ["h1: Tiny", "h2: Second heading"]);
}

/// `@format` filters.
#[test]
fn format_filters() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null(r#""a b & c" | @uri"#), "a%20b%20%26%20c");
    assert_eq!(run_null(r#"["Ada","Bo"] | @csv"#), r#""Ada","Bo""#);
    assert_eq!(run_null(r#"["a","b"] | @tsv"#), "a\tb");
    assert_eq!(
        run_null(r#"["one two","three"] | @sh"#),
        "'one two' 'three'"
    );
    assert_eq!(run_null(r#""<b>hi</b>" | @html"#), "&lt;b&gt;hi&lt;/b&gt;");
    assert_eq!(run_null(r"{x: 1} | @json"), r#"{"x":1}"#);
}

/// `error(msg)` raises a runtime error; `?` swallows it to `empty`.
#[test]
fn error_builtin_raises_and_catches() {
    let raised = compile(r#"error("boom")"#)
        .run_with_env(Value::Null, mdqy::Env::default())
        .next()
        .unwrap();
    assert!(raised.is_err(), "expected error, got {raised:?}");
    let caught = compile(r#"[error("boom")?]"#)
        .run_with_env(Value::Null, mdqy::Env::default())
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(
        matches!(&caught[0], Value::Array(a) if a.is_empty()),
        "expected empty array, got {caught:?}"
    );
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

/// `reduce` with an object-mutating update must not silently echo the
/// document. Either compute the histogram or surface an error.
#[test]
fn reduce_with_assign_does_not_swallow() {
    let q = compile(r#"reduce ("a","b","a") as $l ({}; .[$l] = (.[$l] // 0) + 1)"#);
    let mut out = q.run_with_env(Value::Null, mdqy::Env::default());
    let first = out.next().expect("at least one output");
    match first {
        Ok(Value::Object(m)) => {
            assert!(matches!(m.get("a"), Some(Value::Number(n)) if (n - 2.0).abs() < 1e-9));
            assert!(matches!(m.get("b"), Some(Value::Number(n)) if (n - 1.0).abs() < 1e-9));
        }
        Err(_) => {}
        Ok(other) => panic!("expected histogram or error, got {other:?}"),
    }
}

/// `is_read_only` should return true unless the expression directly
/// matches one of the patterns `transform_bytes` handles (`|=`, `del`,
/// `walk`) reachable through `Pipe`/`Comma`. Assignments nested inside
/// other constructors (object/array/if/reduce/foreach) target local
/// values, not the document.
#[test]
fn is_read_only_matches_mutate_grammar() {
    assert!(!compile(".foo |= 1").is_read_only());
    assert!(!compile("del(.foo)").is_read_only());
    assert!(!compile("walk(.)").is_read_only());
    assert!(!compile("headings | .text |= ascii_upcase").is_read_only());

    assert!(compile(r#"reduce ("a","b") as $l ({}; .[$l] = 1)"#).is_read_only());
    assert!(compile("foreach range(3) as $x ({}; .a = $x; .)").is_read_only());
    assert!(compile("{a: (.foo = 1)}").is_read_only());
    assert!(compile("if true then (.foo = 1) else . end").is_read_only());
    assert!(compile("[.foo |= 1]").is_read_only());
}

/// `as $x` should bind the immediately preceding term, not the whole
/// pipeline. Regression: `2 | . as $x | select($x > 1)` used to greedy-
/// absorb `2 | .` into the bind, leaving `outer` = the outer input.
#[test]
fn as_binds_preceding_term_not_pipeline() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(expr)
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(run_null("2 | . as $x | select($x > 1)"), ["2"]);
    assert_eq!(run_null("2 | (. as $x | select($x > 1))"), ["2"]);
    assert_eq!(run_null("5 | . as $x | $x + 1"), ["6"]);
}

/// `split("")` should split into single characters, matching jq.
#[test]
fn split_empty_yields_characters() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(&format!("{expr} | tojson"))
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(run_null(r#""abc" | split("")"#), [r#"["a","b","c"]"#]);
    assert_eq!(run_null(r#""" | split("")"#), ["[]"]);
}

/// `paths(f)` should return only paths whose value satisfies `f`.
/// Regression: predicate was silently ignored, returning all paths.
#[test]
fn paths_filter_applies_predicate() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(&format!("{expr} | tojson"))
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(
        run_null("{a: {b: 1}, c: 2} | [paths(. == 1)]"),
        [r#"[["a","b"]]"#],
    );
    assert_eq!(
        run_null("{a: {b: 1}, c: 2} | [paths(type == \"number\")]"),
        [r#"[["a","b"],["c"]]"#],
    );
}

/// Comparison operators chain left-to-right, matching jq.
/// `1 < 2 == true` parses as `(1 < 2) == true` and is `true`.
#[test]
fn comparisons_left_associate() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null("1 < 2 == true"), "true");
    assert_eq!(run_null("1 == 1 == true"), "true");
    assert_eq!(run_null("3 > 2 != false"), "true");
}

/// `any(f)` and `all(f)` should evaluate `f` per element and reduce
/// with OR/AND. Regression: predicate was silently ignored, leaving
/// truthy reduction of the raw items.
#[test]
fn any_all_apply_predicate() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null("[1,2,3] | any(. > 2)"), "true");
    assert_eq!(run_null("[1,2,3] | any(. > 99)"), "false");
    assert_eq!(run_null("[1,2,3] | all(. > 0)"), "true");
    assert_eq!(run_null("[1,2,3] | all(. > 1)"), "false");
    // 0-arg form still does truthy reduction of items.
    assert_eq!(run_null("[true, false] | any"), "true");
    assert_eq!(run_null("[true, false] | all"), "false");
}

/// String slicing should work like jq: clamp by Unicode codepoint.
#[test]
fn string_slice_clamps_by_codepoint() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(expr)
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(run_null(r#""abcdef" | .[1:4]"#), ["bcd"]);
    assert_eq!(run_null(r#""abcdef" | .[-2:]"#), ["ef"]);
    assert_eq!(run_null(r#""abcdef" | .[:0]"#), [""]);
    assert_eq!(run_null(r#""abc" | .[5:10]"#), [""]);
    assert_eq!(run_null(r#""héllo" | .[0:2]"#), ["hé"]);
}

/// `{k: stream}` should fan out across the value stream, producing
/// one object per output. jq spec: `{a: (1,2,3)}` yields 3 objects.
#[test]
fn object_ctor_fans_out_value_stream() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(&format!("{expr} | tojson"))
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(
        run_null("{a: (1,2,3)}"),
        [r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":3}"#],
    );
    assert_eq!(
        run_null("{a: (1,2), b: (10,20)}"),
        [
            r#"{"a":1,"b":10}"#,
            r#"{"a":1,"b":20}"#,
            r#"{"a":2,"b":10}"#,
            r#"{"a":2,"b":20}"#,
        ],
    );
}

/// `as $x` body should extend through subsequent pipes, matching jq.
/// `EXPR as $x | a | b | c` reads as `EXPR as $x | (a | b | c)` so
/// `$x` stays in scope across the whole rhs.
#[test]
fn as_body_extends_through_pipes() {
    fn run_null(expr: &str) -> Vec<String> {
        compile(expr)
            .run_with_env(Value::Null, mdqy::Env::default())
            .map(Result::unwrap)
            .map(|v| render(&v))
            .collect()
    }
    assert_eq!(run_null(r#""X" as $x | "Y" | $x"#), ["X"]);
    assert_eq!(run_null("1 as $x | 2 as $y | $x + $y"), ["3"]);
    assert_eq!(
        run_null(r#""X" as $x | ["a","X","b"] | map(. == $x) | tojson"#),
        ["[false,true,false]"],
    );
}

/// `not` works as a 0-ary postfix filter (`EXPR | not`) and as a
/// prefix unary (`not EXPR`), matching jq.
#[test]
fn not_works_as_prefix_and_postfix() {
    fn run_null(expr: &str) -> String {
        render(
            &compile(expr)
                .run_with_env(Value::Null, mdqy::Env::default())
                .next()
                .unwrap()
                .unwrap(),
        )
    }
    assert_eq!(run_null("null | true | not"), "false");
    assert_eq!(run_null("null | false | not"), "true");
    assert_eq!(run_null("null | (1 == 1) | not"), "false");
    assert_eq!(run_null("not true"), "false");
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

/// `sections` (no args) yields one Section per heading, body included.
#[test]
fn sections_yields_one_per_heading() {
    let kinds: Vec<_> = compile("sections | .kind")
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(kinds, ["section", "section"]);

    let titles: Vec<_> = compile("sections | .children[0].text")
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(titles, ["Tiny", "Second heading"]);
}

/// `sections(N)` filters by heading level.
#[test]
fn sections_filters_by_level() {
    let h2: Vec<_> = compile("sections(2) | .children[0].text")
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(h2, ["Second heading"]);

    let h3: Vec<_> = compile("[sections(3)] | length")
        .run_tree(&parse(SRC))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(h3, ["0"]);
}

/// `.text` on a Section flattens heading and body together.
#[test]
fn sections_text_includes_body() {
    let src = "## Alpha\n\nbody one.\n\n## Beta\n\nbody two.\n";
    let out: Vec<_> = compile("sections(2) | .text")
        .run_tree(&parse(src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["Alphabody one.", "Betabody two."]);
}

/// Each nested heading produces its own Section. Sub-sections
/// appear after their enclosing section, in document order.
#[test]
fn sections_recurse_into_nested_headings() {
    let src = "# A\n\nintro\n\n## B\n\nb body\n\n### C\n\nc body\n\n## D\n\nd body\n";
    let titles: Vec<_> = compile("sections | .children[0].text")
        .run_tree(&parse(src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(titles, ["A", "B", "C", "D"]);

    let h3: Vec<_> = compile("sections(3) | .text")
        .run_tree(&parse(src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(h3, ["Cc body"]);
}

/// jq-style level filter on the Section heading reads naturally.
#[test]
fn sections_level_filter_via_select() {
    let src = "# A\n\n## B\n\n## C\n";
    let out: Vec<_> = compile("sections | select(.children[0].level == 2) | .children[0].text")
        .run_tree(&parse(src))
        .map(Result::unwrap)
        .map(|v| render(&v))
        .collect();
    assert_eq!(out, ["B", "C"]);
}

/// Gnarly read queries against a rich fixture. Each row exercises a
/// distinct combination of language features. JSON outputs go through
/// `tojson` so the expected value stays a single canonical string.
#[test]
fn complex_query_stress() {
    let cases: &[(&str, &[&str])] = &[
        ("[sections] | length", &["7"]),
        ("[sections(2)] | length", &["3"]),
        ("[sections(3)] | length", &["2"]),
        (
            "sections(2) | .children[0].text",
            &["Install", "Usage", "Appendix"],
        ),
        (
            "# Install > codeblocks:lang(bash):first | .literal",
            &["sudo apt install foo\n"],
        ),
        ("# Usage > links | .href", &["#install"]),
        (
            r#"[.. | select(type == "code" and (.lang // "") == "rust")] | length"#,
            &["1"],
        ),
        (
            r#"if (frontmatter.title // null) == (h1:first | .text) then "match" else "mismatch" end"#,
            &["match"],
        ),
        (
            r#"headings | "\(.level)#\(.anchor): \(.text)""#,
            &[
                "1#stress-doc: Stress Doc",
                "2#install: Install",
                "3#linux: Linux",
                "3#macos: Macos",
                "2#usage: Usage",
                "2#appendix: Appendix",
                "4#deep-heading: Deep heading",
            ],
        ),
        (
            "[codeblocks | .lang] | group_by(.) | map({lang: .[0], count: length}) | sort_by(.lang) | tojson",
            &[r#"[{"count":4,"lang":"bash"},{"count":2,"lang":"python"},{"count":1,"lang":"rust"},{"count":1,"lang":"text"}]"#],
        ),
        (
            r#"[codeblocks | {lang, lines: (.literal | split("\n") | length)}] | group_by(.lang) | map({lang: .[0].lang, total: (map(.lines) | add)}) | sort_by(-.total) | tojson"#,
            &[r#"[{"lang":"bash","total":8},{"lang":"python","total":4},{"lang":"rust","total":4},{"lang":"text","total":2}]"#],
        ),
        (
            "[headings | .text] | sort_by(length) | .[0:3] | tojson",
            &[r#"["Linux","Macos","Usage"]"#],
        ),
        (
            r#"[.. | select(type == "link" or type == "image") | (.href // .src)] | unique | tojson"#,
            &[r##"["#install","#nowhere","http://old.example.com/path","https://example.com/","images/diagram.png"]"##],
        ),
        (
            r##"[headings | .anchor] as $a | [links | .href | select(startswith("#")) | ltrimstr("#") | (. as $x | select(($a | map(. == $x) | any) == false))] | tojson"##,
            &[r#"["nowhere"]"#],
        ),
    ];
    for (expr, want) in cases {
        assert_eq!(&stress_strings(expr), want, "query: {expr}");
    }
}

/// Mutation stress: each row asserts substring presence/absence on the
/// transformed bytes. Patterns are picked so substring containment is
/// unambiguous (no `## X` vs `### X` confusion).
#[test]
fn complex_mutation_stress() {
    type Case<'a> = (&'a str, &'a [&'a str], &'a [&'a str]);
    let cases: &[Case] = &[
        (
            r#"(.. | select(type == "link") | select(.href | startswith("http:"))).href |= sub("http:"; "https:")"#,
            &["https://old.example.com/path"],
            &["http://old.example.com"],
        ),
        (
            r#"(.. | select(type == "code") | select(.lang == "bash")).lang |= "shell""#,
            &["shell"],
            &["bash"],
        ),
    ];
    for (expr, must_contain, must_not_contain) in cases {
        let out = compile(expr)
            .transform_bytes(STRESS.as_bytes())
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
