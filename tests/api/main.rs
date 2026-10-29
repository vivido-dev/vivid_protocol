//! Tests that exercise only the public API, one module per source module.

mod anchor;
mod audio_input;
mod context;
mod geometry;
mod grant;
mod hid;
mod idempotency;
mod identity;
mod lease;
mod media;
mod messages;
mod observation;
mod overlay_wire_text;
mod overlay_wire_text_styled;
mod resource;
mod revision;
mod scene;
mod surface;
mod surface_desktop_parameter_tests;
mod target;
mod time;
#[cfg(any(feature = "native", feature = "native-transport"))]
mod trace;
mod track;
mod web;
