//! Parsing for `:` command lines: line ranges, and `:s` and `:g` arguments.

mod global;
mod range;
mod substitute;

pub use global::{Global, parse_global};
pub use range::{LineRange, RangeContext, parse_range};
pub use substitute::{Flags, Substitute, parse_substitute};
