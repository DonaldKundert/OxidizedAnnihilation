//! Pure parsers from bytes to typed structs, one module per format.
//!
//! Rules: take `&[u8]`, never touch the filesystem, never panic on bad input,
//! read through [`reader::Reader`], and validate counts before allocating.
//! Each module owns its error type and its docs in `docs/formats/`.

pub mod reader;

pub mod cob;
pub mod gaf;
pub mod gui;
pub mod model3do;
pub mod tdf;
pub mod tnt;
