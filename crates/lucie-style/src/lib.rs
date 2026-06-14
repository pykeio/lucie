use std::{
	hash::{Hash, Hasher},
	iter, mem,
	num::NonZeroU32,
	ops::Range
};

use lucie_common::{
	SharedString,
	color::{Background, Hsla, Rgba, black},
	geometry::{
		AbsoluteLength, Bounds, Corners, CornersRefinement, DefiniteLength, DevicePixels, Edges, EdgesRefinement, GridLocation, Length, Pixels, Point,
		PointRefinement, Size, SizeRefinement, phi, point, rems, size
	},
	refineable::Refineable
};
use smallvec::SmallVec;

mod text;
pub use self::text::*;
mod styled;
pub use self::styled::Styled;

/// How to fit the image into the bounds of the element.
pub enum ObjectFit {
	/// The image will be stretched to fill the bounds of the element.
	Fill,
	/// The image will be scaled to fit within the bounds of the element.
	Contain,
	/// The image will be scaled to cover the bounds of the element.
	Cover,
	/// The image will be scaled down to fit within the bounds of the element.
	ScaleDown,
	/// The image will maintain its original size.
	None
}

impl ObjectFit {
	/// Get the bounds of the image within the given bounds.
	pub fn get_bounds(&self, bounds: Bounds<Pixels>, image_size: Size<DevicePixels>) -> Bounds<Pixels> {
		let image_size = image_size.map(|dimension| Pixels::from(u32::from(dimension)));
		let image_ratio = image_size.width / image_size.height;
		let bounds_ratio = bounds.size.width / bounds.size.height;

		match self {
			ObjectFit::Fill => bounds,
			ObjectFit::Contain => {
				let new_size = if bounds_ratio > image_ratio {
					size(image_size.width * (bounds.size.height / image_size.height), bounds.size.height)
				} else {
					size(bounds.size.width, image_size.height * (bounds.size.width / image_size.width))
				};

				Bounds {
					origin: point(bounds.origin.x + (bounds.size.width - new_size.width) / 2.0, bounds.origin.y + (bounds.size.height - new_size.height) / 2.0),
					size: new_size
				}
			}
			ObjectFit::ScaleDown => {
				// Check if the image is larger than the bounds in either dimension.
				if image_size.width > bounds.size.width || image_size.height > bounds.size.height {
					// If the image is larger, use the same logic as Contain to scale it down.
					let new_size = if bounds_ratio > image_ratio {
						size(image_size.width * (bounds.size.height / image_size.height), bounds.size.height)
					} else {
						size(bounds.size.width, image_size.height * (bounds.size.width / image_size.width))
					};

					Bounds {
						origin: point(
							bounds.origin.x + (bounds.size.width - new_size.width) / 2.0,
							bounds.origin.y + (bounds.size.height - new_size.height) / 2.0
						),
						size: new_size
					}
				} else {
					// If the image is smaller than or equal to the container, display it at its original size,
					// centered within the container.
					let original_size = size(image_size.width, image_size.height);
					Bounds {
						origin: point(
							bounds.origin.x + (bounds.size.width - original_size.width) / 2.0,
							bounds.origin.y + (bounds.size.height - original_size.height) / 2.0
						),
						size: original_size
					}
				}
			}
			ObjectFit::Cover => {
				let new_size = if bounds_ratio > image_ratio {
					size(bounds.size.width, image_size.height * (bounds.size.width / image_size.width))
				} else {
					size(image_size.width * (bounds.size.height / image_size.height), bounds.size.height)
				};

				Bounds {
					origin: point(bounds.origin.x + (bounds.size.width - new_size.width) / 2.0, bounds.origin.y + (bounds.size.height - new_size.height) / 2.0),
					size: new_size
				}
			}
			ObjectFit::None => Bounds {
				origin: bounds.origin,
				size: image_size
			}
		}
	}
}

/// The CSS styling that can be applied to an element via the `Styled` trait
#[derive(Clone, Refineable, Debug)]
#[refineable(Debug, PartialEq)]
pub struct Style {
	/// What layout strategy should be used?
	pub display: Display,

	/// Should the element be painted on screen?
	pub visibility: Visibility,

	// Overflow properties
	/// How children overflowing their container should affect layout
	#[refineable]
	pub overflow: Point<Overflow>,
	/// How much space (in points) should be reserved for the scrollbars of `Overflow::Scroll` and `Overflow::Auto`
	/// nodes.
	pub scrollbar_width: AbsoluteLength,
	/// Whether both x and y axis should be scrollable at the same time.
	pub allow_concurrent_scroll: bool,
	/// Whether scrolling should be restricted to the axis indicated by the mouse wheel.
	///
	/// This means that:
	/// - The mouse wheel alone will only ever scroll the Y axis.
	/// - Holding `Shift` and using the mouse wheel will scroll the X axis.
	///
	/// ## Motivation
	///
	/// On the web when scrolling with the mouse wheel, scrolling up and down will always scroll the Y axis, even when
	/// the mouse is over a horizontally-scrollable element.
	///
	/// The only way to scroll horizontally is to hold down `Shift` while scrolling, which then changes the scroll axis
	/// to the X axis.
	///
	/// Currently, Lucie operates differently from the web in that it will scroll an element in either the X or Y axis
	/// when scrolling with just the mouse wheel. This causes problems when scrolling in a vertical list that contains
	/// horizontally-scrollable elements, as when you get to the horizontally-scrollable elements the scroll will be
	/// hijacked.
	///
	/// Ideally we would match the web's behavior and not have a need for this, but right now we're adding this opt-in
	/// style property to limit the potential blast radius.
	pub restrict_scroll_to_axis: bool,

	// Position properties
	/// What should the `position` value of this struct use as a base offset?
	pub position: Position,
	/// How should the position of this element be tweaked relative to the layout defined?
	#[refineable]
	pub inset: Edges<Length>,

	// Size properties
	/// Sets the initial size of the item
	#[refineable]
	pub size: Size<Length>,
	/// Controls the minimum size of the item
	#[refineable]
	pub min_size: Size<Length>,
	/// Controls the maximum size of the item
	#[refineable]
	pub max_size: Size<Length>,
	/// Sets the preferred aspect ratio for the item. The ratio is calculated as width divided by height.
	pub aspect_ratio: Option<f32>,

	// Spacing Properties
	/// How large should the margin be on each side?
	#[refineable]
	pub margin: Edges<Length>,
	/// How large should the padding be on each side?
	#[refineable]
	pub padding: Edges<DefiniteLength>,
	/// How large should the border be on each side?
	#[refineable]
	pub border_widths: Edges<AbsoluteLength>,

	// Alignment properties
	/// How this node's children aligned in the cross/block axis?
	pub align_items: Option<AlignItems>,
	/// How this node should be aligned in the cross/block axis. Falls back to the parents [`AlignItems`] if not set
	pub align_self: Option<AlignSelf>,
	/// How should content contained within this item be aligned in the cross/block axis
	pub align_content: Option<AlignContent>,
	/// How should contained within this item be aligned in the main/inline axis
	pub justify_content: Option<JustifyContent>,
	/// How large should the gaps between items in a flex container be?
	#[refineable]
	pub gap: Size<DefiniteLength>,

	// Flexbox properties
	/// Which direction does the main axis flow in?
	pub flex_direction: FlexDirection,
	/// Should elements wrap, or stay in a single line?
	pub flex_wrap: FlexWrap,
	/// Sets the initial main axis size of the item
	pub flex_basis: Length,
	/// The relative rate at which this item grows when it is expanding to fill space, 0.0 is the default value, and
	/// this value must be positive.
	pub flex_grow: f32,
	/// The relative rate at which this item shrinks when it is contracting to fit into space, 1.0 is the default value,
	/// and this value must be positive.
	pub flex_shrink: f32,

	/// The fill color of this element
	pub background: Option<Fill>,

	/// The border color of this element
	pub border_color: Option<Hsla>,

	/// The border style of this element
	pub border_style: BorderStyle,

	/// The radius of the corners of this element
	#[refineable]
	pub corner_radii: Corners<AbsoluteLength>,

	/// Box shadow of the element
	pub box_shadow: SmallVec<[BoxShadow; 2]>,

	/// The text style of this element
	#[refineable]
	pub text: TextStyleRefinement,

	/// The mouse cursor style shown when the mouse pointer is over an element.
	pub mouse_cursor: Option<CursorStyle>,

	/// The opacity of this element
	pub opacity: Option<f32>,

	/// The grid columns of this element
	/// Equivalent to the Tailwind `grid-cols-<number>`
	pub grid_cols: Option<u16>,

	/// The grid columns with min-content minimum sizing.
	/// Unlike grid_cols, it won't shrink to width 0 in AvailableSpace::MinContent constraints.
	pub grid_cols_min_content: Option<u16>,

	/// The row span of this element
	/// Equivalent to the Tailwind `grid-rows-<number>`
	pub grid_rows: Option<u16>,

	/// The grid location of this element
	pub grid_location: Option<GridLocation>,

	/// Whether to draw a red debugging outline around this element
	#[cfg(debug_assertions)]
	pub debug: bool,

	/// Whether to draw a red debugging outline around this element and all of its conforming children
	#[cfg(debug_assertions)]
	pub debug_below: bool
}

impl Styled for StyleRefinement {
	fn style(&mut self) -> &mut StyleRefinement {
		self
	}
}

impl StyleRefinement {
	/// The grid location of this element
	pub fn grid_location_mut(&mut self) -> &mut GridLocation {
		self.grid_location.get_or_insert_default()
	}
}

/// The value of the visibility property, similar to the CSS property `visibility`
#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
	/// The element should be drawn as normal.
	#[default]
	Visible,
	/// The element should not be drawn, but should still take up space in the layout.
	Hidden
}

/// The possible values of the box-shadow property
#[derive(Clone, Debug, PartialEq)]
pub struct BoxShadow {
	/// What color should the shadow have?
	pub color: Hsla,
	/// How should it be offset from its element?
	pub offset: Point<Pixels>,
	/// How much should the shadow be blurred?
	pub blur_radius: Pixels,
	/// How much should the shadow spread?
	pub spread_radius: Pixels
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BorderStyle {
	/// A solid border.
	#[default]
	Solid = 0,
	/// A dashed border.
	Dashed = 1
}

/// How to handle whitespace in text
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum WhiteSpace {
	/// Normal line wrapping when text overflows the width of the element
	#[default]
	Normal,
	/// No line wrapping, text will overflow the width of the element
	Nowrap
}

/// How to truncate text that overflows the width of the element
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextOverflow {
	/// Truncate the text at the end when it doesn't fit, and represent this truncation by displaying the
	/// provided string ("hello wor...")
	Truncate(SharedString),
	/// Truncate the text at the start when it doesn't fit, and represent this truncation by displaying the
	/// provided string ("...llo world")
	TruncateStart(SharedString)
}

/// How to align text within the element
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
	/// Align the text to the left of the element
	#[default]
	Left,

	/// Center the text within the element
	Center,

	/// Align the text to the right of the element
	Right
}

/// The properties that can be used to style text
#[derive(Refineable, Clone, Debug, PartialEq)]
#[refineable(Debug, PartialEq)]
pub struct TextStyle {
	/// The color of the text
	pub color: Hsla,

	/// The font family to use
	pub font_family: SharedString,

	/// The font features to use
	pub font_features: FontFeatures,

	/// The fallback fonts to use
	pub font_fallbacks: Option<FontFallbacks>,

	/// The font size to use, in pixels or rems.
	pub font_size: AbsoluteLength,

	/// The line height to use, in pixels or fractions
	pub line_height: DefiniteLength,

	/// The font weight, e.g. bold
	pub font_weight: FontWeight,

	/// The font style, e.g. italic
	pub font_style: FontStyle,

	/// The background color of the text
	pub background_color: Option<Hsla>,

	/// The underline style of the text
	pub underline: Option<UnderlineStyle>,

	/// The strikethrough style of the text
	pub strikethrough: Option<StrikethroughStyle>,

	/// How to handle whitespace in the text
	pub white_space: WhiteSpace,

	/// The text should be truncated if it overflows the width of the element
	pub text_overflow: Option<TextOverflow>,

	/// How the text should be aligned within the element
	pub text_align: TextAlign,

	/// The number of lines to display before truncating the text
	pub line_clamp: Option<NonZeroU32>
}

impl Default for TextStyle {
	fn default() -> Self {
		TextStyle {
			color: black(),
			font_family: "system-ui".into(),
			font_features: FontFeatures::default(),
			font_fallbacks: None,
			font_size: rems(1.).into(),
			line_height: phi(),
			font_weight: FontWeight::default(),
			font_style: FontStyle::default(),
			background_color: None,
			underline: None,
			strikethrough: None,
			white_space: WhiteSpace::Normal,
			text_overflow: None,
			text_align: TextAlign::default(),
			line_clamp: None
		}
	}
}

impl TextStyle {
	/// Create a new text style with the given highlighting applied.
	pub fn highlight(mut self, style: impl Into<HighlightStyle>) -> Self {
		let style = style.into();
		if let Some(weight) = style.font_weight {
			self.font_weight = weight;
		}
		if let Some(style) = style.font_style {
			self.font_style = style;
		}

		if let Some(color) = style.color {
			self.color = self.color.blend(color);
		}

		if let Some(factor) = style.fade_out {
			self.color.fade_out(factor);
		}

		if let Some(background_color) = style.background_color {
			self.background_color = Some(background_color);
		}

		if let Some(underline) = style.underline {
			self.underline = Some(underline);
		}

		if let Some(strikethrough) = style.strikethrough {
			self.strikethrough = Some(strikethrough);
		}

		self
	}

	/// Get the font configured for this text style.
	pub fn font(&self) -> Font {
		Font {
			family: self.font_family.clone(),
			features: self.font_features.clone(),
			fallbacks: self.font_fallbacks.clone(),
			weight: self.font_weight,
			style: self.font_style
		}
	}

	/// Returns the rounded line height in pixels.
	pub fn line_height_in_pixels(&self, rem_size: Pixels) -> Pixels {
		self.line_height.to_pixels(self.font_size, rem_size).round()
	}

	/// Convert this text style into a [`TextRun`], for the given length of the text.
	pub fn to_run(&self, len: usize) -> TextRun {
		TextRun {
			len,
			font: Font {
				family: self.font_family.clone(),
				features: self.font_features.clone(),
				fallbacks: self.font_fallbacks.clone(),
				weight: self.font_weight,
				style: self.font_style
			},
			font_size: self.font_size,
			color: self.color,
			background_color: self.background_color,
			underline: self.underline,
			strikethrough: self.strikethrough
		}
	}
}

/// The style of the cursor (pointer)
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub enum CursorStyle {
	/// The default cursor
	#[default]
	Arrow,

	/// A text input cursor
	/// corresponds to the CSS cursor value `text`
	IBeam,

	/// A crosshair cursor
	/// corresponds to the CSS cursor value `crosshair`
	Crosshair,

	/// A closed hand cursor
	/// corresponds to the CSS cursor value `grabbing`
	ClosedHand,

	/// An open hand cursor
	/// corresponds to the CSS cursor value `grab`
	OpenHand,

	/// A pointing hand cursor
	/// corresponds to the CSS cursor value `pointer`
	PointingHand,

	/// A resize left cursor
	/// corresponds to the CSS cursor value `w-resize`
	ResizeLeft,

	/// A resize right cursor
	/// corresponds to the CSS cursor value `e-resize`
	ResizeRight,

	/// A resize cursor to the left and right
	/// corresponds to the CSS cursor value `ew-resize`
	ResizeLeftRight,

	/// A resize up cursor
	/// corresponds to the CSS cursor value `n-resize`
	ResizeUp,

	/// A resize down cursor
	/// corresponds to the CSS cursor value `s-resize`
	ResizeDown,

	/// A resize cursor directing up and down
	/// corresponds to the CSS cursor value `ns-resize`
	ResizeUpDown,

	/// A resize cursor directing up-left and down-right
	/// corresponds to the CSS cursor value `nesw-resize`
	ResizeUpLeftDownRight,

	/// A resize cursor directing up-right and down-left
	/// corresponds to the CSS cursor value `nwse-resize`
	ResizeUpRightDownLeft,

	/// A cursor indicating that the item/column can be resized horizontally.
	/// corresponds to the CSS cursor value `col-resize`
	ResizeColumn,

	/// A cursor indicating that the item/row can be resized vertically.
	/// corresponds to the CSS cursor value `row-resize`
	ResizeRow,

	/// A text input cursor for vertical layout
	/// corresponds to the CSS cursor value `vertical-text`
	IBeamCursorForVerticalLayout,

	/// A cursor indicating that the operation is not allowed
	/// corresponds to the CSS cursor value `not-allowed`
	OperationNotAllowed,

	/// A cursor indicating that the operation will result in a link
	/// corresponds to the CSS cursor value `alias`
	DragLink,

	/// A cursor indicating that the operation will result in a copy
	/// corresponds to the CSS cursor value `copy`
	DragCopy,

	/// A cursor indicating that the operation will result in a context menu
	/// corresponds to the CSS cursor value `context-menu`
	ContextualMenu,

	/// Hide the cursor
	None
}

/// A highlight style to apply, similar to a `TextStyle` except
/// for a single font, uniformly sized and spaced text.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct HighlightStyle {
	/// The color of the text
	pub color: Option<Hsla>,

	/// The font weight, e.g. bold
	pub font_weight: Option<FontWeight>,

	/// The font style, e.g. italic
	pub font_style: Option<FontStyle>,

	/// The background color of the text
	pub background_color: Option<Hsla>,

	/// The underline style of the text
	pub underline: Option<UnderlineStyle>,

	/// The underline style of the text
	pub strikethrough: Option<StrikethroughStyle>,

	/// Similar to the CSS `opacity` property, this will cause the text to be less vibrant.
	pub fade_out: Option<f32>
}

impl Eq for HighlightStyle {}

impl Hash for HighlightStyle {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.color.hash(state);
		self.font_weight.hash(state);
		self.font_style.hash(state);
		self.background_color.hash(state);
		self.underline.hash(state);
		self.strikethrough.hash(state);
		state.write_u32(u32::from_be_bytes(self.fade_out.map(|f| f.to_be_bytes()).unwrap_or_default()));
	}
}

impl Style {
	/// Returns true if the style is visible and the background is opaque.
	pub fn has_opaque_background(&self) -> bool {
		self.background
			.as_ref()
			.is_some_and(|fill| fill.color().is_some_and(|color| !color.is_transparent()))
	}

	/// Get the text style in this element style.
	pub fn text_style(&self) -> Option<&TextStyleRefinement> {
		if self.text.is_some() { Some(&self.text) } else { None }
	}
}

impl Default for Style {
	fn default() -> Self {
		Style {
			display: Display::Block,
			visibility: Visibility::Visible,
			overflow: Point {
				x: Overflow::Visible,
				y: Overflow::Visible
			},
			allow_concurrent_scroll: false,
			restrict_scroll_to_axis: false,
			scrollbar_width: AbsoluteLength::default(),
			position: Position::Relative,
			inset: Edges::auto(),
			margin: Edges::<Length>::zero(),
			padding: Edges::<DefiniteLength>::zero(),
			border_widths: Edges::<AbsoluteLength>::zero(),
			size: Size::auto(),
			min_size: Size::auto(),
			max_size: Size::auto(),
			aspect_ratio: None,
			gap: Size::default(),
			// Alignment
			align_items: None,
			align_self: None,
			align_content: None,
			justify_content: None,
			// Flexbox
			flex_direction: FlexDirection::Row,
			flex_wrap: FlexWrap::NoWrap,
			flex_grow: 0.0,
			flex_shrink: 1.0,
			flex_basis: Length::Auto,
			background: None,
			border_color: None,
			border_style: BorderStyle::default(),
			corner_radii: Corners::default(),
			box_shadow: Default::default(),
			text: TextStyleRefinement::default(),
			mouse_cursor: None,
			opacity: None,
			grid_rows: None,
			grid_cols: None,
			grid_cols_min_content: None,
			grid_location: None,

			#[cfg(debug_assertions)]
			debug: false,
			#[cfg(debug_assertions)]
			debug_below: false
		}
	}
}

/// The properties that can be applied to an underline.
#[derive(Refineable, Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct UnderlineStyle {
	/// The thickness of the underline.
	pub thickness: Pixels,

	/// The color of the underline.
	pub color: Option<Hsla>,

	/// Whether the underline should be wavy, like in a spell checker.
	pub wavy: bool
}

/// The properties that can be applied to a strikethrough.
#[derive(Refineable, Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct StrikethroughStyle {
	/// The thickness of the strikethrough.
	pub thickness: Pixels,

	/// The color of the strikethrough.
	pub color: Option<Hsla>
}

/// The kinds of fill that can be applied to a shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
	/// A solid color fill.
	Color(Background)
}

impl Fill {
	/// Unwrap this fill into a solid color, if it is one.
	///
	/// If the fill is not a solid color, this method returns `None`.
	pub fn color(&self) -> Option<Background> {
		match self {
			Fill::Color(color) => Some(*color)
		}
	}
}

impl Default for Fill {
	fn default() -> Self {
		Self::Color(Background::default())
	}
}

impl From<Hsla> for Fill {
	fn from(color: Hsla) -> Self {
		Self::Color(color.into())
	}
}

impl From<Rgba> for Fill {
	fn from(color: Rgba) -> Self {
		Self::Color(color.into())
	}
}

impl From<Background> for Fill {
	fn from(background: Background) -> Self {
		Self::Color(background)
	}
}

impl From<TextStyle> for HighlightStyle {
	fn from(other: TextStyle) -> Self {
		Self::from(&other)
	}
}

impl From<&TextStyle> for HighlightStyle {
	fn from(other: &TextStyle) -> Self {
		Self {
			color: Some(other.color),
			font_weight: Some(other.font_weight),
			font_style: Some(other.font_style),
			background_color: other.background_color,
			underline: other.underline,
			strikethrough: other.strikethrough,
			fade_out: None
		}
	}
}

impl HighlightStyle {
	/// Create a highlight style with just a color
	pub fn color(color: Hsla) -> Self {
		Self {
			color: Some(color),
			..Default::default()
		}
	}
	/// Blend this highlight style with another.
	/// Non-continuous properties, like font_weight and font_style, are overwritten.
	#[must_use]
	pub fn highlight(self, other: HighlightStyle) -> Self {
		Self {
			color: other
				.color
				.map(|other_color| if let Some(color) = self.color { color.blend(other_color) } else { other_color })
				.or(self.color),
			font_weight: other.font_weight.or(self.font_weight),
			font_style: other.font_style.or(self.font_style),
			background_color: other.background_color.or(self.background_color),
			underline: other.underline.or(self.underline),
			strikethrough: other.strikethrough.or(self.strikethrough),
			fade_out: other
				.fade_out
				.map(|source_fade| {
					self.fade_out
						.map(|dest_fade| (dest_fade * (1. + source_fade)).clamp(0., 1.))
						.unwrap_or(source_fade)
				})
				.or(self.fade_out)
		}
	}
}

impl From<Hsla> for HighlightStyle {
	fn from(color: Hsla) -> Self {
		Self {
			color: Some(color),
			..Default::default()
		}
	}
}

impl From<FontWeight> for HighlightStyle {
	fn from(font_weight: FontWeight) -> Self {
		Self {
			font_weight: Some(font_weight),
			..Default::default()
		}
	}
}

impl From<FontStyle> for HighlightStyle {
	fn from(font_style: FontStyle) -> Self {
		Self {
			font_style: Some(font_style),
			..Default::default()
		}
	}
}

impl From<Rgba> for HighlightStyle {
	fn from(color: Rgba) -> Self {
		Self {
			color: Some(color.into()),
			..Default::default()
		}
	}
}

/// Combine and merge the highlights and ranges in the two iterators.
pub fn combine_highlights(
	a: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
	b: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>
) -> impl Iterator<Item = (Range<usize>, HighlightStyle)> {
	let mut endpoints = Vec::new();
	let mut highlights = Vec::new();
	for (range, highlight) in a.into_iter().chain(b) {
		if !range.is_empty() {
			let highlight_id = highlights.len();
			endpoints.push((range.start, highlight_id, true));
			endpoints.push((range.end, highlight_id, false));
			highlights.push(highlight);
		}
	}
	endpoints.sort_unstable_by_key(|(position, _, _)| *position);
	let mut endpoints = endpoints.into_iter().peekable();

	let mut styles_active = vec![false; highlights.len()];
	let mut ix = 0;
	iter::from_fn(move || {
		while let Some((endpoint_ix, highlight_id, is_start)) = endpoints.peek() {
			let prev_index = mem::replace(&mut ix, *endpoint_ix);
			if ix > prev_index && styles_active.iter().any(|x| *x) {
				let current_style = styles_active
					.iter()
					.enumerate()
					.filter_map(|(i, active)| active.then_some(i))
					.fold(HighlightStyle::default(), |acc, highlight_id| acc.highlight(highlights[highlight_id]));
				return Some((prev_index..ix, current_style));
			}

			styles_active[*highlight_id] = *is_start;
			endpoints.next();
		}
		None
	})
}

/// Used to control how child nodes are aligned.
/// For Flexbox it controls alignment in the cross axis
/// For Grid it controls alignment in the block axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/align-items)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AlignItems {
	/// Items are packed toward the start of the axis
	Start,
	/// Items are packed toward the end of the axis
	End,
	/// Items are packed towards the flex-relative start of the axis.
	///
	/// For flex containers with flex_direction RowReverse or ColumnReverse this is equivalent
	/// to End. In all other cases it is equivalent to Start.
	FlexStart,
	/// Items are packed towards the flex-relative end of the axis.
	///
	/// For flex containers with flex_direction RowReverse or ColumnReverse this is equivalent
	/// to Start. In all other cases it is equivalent to End.
	FlexEnd,
	/// Items are packed along the center of the cross axis
	Center,
	/// Items are aligned such as their baselines align
	Baseline,
	/// Stretch to fill the container
	Stretch
}
/// Used to control how child nodes are aligned.
/// Does not apply to Flexbox, and will be ignored if specified on a flex container
/// For Grid it controls alignment in the inline axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/justify-items)
pub type JustifyItems = AlignItems;
/// Used to control how the specified nodes is aligned.
/// Overrides the parent Node's `AlignItems` property.
/// For Flexbox it controls alignment in the cross axis
/// For Grid it controls alignment in the block axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/align-self)
pub type AlignSelf = AlignItems;
/// Used to control how the specified nodes is aligned.
/// Overrides the parent Node's `JustifyItems` property.
/// Does not apply to Flexbox, and will be ignored if specified on a flex child
/// For Grid it controls alignment in the inline axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/justify-self)
pub type JustifySelf = AlignItems;

/// Sets the distribution of space between and around content items
/// For Flexbox it controls alignment in the cross axis
/// For Grid it controls alignment in the block axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/align-content)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
// Copy of taffy::style type of the same name, to derive JsonSchema.
pub enum AlignContent {
	/// Items are packed toward the start of the axis
	Start,
	/// Items are packed toward the end of the axis
	End,
	/// Items are packed towards the flex-relative start of the axis.
	///
	/// For flex containers with flex_direction RowReverse or ColumnReverse this is equivalent
	/// to End. In all other cases it is equivalent to Start.
	FlexStart,
	/// Items are packed towards the flex-relative end of the axis.
	///
	/// For flex containers with flex_direction RowReverse or ColumnReverse this is equivalent
	/// to Start. In all other cases it is equivalent to End.
	FlexEnd,
	/// Items are centered around the middle of the axis
	Center,
	/// Items are stretched to fill the container
	Stretch,
	/// The first and last items are aligned flush with the edges of the container (no gap)
	/// The gap between items is distributed evenly.
	SpaceBetween,
	/// The gap between the first and last items is exactly THE SAME as the gap between items.
	/// The gaps are distributed evenly
	SpaceEvenly,
	/// The gap between the first and last items is exactly HALF the gap between items.
	/// The gaps are distributed evenly in proportion to these ratios.
	SpaceAround
}

/// Sets the distribution of space between and around content items
/// For Flexbox it controls alignment in the main axis
/// For Grid it controls alignment in the inline axis
///
/// [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/justify-content)
pub type JustifyContent = AlignContent;

/// Sets the layout used for the children of this node
///
/// The default values depends on on which feature flags are enabled. The order of precedence is: Flex, Grid, Block,
/// None.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Display {
	/// The children will follow the block layout algorithm
	Block,
	/// The children will follow the flexbox layout algorithm
	#[default]
	Flex,
	/// The children will follow the CSS Grid layout algorithm
	Grid,
	/// The children will not be laid out, and will follow absolute positioning
	None
}

/// Controls whether flex items are forced onto one line or can wrap onto multiple lines.
///
/// Defaults to [`FlexWrap::NoWrap`]
///
/// [Specification](https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property)
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum FlexWrap {
	/// Items will not wrap and stay on a single line
	#[default]
	NoWrap,
	/// Items will wrap according to this item's [`FlexDirection`]
	Wrap,
	/// Items will wrap in the opposite direction to this item's [`FlexDirection`]
	WrapReverse
}

/// The direction of the flexbox layout main axis.
///
/// There are always two perpendicular layout axes: main (or primary) and cross (or secondary).
/// Adding items will cause them to be positioned adjacent to each other along the main axis.
/// By varying this value throughout your tree, you can create complex axis-aligned layouts.
///
/// Items are always aligned relative to the cross axis, and justified relative to the main axis.
///
/// The default behavior is [`FlexDirection::Row`].
///
/// [Specification](https://www.w3.org/TR/css-flexbox-1/#flex-direction-property)
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum FlexDirection {
	/// Defines +x as the main axis
	///
	/// Items will be added from left to right in a row.
	#[default]
	Row,
	/// Defines +y as the main axis
	///
	/// Items will be added from top to bottom in a column.
	Column,
	/// Defines -x as the main axis
	///
	/// Items will be added from right to left in a row.
	RowReverse,
	/// Defines -y as the main axis
	///
	/// Items will be added from bottom to top in a column.
	ColumnReverse
}

/// How children overflowing their container should affect layout
///
/// In CSS the primary effect of this property is to control whether contents of a parent container that overflow that
/// container should be displayed anyway, be clipped, or trigger the container to become a scroll container. However it
/// also has secondary effects on layout, the main ones being:
///
///   - The automatic minimum size Flexbox/CSS Grid items with non-`Visible` overflow is `0` rather than being content
///     based
///   - `Overflow::Scroll` nodes have space in the layout reserved for a scrollbar (width controlled by the
///     `scrollbar_width` property)
///
/// In Taffy, we only implement the layout related secondary effects as we are not concerned with drawing/painting. The
/// amount of space reserved for a scrollbar is controlled by the `scrollbar_width` property. If this is `0` then
/// `Scroll` behaves identically to `Hidden`.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/overflow>
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Overflow {
	/// The automatic minimum size of this node as a flexbox/grid item should be based on the size of its content.
	/// Content that overflows this node *should* contribute to the scroll region of its parent.
	#[default]
	Visible,
	/// The automatic minimum size of this node as a flexbox/grid item should be based on the size of its content.
	/// Content that overflows this node should *not* contribute to the scroll region of its parent.
	Clip,
	/// The automatic minimum size of this node as a flexbox/grid item should be `0`.
	/// Content that overflows this node should *not* contribute to the scroll region of its parent.
	Hidden,
	/// The automatic minimum size of this node as a flexbox/grid item should be `0`. Additionally, space should be
	/// reserved for a scrollbar. The amount of space reserved is controlled by the `scrollbar_width` property.
	/// Content that overflows this node should *not* contribute to the scroll region of its parent.
	Scroll
}

/// The positioning strategy for this item.
///
/// This controls both how the origin is determined for the [`Style::position`] field,
/// and whether or not the item will be controlled by flexbox's layout algorithm.
///
/// WARNING: this enum follows the behavior of [CSS's `position` property](https://developer.mozilla.org/en-US/docs/Web/CSS/position),
/// which can be unintuitive.
///
/// [`Position::Relative`] is the default value, in contrast to the default behavior in CSS.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Position {
	/// The offset is computed relative to the final position given by the layout algorithm.
	/// Offsets do not affect the position of any other items; they are effectively a correction factor applied at the
	/// end.
	#[default]
	Relative,
	/// The offset is computed relative to this item's closest positioned ancestor, if any.
	/// Otherwise, it is placed relative to the origin.
	/// No space is created for the item in the page layout, and its size will not be altered.
	///
	/// WARNING: to opt-out of layouting entirely, you must use [`Display::None`] instead on your [`Style`] object.
	Absolute
}

#[cfg(test)]
mod tests {
	use lucie_common::{
		color::{blue, green, red, yellow},
		geometry::px
	};

	use super::*;

	#[test]
	fn test_basic_highlight_style_combination() {
		let style_a = HighlightStyle::default();
		let style_b = HighlightStyle::default();
		let style_a = style_a.highlight(style_b);
		assert_eq!(style_a, HighlightStyle::default(), "Combining empty styles should not produce a non-empty style.");

		let style_b = HighlightStyle {
			color: Some(red()),
			strikethrough: Some(StrikethroughStyle {
				thickness: px(2.),
				color: Some(blue())
			}),
			fade_out: Some(0.),
			font_style: Some(FontStyle::Italic),
			font_weight: Some(FontWeight(300.)),
			background_color: Some(yellow()),
			underline: Some(UnderlineStyle {
				thickness: px(2.),
				color: Some(red()),
				wavy: true
			})
		};
		let expected_style = style_b;

		let style_a = style_a.highlight(style_b);
		assert_eq!(style_a, expected_style, "Blending an empty style with another style should return the other style");

		let style_b = style_b.highlight(Default::default());
		assert_eq!(style_b, expected_style, "Blending a style with an empty style should not change the style.");

		let style_c = expected_style;

		let style_d = HighlightStyle {
			color: Some(blue().alpha(0.7)),
			strikethrough: Some(StrikethroughStyle {
				thickness: px(4.),
				color: Some(lucie_common::color::red())
			}),
			fade_out: Some(0.),
			font_style: Some(FontStyle::Oblique),
			font_weight: Some(FontWeight(800.)),
			background_color: Some(green()),
			underline: Some(UnderlineStyle {
				thickness: px(4.),
				color: None,
				wavy: false
			})
		};

		let expected_style = HighlightStyle {
			color: Some(red().blend(blue().alpha(0.7))),
			strikethrough: Some(StrikethroughStyle {
				thickness: px(4.),
				color: Some(red())
			}),
			// TODO this does not seem right
			fade_out: Some(0.),
			font_style: Some(FontStyle::Oblique),
			font_weight: Some(FontWeight(800.)),
			background_color: Some(green()),
			underline: Some(UnderlineStyle {
				thickness: px(4.),
				color: None,
				wavy: false
			})
		};

		let style_c = style_c.highlight(style_d);
		assert_eq!(style_c, expected_style, "Blending styles should blend properties where possible and override all others");
	}

	#[test]
	fn test_combine_highlights() {
		assert_eq!(
			combine_highlights(
				[(0..5, green().into()), (4..10, FontWeight::BOLD.into()), (15..20, yellow().into()),],
				[(2..6, FontStyle::Italic.into()), (1..3, blue().into()), (21..23, red().into()),]
			)
			.collect::<Vec<_>>(),
			[
				(
					0..1,
					HighlightStyle {
						color: Some(green()),
						..Default::default()
					}
				),
				(
					1..2,
					HighlightStyle {
						color: Some(blue()),
						..Default::default()
					}
				),
				(
					2..3,
					HighlightStyle {
						color: Some(blue()),
						font_style: Some(FontStyle::Italic),
						..Default::default()
					}
				),
				(
					3..4,
					HighlightStyle {
						color: Some(green()),
						font_style: Some(FontStyle::Italic),
						..Default::default()
					}
				),
				(
					4..5,
					HighlightStyle {
						color: Some(green()),
						font_weight: Some(FontWeight::BOLD),
						font_style: Some(FontStyle::Italic),
						..Default::default()
					}
				),
				(
					5..6,
					HighlightStyle {
						font_weight: Some(FontWeight::BOLD),
						font_style: Some(FontStyle::Italic),
						..Default::default()
					}
				),
				(
					6..10,
					HighlightStyle {
						font_weight: Some(FontWeight::BOLD),
						..Default::default()
					}
				),
				(
					15..20,
					HighlightStyle {
						color: Some(yellow()),
						..Default::default()
					}
				),
				(
					21..23,
					HighlightStyle {
						color: Some(red()),
						..Default::default()
					}
				)
			]
		);
	}

	#[test]
	fn test_text_style_refinement() {
		let mut style = Style::default();
		style.refine(&StyleRefinement::default().text_size(px(20.0)));
		style.refine(&StyleRefinement::default().font_weight(FontWeight::SEMIBOLD));

		assert_eq!(Some(AbsoluteLength::from(px(20.0))), style.text_style().unwrap().font_size);

		assert_eq!(Some(FontWeight::SEMIBOLD), style.text_style().unwrap().font_weight);
	}
}
