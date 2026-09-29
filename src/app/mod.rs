//! GPUI views for the mail client.
//!
//! `gpui-kit` re-exports GPUI at its root, so `use gpui_kit::*` *is* GPUI.

pub mod actions;
pub mod chrome;
pub mod compose;
pub mod mail_app;
pub mod palette;
pub mod panels;
pub mod settings;
pub mod snooze;

pub use mail_app::MailApp;
