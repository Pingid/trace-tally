#![cfg_attr(feature = "tracing", doc = include_str!("../README.md"))]

#[cfg(feature = "tracing")]
pub mod layer;
pub mod model;
pub mod render;

pub mod widgets;

#[cfg(test)]
mod test;

/// Re-exports of all public types and traits.
pub mod prelude {
    #[cfg(feature = "tracing")]
    pub use crate::layer::{ActionSink, InlineSink, TaskLayer, TraceMapper, TraceMapperExt};
    pub use crate::model::{Action, EventView, TaskId, TaskTree, TaskView};
    pub use crate::render::{
        ActionOf, ActionSource, Frame, LiveRegion, RenderLoop, Renderer, TaskRenderer, TreeOf,
    };
    pub use crate::widgets::*;
}

pub use crate::prelude::*;
