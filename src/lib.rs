//! pixel-cast: stream pictures to LED matrices over E1.31 (sACN).
//!
//! The library holds everything the command line is built from, so that each part is tested on its own.

pub mod geometry;
pub mod sacn;
pub mod sender;
pub mod target;
