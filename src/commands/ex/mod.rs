//! Parsing for `:` command lines: line ranges, and `:s` arguments.

mod range;
mod substitute;

pub use range::{LineRange, RangeContext, parse_range};
pub use substitute::{Flags, Substitute, parse_substitute};
