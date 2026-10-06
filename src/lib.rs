//! x2k: send long-form articles (starting with X Articles) to your Kindle.
//!
//! The binary in `main.rs` only parses arguments; everything else lives here.

pub mod app;
pub mod article;
pub mod config;
pub mod render;
pub mod sender;
pub mod source;
pub mod sources;
