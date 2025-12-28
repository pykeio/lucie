//! The GPUI prelude is a collection of traits and types that are widely used
//! throughout the library. It is recommended to import this prelude into your
//! application to avoid having to import each trait individually.

pub use lucie_common::refineable::Refineable;
pub use lucie_style::Styled;

pub use crate::{
	AppContext as _, BorrowAppContext, Context, Element, FluentBuilder, InteractiveElement, IntoElement, ParentElement, Render, RenderOnce,
	StatefulInteractiveElement, StyledImage, VisualContext
};
