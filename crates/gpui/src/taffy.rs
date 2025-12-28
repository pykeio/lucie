use std::{fmt::Debug, ops::Range};

use lucie_common::geometry::{AbsoluteLength, Bounds, DefiniteLength, Edges, GridPlacement, Length, Pixels, Size, point, size};
use lucie_style::Style;
use rapidhash::fast::{RapidHashMap, RapidHashSet};
use taffy::{
	TaffyTree, TraversePartialTree as _,
	geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
	style::AvailableSpace as TaffyAvailableSpace,
	tree::NodeId
};

use crate::{App, Window};

type NodeMeasureFn = Box<dyn FnMut(Size<Option<Pixels>>, Size<AvailableSpace>, &mut Window, &mut App) -> Size<Pixels>>;

struct NodeContext {
	measure: NodeMeasureFn
}
pub struct TaffyLayoutEngine {
	taffy: TaffyTree<NodeContext>,
	absolute_layout_bounds: RapidHashMap<LayoutId, Bounds<Pixels>>,
	computed_layouts: RapidHashSet<LayoutId>,
	layout_bounds_scratch_space: Vec<LayoutId>
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl TaffyLayoutEngine {
	pub fn new() -> Self {
		let mut taffy = TaffyTree::new();
		taffy.enable_rounding();
		TaffyLayoutEngine {
			taffy,
			absolute_layout_bounds: RapidHashMap::default(),
			computed_layouts: RapidHashSet::default(),
			layout_bounds_scratch_space: Vec::new()
		}
	}

	pub fn clear(&mut self) {
		self.taffy.clear();
		self.absolute_layout_bounds.clear();
		self.computed_layouts.clear();
	}

	pub fn request_layout(&mut self, style: Style, rem_size: Pixels, scale_factor: f32, children: &[LayoutId]) -> LayoutId {
		let taffy_style = style.to_taffy(rem_size, scale_factor);

		if children.is_empty() {
			self.taffy.new_leaf(taffy_style).expect(EXPECT_MESSAGE).into()
		} else {
			self.taffy
                // This is safe because LayoutId is repr(transparent) to taffy::tree::NodeId.
                .new_with_children(taffy_style, LayoutId::to_taffy_slice(children))
                .expect(EXPECT_MESSAGE)
                .into()
		}
	}

	pub fn request_measured_layout(
		&mut self,
		style: Style,
		rem_size: Pixels,
		scale_factor: f32,
		measure: impl FnMut(Size<Option<Pixels>>, Size<AvailableSpace>, &mut Window, &mut App) -> Size<Pixels> + 'static
	) -> LayoutId {
		let taffy_style = style.to_taffy(rem_size, scale_factor);

		self.taffy
			.new_leaf_with_context(taffy_style, NodeContext { measure: Box::new(measure) })
			.expect(EXPECT_MESSAGE)
			.into()
	}

	// Used to understand performance
	#[allow(dead_code)]
	fn count_all_children(&self, parent: LayoutId) -> anyhow::Result<u32> {
		let mut count = 0;

		for child in self.taffy.children(parent.0)? {
			// Count this child.
			count += 1;

			// Count all of this child's children.
			count += self.count_all_children(LayoutId(child))?
		}

		Ok(count)
	}

	// Used to understand performance
	#[allow(dead_code)]
	fn max_depth(&self, depth: u32, parent: LayoutId) -> anyhow::Result<u32> {
		println!("{parent:?} at depth {depth} has {} children", self.taffy.child_count(parent.0));

		let mut max_child_depth = 0;

		for child in self.taffy.children(parent.0)? {
			max_child_depth = std::cmp::max(max_child_depth, self.max_depth(0, LayoutId(child))?);
		}

		Ok(depth + 1 + max_child_depth)
	}

	// Used to understand performance
	#[allow(dead_code)]
	fn get_edges(&self, parent: LayoutId) -> anyhow::Result<Vec<(LayoutId, LayoutId)>> {
		let mut edges = Vec::new();

		for child in self.taffy.children(parent.0)? {
			edges.push((parent, LayoutId(child)));

			edges.extend(self.get_edges(LayoutId(child))?);
		}

		Ok(edges)
	}

	pub fn compute_layout(&mut self, id: LayoutId, available_space: Size<AvailableSpace>, window: &mut Window, cx: &mut App) {
		// Leaving this here until we have a better instrumentation approach.
		// println!("Laying out {} children", self.count_all_children(id)?);
		// println!("Max layout depth: {}", self.max_depth(0, id)?);

		// Output the edges (branches) of the tree in Mermaid format for visualization.
		// println!("Edges:");
		// for (a, b) in self.get_edges(id)? {
		//     println!("N{} --> N{}", u64::from(a), u64::from(b));
		// }
		//

		if !self.computed_layouts.insert(id) {
			let mut stack = &mut self.layout_bounds_scratch_space;
			stack.push(id);
			while let Some(id) = stack.pop() {
				self.absolute_layout_bounds.remove(&id);
				stack.extend(self.taffy.children(id.into()).expect(EXPECT_MESSAGE).into_iter().map(LayoutId::from));
			}
		}

		let scale_factor = window.scale_factor();

		let transform = |v: AvailableSpace| match v {
			AvailableSpace::Definite(pixels) => AvailableSpace::Definite(Pixels(pixels.0 * scale_factor)),
			AvailableSpace::MinContent => AvailableSpace::MinContent,
			AvailableSpace::MaxContent => AvailableSpace::MaxContent
		};
		let available_space = size(transform(available_space.width), transform(available_space.height));

		self.taffy
			.compute_layout_with_measure(id.into(), size_to_taffy(available_space), |known_dimensions, available_space, _id, node_context, _style| {
				let Some(node_context) = node_context else {
					return taffy::geometry::Size::default();
				};

				let known_dimensions = Size {
					width: known_dimensions.width.map(|e| Pixels(e / scale_factor)),
					height: known_dimensions.height.map(|e| Pixels(e / scale_factor))
				};

				let available_space: Size<AvailableSpace> = size_from_taffy(available_space);
				let untransform = |ev: AvailableSpace| match ev {
					AvailableSpace::Definite(pixels) => AvailableSpace::Definite(Pixels(pixels.0 / scale_factor)),
					AvailableSpace::MinContent => AvailableSpace::MinContent,
					AvailableSpace::MaxContent => AvailableSpace::MaxContent
				};
				let available_space = size(untransform(available_space.width), untransform(available_space.height));

				let a: Size<Pixels> = (node_context.measure)(known_dimensions, available_space, window, cx);
				size_to_taffy(size(a.width.0 * scale_factor, a.height.0 * scale_factor))
			})
			.expect(EXPECT_MESSAGE);
	}

	pub fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
		if let Some(layout) = self.absolute_layout_bounds.get(&id).cloned() {
			return layout;
		}

		let layout = self.taffy.layout(id.into()).expect(EXPECT_MESSAGE);
		let mut bounds = Bounds {
			origin: point(Pixels(layout.location.x / scale_factor), Pixels(layout.location.y / scale_factor)),
			size: size(Pixels(layout.size.width / scale_factor), Pixels(layout.size.height / scale_factor))
		};

		if let Some(parent_id) = self.taffy.parent(id.0) {
			let parent_bounds = self.layout_bounds(parent_id.into(), scale_factor);
			bounds.origin += parent_bounds.origin;
		}
		self.absolute_layout_bounds.insert(id, bounds);

		bounds
	}
}

/// A unique identifier for a layout node, generated when requesting a layout from Taffy
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
#[repr(transparent)]
pub struct LayoutId(NodeId);

impl LayoutId {
	fn to_taffy_slice(node_ids: &[Self]) -> &[taffy::NodeId] {
		// SAFETY: LayoutId is repr(transparent) to taffy::tree::NodeId.
		unsafe { std::mem::transmute::<&[LayoutId], &[taffy::NodeId]>(node_ids) }
	}
}

impl std::hash::Hash for LayoutId {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		u64::from(self.0).hash(state);
	}
}

impl From<NodeId> for LayoutId {
	fn from(node_id: NodeId) -> Self {
		Self(node_id)
	}
}

impl From<LayoutId> for NodeId {
	fn from(layout_id: LayoutId) -> NodeId {
		layout_id.0
	}
}

trait ToTaffy<Output> {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> Output;
}

impl ToTaffy<taffy::style::Style> for Style {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Style {
		use taffy::style_helpers::{fr, length, minmax, repeat};

		fn to_grid_line(placement: &Range<GridPlacement>) -> taffy::Line<taffy::GridPlacement> {
			taffy::Line {
				start: grid_placement_to_taffy(placement.start),
				end: grid_placement_to_taffy(placement.end)
			}
		}

		fn to_grid_repeat<T: taffy::style::CheapCloneStr>(unit: &Option<u16>) -> Vec<taffy::GridTemplateComponent<T>> {
			// grid-template-columns: repeat(<number>, minmax(0, 1fr));
			unit.map(|count| vec![repeat(count, vec![minmax(length(0.0), fr(1.0))])])
				.unwrap_or_default()
		}

		taffy::style::Style {
			display: match self.display {
				lucie_style::Display::Block => taffy::style::Display::Block,
				lucie_style::Display::Flex => taffy::style::Display::Flex,
				lucie_style::Display::Grid => taffy::style::Display::Grid,
				lucie_style::Display::None => taffy::style::Display::None
			},
			overflow: TaffyPoint {
				x: overflow_to_taffy(self.overflow.x),
				y: overflow_to_taffy(self.overflow.y)
			},
			scrollbar_width: self.scrollbar_width.to_taffy(rem_size, scale_factor),
			position: match self.position {
				lucie_style::Position::Relative => taffy::style::Position::Relative,
				lucie_style::Position::Absolute => taffy::style::Position::Absolute
			},
			inset: self.inset.to_taffy(rem_size, scale_factor),
			size: self.size.to_taffy(rem_size, scale_factor),
			min_size: self.min_size.to_taffy(rem_size, scale_factor),
			max_size: self.max_size.to_taffy(rem_size, scale_factor),
			aspect_ratio: self.aspect_ratio,
			margin: self.margin.to_taffy(rem_size, scale_factor),
			padding: self.padding.to_taffy(rem_size, scale_factor),
			border: self.border_widths.to_taffy(rem_size, scale_factor),
			align_items: self.align_items.map(items_to_taffy),
			align_self: self.align_self.map(items_to_taffy),
			align_content: self.align_content.map(align_to_taffy),
			justify_content: self.justify_content.map(align_to_taffy),
			gap: self.gap.to_taffy(rem_size, scale_factor),
			flex_direction: match self.flex_direction {
				lucie_style::FlexDirection::Row => taffy::style::FlexDirection::Row,
				lucie_style::FlexDirection::Column => taffy::style::FlexDirection::Column,
				lucie_style::FlexDirection::RowReverse => taffy::style::FlexDirection::RowReverse,
				lucie_style::FlexDirection::ColumnReverse => taffy::style::FlexDirection::ColumnReverse
			},
			flex_wrap: match self.flex_wrap {
				lucie_style::FlexWrap::NoWrap => taffy::style::FlexWrap::NoWrap,
				lucie_style::FlexWrap::Wrap => taffy::style::FlexWrap::Wrap,
				lucie_style::FlexWrap::WrapReverse => taffy::style::FlexWrap::WrapReverse
			},
			flex_basis: self.flex_basis.to_taffy(rem_size, scale_factor),
			flex_grow: self.flex_grow,
			flex_shrink: self.flex_shrink,
			grid_template_rows: to_grid_repeat(&self.grid_rows),
			grid_template_columns: to_grid_repeat(&self.grid_cols),
			grid_row: self
				.grid_location
				.as_ref()
				.map(|location| to_grid_line(&location.row))
				.unwrap_or_default(),
			grid_column: self
				.grid_location
				.as_ref()
				.map(|location| to_grid_line(&location.column))
				.unwrap_or_default(),
			..Default::default()
		}
	}
}

#[inline]
fn overflow_to_taffy(overflow: lucie_style::Overflow) -> taffy::style::Overflow {
	match overflow {
		lucie_style::Overflow::Visible => taffy::style::Overflow::Visible,
		lucie_style::Overflow::Clip => taffy::style::Overflow::Clip,
		lucie_style::Overflow::Hidden => taffy::style::Overflow::Hidden,
		lucie_style::Overflow::Scroll => taffy::style::Overflow::Scroll
	}
}

#[inline]
fn align_to_taffy(align: lucie_style::AlignContent) -> taffy::style::AlignContent {
	match align {
		lucie_style::AlignContent::Start => taffy::style::AlignContent::Start,
		lucie_style::AlignContent::End => taffy::style::AlignContent::End,
		lucie_style::AlignContent::FlexStart => taffy::style::AlignContent::FlexStart,
		lucie_style::AlignContent::FlexEnd => taffy::style::AlignContent::FlexEnd,
		lucie_style::AlignContent::Center => taffy::style::AlignContent::Center,
		lucie_style::AlignContent::Stretch => taffy::style::AlignContent::Stretch,
		lucie_style::AlignContent::SpaceBetween => taffy::style::AlignContent::SpaceBetween,
		lucie_style::AlignContent::SpaceEvenly => taffy::style::AlignContent::SpaceEvenly,
		lucie_style::AlignContent::SpaceAround => taffy::style::AlignContent::SpaceAround
	}
}

#[inline]
fn items_to_taffy(items: lucie_style::AlignItems) -> taffy::style::AlignItems {
	match items {
		lucie_style::AlignItems::Start => taffy::style::AlignItems::Start,
		lucie_style::AlignItems::End => taffy::style::AlignItems::End,
		lucie_style::AlignItems::FlexStart => taffy::style::AlignItems::FlexStart,
		lucie_style::AlignItems::FlexEnd => taffy::style::AlignItems::FlexEnd,
		lucie_style::AlignItems::Center => taffy::style::AlignItems::Center,
		lucie_style::AlignItems::Baseline => taffy::style::AlignItems::Baseline,
		lucie_style::AlignItems::Stretch => taffy::style::AlignItems::Stretch
	}
}

impl ToTaffy<f32> for AbsoluteLength {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> f32 {
		match self {
			AbsoluteLength::Pixels(pixels) => {
				let pixels: f32 = pixels.into();
				pixels * scale_factor
			}
			AbsoluteLength::Rems(rems) => {
				let pixels: f32 = (*rems * rem_size).into();
				pixels * scale_factor
			}
		}
	}
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for Length {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::prelude::LengthPercentageAuto {
		match self {
			Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
			Length::Auto => taffy::prelude::LengthPercentageAuto::auto()
		}
	}
}

impl ToTaffy<taffy::style::Dimension> for Length {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::prelude::Dimension {
		match self {
			Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
			Length::Auto => taffy::prelude::Dimension::auto()
		}
	}
}

impl ToTaffy<taffy::style::LengthPercentage> for DefiniteLength {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
		match self {
			DefiniteLength::Absolute(length) => match length {
				AbsoluteLength::Pixels(pixels) => {
					let pixels: f32 = pixels.into();
					taffy::style::LengthPercentage::length(pixels * scale_factor)
				}
				AbsoluteLength::Rems(rems) => {
					let pixels: f32 = (*rems * rem_size).into();
					taffy::style::LengthPercentage::length(pixels * scale_factor)
				}
			},
			DefiniteLength::Fraction(fraction) => taffy::style::LengthPercentage::percent(*fraction)
		}
	}
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for DefiniteLength {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
		match self {
			DefiniteLength::Absolute(length) => match length {
				AbsoluteLength::Pixels(pixels) => {
					let pixels: f32 = pixels.into();
					taffy::style::LengthPercentageAuto::length(pixels * scale_factor)
				}
				AbsoluteLength::Rems(rems) => {
					let pixels: f32 = (*rems * rem_size).into();
					taffy::style::LengthPercentageAuto::length(pixels * scale_factor)
				}
			},
			DefiniteLength::Fraction(fraction) => taffy::style::LengthPercentageAuto::percent(*fraction)
		}
	}
}

impl ToTaffy<taffy::style::Dimension> for DefiniteLength {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
		match self {
			DefiniteLength::Absolute(length) => match length {
				AbsoluteLength::Pixels(pixels) => {
					let pixels: f32 = pixels.into();
					taffy::style::Dimension::length(pixels * scale_factor)
				}
				AbsoluteLength::Rems(rems) => taffy::style::Dimension::length((*rems * rem_size * scale_factor).into())
			},
			DefiniteLength::Fraction(fraction) => taffy::style::Dimension::percent(*fraction)
		}
	}
}

impl ToTaffy<taffy::style::LengthPercentage> for AbsoluteLength {
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
		match self {
			AbsoluteLength::Pixels(pixels) => {
				let pixels: f32 = pixels.into();
				taffy::style::LengthPercentage::length(pixels * scale_factor)
			}
			AbsoluteLength::Rems(rems) => {
				let pixels: f32 = (*rems * rem_size).into();
				taffy::style::LengthPercentage::length(pixels * scale_factor)
			}
		}
	}
}

impl<T, U> ToTaffy<TaffySize<U>> for Size<T>
where
	T: ToTaffy<U> + Clone + Debug + Default + PartialEq
{
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffySize<U> {
		TaffySize {
			width: self.width.to_taffy(rem_size, scale_factor),
			height: self.height.to_taffy(rem_size, scale_factor)
		}
	}
}

impl<T, U> ToTaffy<TaffyRect<U>> for Edges<T>
where
	T: ToTaffy<U> + Clone + Debug + Default + PartialEq
{
	fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffyRect<U> {
		TaffyRect {
			top: self.top.to_taffy(rem_size, scale_factor),
			right: self.right.to_taffy(rem_size, scale_factor),
			bottom: self.bottom.to_taffy(rem_size, scale_factor),
			left: self.left.to_taffy(rem_size, scale_factor)
		}
	}
}

fn size_from_taffy<T, U>(size: TaffySize<T>) -> Size<U>
where
	T: Into<U>,
	U: Clone + Debug + Default + PartialEq
{
	Size {
		width: size.width.into(),
		height: size.height.into()
	}
}

fn size_to_taffy<T, U>(size: Size<T>) -> TaffySize<U>
where
	T: Into<U> + Clone + Debug + Default + PartialEq
{
	TaffySize {
		width: size.width.into(),
		height: size.height.into()
	}
}

fn grid_placement_to_taffy(placement: GridPlacement) -> taffy::GridPlacement {
	match placement {
		GridPlacement::Auto => taffy::GridPlacement::Auto,
		GridPlacement::Line(l) => taffy::GridPlacement::Line(l.into()),
		GridPlacement::Span(s) => taffy::GridPlacement::Span(s)
	}
}

/// The space available for an element to be laid out in
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
pub enum AvailableSpace {
	/// The amount of space available is the specified number of pixels
	Definite(Pixels),
	/// The amount of space available is indefinite and the node should be laid out under a min-content constraint
	#[default]
	MinContent,
	/// The amount of space available is indefinite and the node should be laid out under a max-content constraint
	MaxContent
}

impl AvailableSpace {
	/// Returns a `Size` with both width and height set to `AvailableSpace::MinContent`.
	///
	/// This function is useful when you want to create a `Size` with the minimum content constraints
	/// for both dimensions.
	///
	/// # Examples
	///
	/// ```
	/// use gpui::AvailableSpace;
	/// let min_content_size = AvailableSpace::min_size();
	/// assert_eq!(min_content_size.width, AvailableSpace::MinContent);
	/// assert_eq!(min_content_size.height, AvailableSpace::MinContent);
	/// ```
	pub const fn min_size() -> Size<Self> {
		Size {
			width: Self::MinContent,
			height: Self::MinContent
		}
	}

	pub fn from_definite(size: Size<Pixels>) -> Size<AvailableSpace> {
		Size {
			width: AvailableSpace::Definite(size.width),
			height: AvailableSpace::Definite(size.height)
		}
	}
}

impl From<AvailableSpace> for TaffyAvailableSpace {
	fn from(space: AvailableSpace) -> TaffyAvailableSpace {
		match space {
			AvailableSpace::Definite(Pixels(value)) => TaffyAvailableSpace::Definite(value),
			AvailableSpace::MinContent => TaffyAvailableSpace::MinContent,
			AvailableSpace::MaxContent => TaffyAvailableSpace::MaxContent
		}
	}
}

impl From<TaffyAvailableSpace> for AvailableSpace {
	fn from(space: TaffyAvailableSpace) -> AvailableSpace {
		match space {
			TaffyAvailableSpace::Definite(value) => AvailableSpace::Definite(Pixels(value)),
			TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
			TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent
		}
	}
}

impl From<Pixels> for AvailableSpace {
	fn from(pixels: Pixels) -> Self {
		AvailableSpace::Definite(pixels)
	}
}
