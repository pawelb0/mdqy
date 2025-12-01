//! JSON output shape check.
//!
//! Parse a fixture, emit its root Node as JSON, and look for the
//! fields downstream tooling depends on. Guards against accidental
//! renames in the flattened schema.

use mdqy::emit::json::{emit, JsonOptions};
use mdqy::{parse, Value};

#[test]
fn identity_emits_root_node_json() {
    let src = std::fs::read_to_string("tests/fixtures/tiny.md").expect("fixture reads");
    let root = parse(&src);
    let mut out = Vec::new();
    emit(&mut out, &Value::from(root), JsonOptions::default()).expect("emit ok");
    let text = String::from_utf8(out).expect("utf8");

    assert!(text.contains("\"kind\": \"root\""));
    assert!(text.contains("\"kind\": \"heading\""));
    assert!(text.contains("\"level\": 1"));
    assert!(text.contains("\"anchor\": \"tiny\""));
    assert!(text.contains("\"kind\": \"code\""));
    assert!(text.contains("\"lang\": \"rust\""));
    assert!(text.contains("\"href\": \"https://example.com\""));
}
