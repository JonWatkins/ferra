//! The pure text model: buffers, positions, motions, text objects, ranges and
//! undo history.
//!
//! Nothing in this module knows about terminals, rendering, modes or
//! configuration. It's the most stable layer and the one that plugins will
//! eventually build on.

mod buffer;
mod history;
pub mod motion;
mod position;
mod range;
pub mod text_object;

pub use buffer::Buffer;
pub use history::{Change, History};
pub use position::Position;
pub use range::{MotionKind, Range};
