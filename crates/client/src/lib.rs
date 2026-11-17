#![forbid(unsafe_code)]
//! Multi-platform client workspace (`core`, `tui`, `ui`) built on [`pulsar_styles`] and [`pulsar_protocol`].

pub mod core;
pub mod tui;
pub mod ui;

pub use pulsar_styles as styles;
