//! Native namespace relationships, independent of decoded display paths.

/// Parent of one ordinary namespace occurrence, scoped to its reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parent {
    /// The explicit filesystem root, which is not an indexed entry.
    Root,
    /// Index of a directory occurrence in the same reader's `entries()`.
    Entry(usize),
}
