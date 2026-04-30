/// Strategy for multi-file input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Aggregation {
    /// Run the query once per file. Default, and what `jq . a.json b.json` does.
    #[default]
    PerFile,
    /// Collect every root node into an array and bind it to `.`.
    Slurp,
    /// Stitch every file's children into one virtual root, run once.
    Merge,
}
