//! The pure text model: buffers, positions, motions, text objects, ranges,
//! undo history and on-disk text format.
//!
//! Nothing in this module knows about terminals, rendering, modes or
//! configuration. It's the most stable layer and the one that plugins will
//! eventually build on.

mod block;
mod buffer;
pub mod columns;
mod history;
pub mod increment;
pub mod indent;
pub mod motion;
mod position;
mod range;
pub mod search;
pub mod text_format;
pub mod text_object;
pub mod wrap;

pub use block::{Block, BlockRow};
pub use buffer::{Buffer, TextEdit};
pub use history::{Change, History, Revision};
pub use indent::IndentUnit;
pub use position::Position;
pub use range::{MotionKind, Range};
pub use text_format::{DEFAULT_FALLBACK, Encoding, FileEncoding, LineEnding, TextFormat};
