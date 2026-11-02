//! SWF translatable-text toolkit — future XIMOD Translator tool.
//!
//! Read a SWF, [`text::extract`] its translatable strings, and [`text::inject`]
//! edited translations back into a valid SWF. See [`swf`] for the conservative
//! container round-trip and [`text`] for the per-source coverage.

pub mod app;
pub mod as2;
pub mod exchange;
pub mod swf;
pub mod text;
