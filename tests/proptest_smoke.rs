//! Property tests for the public library surface. None of these
//! pin specific behaviour — they assert invariants that should hold
//! across every input the strategies generate.
//!
//! What they catch:
//!   * panics in the lexer or parser on hostile/garbage input
//!   * non-byte-exact round-trips for clean synthetic markdown
//!   * runtime panics in the tree evaluator on simple expressions

use mdqy::{parse, Query};
use proptest::prelude::*;

/// Build a `String` of pseudo-random bytes from the alphabet most
/// likely to confuse the lexer: punctuation it tokenises, escapes,
/// and a handful of letters / digits to keep idents possible.
fn lex_garbage_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just('"'), Just('\\'), Just('('), Just(')'), Just('['), Just(']'),
            Just('{'), Just('}'), Just('|'), Just(';'), Just(','), Just('.'),
            Just(':'), Just('@'), Just('$'), Just('#'), Just('?'), Just('+'),
            Just('-'), Just('*'), Just('/'), Just('%'), Just('<'), Just('>'),
            Just('='), Just('!'), Just(' '), Just('\n'), Just('\t'),
            Just('a'), Just('h'), Just('1'), Just('0'), Just('_'),
        ],
        0..120,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// Synthetic expressions over the documented vocabulary. Composed by
/// concatenating snippets, so the final string is rarely valid — that's
/// fine; the property only asserts no-panic.
fn expr_strategy() -> impl Strategy<Value = String> {
    let snippets: &[&str] = &[
        ".", "..", "h1", "h2", "h6", "headings", "codeblocks", "links",
        "| .text", "| .level", "| select(.level == 1)", " | first",
        ":first", ":last", ":nth(0)", ":nth(-1)", ":lang(rust)",
        "[range(5)]", "length", "tostring", "@json", "@csv", "@uri",
        " | sort_by(.level)", " | unique", " | reverse",
        "{a:1, b:2}", "{(.x):1}", "[1, 2, 3]", "1 + 2", "if true then 1 else 2 end",
        "as $x", "$x", "(.foo)?", "5 // null", "del(.title)",
        ".foo |= 1", "walk(.)", "walk(.level |= . + 1)",
    ];
    proptest::collection::vec(proptest::sample::select(snippets), 1..6)
        .prop_map(|parts| parts.join(" "))
}

/// Markdown built from blocks that have stable round-trip behaviour
/// (heading, paragraph, fenced code, hr). Avoids tables / lists where
/// pulldown-cmark + cmark-to-cmark have known formatting drift.
fn clean_doc_strategy() -> impl Strategy<Value = String> {
    let block = prop_oneof![
        ("[a-zA-Z][a-zA-Z0-9 ]{0,30}", 1u8..=6).prop_map(|(t, l)| {
            format!("{} {}\n", "#".repeat(l as usize), t)
        }),
        "[a-z][a-z .]{1,40}".prop_map(|p| format!("{p}\n")),
        ("[a-z]{2,8}", "[a-z0-9 ]{0,30}").prop_map(|(lang, body)| {
            format!("```{lang}\n{body}\n```\n")
        }),
        Just("---\n".to_string()),
    ];
    proptest::collection::vec(block, 0..6)
        .prop_map(|blocks| blocks.join("\n"))
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// Compile must never panic, even on completely random bytes.
    /// `Query::compile` should always return `Ok` or `Err`.
    #[test]
    fn compile_never_panics_on_garbage(s in lex_garbage_strategy()) {
        let _ = Query::compile(&s);
    }

    /// Compile on hand-shaped expression fragments must not panic.
    #[test]
    fn compile_never_panics_on_expr_snippets(s in expr_strategy()) {
        let _ = Query::compile(&s);
    }

    /// `parse` (the markdown front-end) must not panic on arbitrary
    /// strings.
    #[test]
    fn parse_never_panics(s in lex_garbage_strategy()) {
        let _ = parse(&s);
    }

    /// Identity query is byte-exact for a clean synthetic doc. The
    /// emitter byte-copies clean subtrees from source, so unless the
    /// generator hits something that confuses the parser the output
    /// must equal the input.
    #[test]
    fn identity_round_trip_byte_exact(doc in clean_doc_strategy()) {
        let q = Query::compile(".").unwrap();
        let bytes = q.transform_bytes(doc.as_bytes())
            .expect("identity transform_bytes");
        prop_assert_eq!(bytes, doc.as_bytes().to_vec());
    }

    /// Running a stream-eligible read query against an arbitrary doc
    /// must not panic and must agree with the tree evaluator.
    #[test]
    fn stream_tree_agree_on_headings_text(doc in clean_doc_strategy()) {
        let q = Query::compile("headings | .text").unwrap();
        // Stream path:
        let stream_out: Vec<String> = q
            .run(pulldown_cmark::Parser::new_ext(&doc, mdqy::markdown_options()))
            .filter_map(|r| match r.ok()? {
                mdqy::Value::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect();
        // Tree path: force tree by wrapping in [...] | .[]
        let tree_q = Query::compile("[headings | .text] | .[]").unwrap();
        let tree_out: Vec<String> = tree_q
            .run_tree(&parse(&doc))
            .filter_map(|r| match r.ok()? {
                mdqy::Value::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect();
        prop_assert_eq!(stream_out, tree_out);
    }
}
