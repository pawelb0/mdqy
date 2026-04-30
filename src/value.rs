use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ast::Node;

/// Every value produced by mdqy evaluation.
///
/// The heavy variants wrap `Arc` so that `map(f)` over a stream of
/// 10k nodes is O(n) arc bumps, not O(n × tree size) copies.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(Arc<str>),
    Array(Arc<Vec<Value>>),
    Object(Arc<BTreeMap<String, Value>>),
    Node(Arc<Node>),
}

impl Value {
    /// `type` string. Nodes return their kind (`"heading"`, `"code"`,
    /// ...) so `select(type == "heading")` works without a separate
    /// predicate.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::Node(n) => n.kind.as_str(),
        }
    }

    /// jq rules: `null` and `false` are falsy, everything else is
    /// truthy. Empty containers and `0` are truthy.
    #[must_use]
    pub fn truthy(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Self::Number(n as f64)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::String(Arc::from(s))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::String(Arc::from(s))
    }
}

impl From<Node> for Value {
    fn from(n: Node) -> Self {
        Self::Node(Arc::new(n))
    }
}
