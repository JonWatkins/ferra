//! The pure text model: buffers, positions and motions.
//!
//! Nothing in this module knows about terminals, rendering, modes or
//! configuration. It's the most stable layer and the one that plugins will
//! eventually build on.

mod buffer;
pub mod motion;
mod position;

pub use buffer::Buffer;
pub use position::Position;
