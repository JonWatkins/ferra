//! Parsing for `:` command lines: line ranges, and `:s`, `:g` and `:sort`
//! arguments.

mod bar;
mod global;
mod range;
mod sort;
mod substitute;

pub use bar::{Bar, split_bar};
pub use global::{Global, parse_global};
pub use range::{LineRange, RangeContext, parse_range};
pub use sort::{SortOptions, parse_sort, sort_lines};
pub use substitute::{Flags, Substitute, parse_substitute};
