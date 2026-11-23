//! Parsing for `:` command lines: line ranges, and `:s`, `:g` and `:sort`
//! arguments.
//!
//! `command_line_execute` parses `[range] name[!] [arguments]` (`commands/ex/`,
//! pure functions with their own tests). The range becomes `Invocation::range`
//! (0-based, inclusive rows). The text after the name is passed untouched as
//! `Invocation::arg_text`, as well as split into `args`, because `:s` patterns
//! can contain spaces. The command name stops at the first character that can't
//! be part of a name, so `s/a/b/` needs no space. A range with no command goes
//! to its last line, which is what makes `:42` work.

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
