//! The `lumenwright` command, over the `lumenwright` library's public API.
//!
//! The binary (`src/main.rs`) parses the arguments and sets up logging. This
//! library holds what it calls and what its tests reuse: [`job`] (the `render`
//! subcommand: flags → [`lumenwright::Request`] → [`lumenwright::render()`] →
//! image files), [`jar`] (finding or downloading the client jar), [`unifont`]
//! (the same for the Unifont that draws CJK text), [`output`]
//! (PNG and PFM writers), [`values`] (the flag values for the library's enums)
//! and [`fixture`] (the generated worlds of `demo-world` and the tests).

pub mod fixture;
pub mod jar;
pub mod job;
pub mod output;
pub mod unifont;
pub mod values;
