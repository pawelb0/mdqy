//! Property tests over the public library surface. Invariants only;
//! no specific behaviour is pinned here.

use mdqy::{parse, Query};
use proptest::prelude::*;

/// Random bytes from the alphabet that's most likely to confuse the
/// lexer: punctuation, escapes, and a few letters and digits.
fn lex_garbage_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just('"'),
            Just('\\'),
            Just('('),
            Just(')'),
            Just('['),
            Just(']'),
            Just('{'),
            Just('}'),
            Just('|'),
            Just(';'),
            Just(','),
            Just('.'),
            Just(':'),
            Just('@'),
            Just('$'),
            Just('#'),
            Just('?'),
            Just('+'),
            Just('-'),
            Just('*'),
            Just('/'),
            Just('%'),
            Just('<'),
            Just('>'),
            Just('='),
            Just('!'),
            Just(' '),
            Just('\n'),
            Just('\t'),
            Just('a'),
            Just('h'),
            Just('1'),
            Just('0'),
            Just('_'),
        ],
        0..120,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
}

/// Concatenation of expression snippets. Mostly invalid grammar; the
/// property is no-panic.
fn expr_strategy() -> impl Strategy<Value = String> {
    let snippets: &[&str] = &[
        ".",
        "..",
        "h1",
        "h2",
        "h6",
        "headings",
        "codeblocks",
        "links",
        "| .text",
        "| .level",
        "| select(.level == 1)",
        " | first",
        ":first",
        ":last",
        ":nth(0)",
        ":nth(-1)",
        ":lang(rust)",
        "[range(5)]",
        "length",
        "tostring",
        "@json",
        "@csv",
        "@uri",
        " | sort_by(.level)",
        " | unique",
        " | reverse",
        "{a:1, b:2}",
        "{(.x):1}",
        "[1, 2, 3]",
        "1 + 2",
        "if true then 1 else 2 end",
        "as $x",
        "$x",
        "(.foo)?",
        "5 // null",
        "del(.title)",
        ".foo |= 1",
        "walk(.)",
        "walk(.level |= . + 1)",
    ];
    proptest::collection::vec(proptest::sample::select(snippets), 1..6)
        .prop_map(|parts| parts.join(" "))
}

/// Markdown built from heading / paragraph / fenced-code / hr blocks.
/// Tables and lists are out: cmark round-trip drifts on those.
fn clean_doc_strategy() -> impl Strategy<Value = String> {
    let block = prop_oneof![
        ("[a-zA-Z][a-zA-Z0-9 ]{0,30}", 1u8..=6)
            .prop_map(|(t, l)| { format!("{} {}\n", "#".repeat(l as usize), t) }),
        "[a-z][a-z .]{1,40}".prop_map(|p| format!("{p}\n")),
        ("[a-z]{2,8}", "[a-z0-9 ]{0,30}")
            .prop_map(|(lang, body)| { format!("```{lang}\n{body}\n```\n") }),
        Just("---\n".to_string()),
    ];
    proptest::collection::vec(block, 0..6).prop_map(|blocks| blocks.join("\n"))
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn compile_never_panics_on_garbage(s in lex_garbage_strategy()) {
        let _ = Query::compile(&s);
    }

    #[test]
    fn compile_never_panics_on_expr_snippets(s in expr_strategy()) {
        let _ = Query::compile(&s);
    }

    #[test]
    fn parse_never_panics(s in lex_garbage_strategy()) {
        let _ = parse(&s);
    }

    /// Identity output equals the source for a clean synthetic doc.
    #[test]
    fn identity_round_trip_byte_exact(doc in clean_doc_strategy()) {
        let q = Query::compile(".").unwrap();
        let bytes = q.transform_bytes(doc.as_bytes())
            .expect("identity transform_bytes");
        prop_assert_eq!(bytes, doc.as_bytes().to_vec());
    }

    /// Stream and tree paths produce the same headings text.
    #[test]
    fn stream_tree_agree_on_headings_text(doc in clean_doc_strategy()) {
        let q = Query::compile("headings | .text").unwrap();
        let stream_out: Vec<String> = q
            .run(pulldown_cmark::Parser::new_ext(&doc, mdqy::markdown_options()))
            .filter_map(|r| match r.ok()? {
                mdqy::Value::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect();
        // Wrapping in `[...] | .[]` forces tree mode.
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
