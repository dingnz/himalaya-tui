//! Color themes for the TUI: the resolved [`Theme`] struct used by
//! every render function, plus the built-in presets shipped as `const`
//! values. [`Theme::resolve`] layers per-field overrides from
//! [`crate::config::ThemeConfig`] on top of the chosen preset.

pub mod default;
pub mod dracula_dark;
pub mod one_light;
pub mod tokyo_night;
mod resolved;

pub use resolved::Theme;
