//! The Currents cover renderer: the two shader passes, the GPU setup they need on native and
//! browser backends, and the interactive viewer both platforms run.

mod camera;
pub mod gpu;
pub mod params;
pub mod renderer;
pub mod settings;
#[cfg(test)]
mod shader_check;
pub mod shaders;
mod view;
pub mod viewer;

pub use params::{PARAM_SLOTS, ParamBlock, ParamOverride};
pub use view::View;
