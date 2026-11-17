//! mdrv-oq's shared core: fetch and parse the Z.AI coding-plan quota,
//! keep one snapshot of it, decide when to alert, and render the numbers
//! every surface (desktop tray and window; the Android notification later)
//! shows. No UI dependencies; everything here is sync and unit-tested.

pub mod api;
pub mod key;
pub mod model;
pub mod render;
pub mod settings;
pub mod thresholds;
