//! Emit the Node JSON schema to stdout. Gated behind the
//! `schema-export` cargo feature.
//!
//! ```sh
//! cargo run --example export_schema --features schema-export \
//!     > docs/node.schema.json
//! ```

#[cfg(feature = "schema-export")]
fn main() {
    let schema = schemars::schema_for!(mdqy::NodeKind);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}

#[cfg(not(feature = "schema-export"))]
fn main() {
    eprintln!("rebuild with --features schema-export");
    std::process::exit(2);
}
