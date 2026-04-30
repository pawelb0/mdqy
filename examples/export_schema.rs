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
