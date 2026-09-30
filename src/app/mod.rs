//! GPUI views for the mail client.
//!
//! `gpui-kit` re-exports GPUI at its root, so `use gpui_kit::*` *is* GPUI.

pub mod actions;
pub mod icons;
pub mod chrome;
pub mod compose;
pub mod dialog;
pub mod folder_picker;
pub mod mail_app;
pub mod menu;
pub mod overlay;
pub mod row;
pub mod palette;
pub mod panels;
pub mod settings;
pub mod snooze;
pub mod ui;

pub use mail_app::MailApp;
