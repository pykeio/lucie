use std::{
	pin::Pin,
	task::{Context, Poll}
};

use lucie_common::{
	color::{BackgroundTag, Hsla},
	geometry::{Bounds, Edges, Pixels, Point, point}
};
use lucie_style::{Fill, Overflow, Style};

use crate::{App, ContentMask, Window, quad};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn file_url_to_path(url: &str) -> Option<std::path::PathBuf> {
	const FILE_SCHEME: &str = "file://";
	let url = percent_encoding::percent_decode_str(url).decode_utf8().ok()?;
	if !url.starts_with(FILE_SCHEME) {
		return None;
	}

	let path_str = &url[FILE_SCHEME.len()..];
	if !path_str.starts_with("/") {
		// has hostname, we're not doing all that
		return None;
	}

	std::path::Path::new(path_str).canonicalize().ok()
}

/// Use this struct for interfacing with the 'debug_below' styling from your own elements.
/// If a parent element has this style set on it, then this struct will be set as a global in
/// Lucie.
#[cfg(debug_assertions)]
pub struct DebugBelow;

#[cfg(debug_assertions)]
impl crate::Global for DebugBelow {}

pub fn paint_style(style: &Style, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App, continuation: impl FnOnce(&mut Window, &mut App)) {
	#[cfg(debug_assertions)]
	if style.debug_below {
		cx.set_global(DebugBelow)
	}

	#[cfg(debug_assertions)]
	if style.debug || cx.has_global::<DebugBelow>() {
		window.paint_quad(crate::outline(bounds, lucie_common::color::red(), lucie_style::BorderStyle::default()));
	}

	let rem_size = window.rem_size();
	let corner_radii = style.corner_radii.to_pixels(rem_size).clamp_radii_for_quad_size(bounds.size);

	window.paint_shadows(bounds, corner_radii, &style.box_shadow);

	let background_color = style.background.as_ref().and_then(Fill::color);
	if background_color.is_some_and(|color| !color.is_transparent()) {
		let mut border_color = match background_color {
			Some(color) => match color.tag {
				BackgroundTag::Solid => color.solid,
				BackgroundTag::LinearGradient => color.colors.first().map(|stop| stop.color).unwrap_or_default(),
				BackgroundTag::PatternSlash => color.solid
			},
			None => Hsla::default()
		};
		border_color.a = 0.;
		window.paint_quad(quad(bounds, corner_radii, background_color.unwrap_or_default(), Edges::default(), border_color, style.border_style));
	}

	continuation(window, cx);

	if is_border_visible(&style) {
		let border_widths = style.border_widths.to_pixels(rem_size);
		let max_border_width = border_widths.max();
		let max_corner_radius = corner_radii.max();

		let top_bounds = Bounds::from_corners(bounds.origin, bounds.top_right() + point(Pixels::ZERO, max_border_width.max(max_corner_radius)));
		let bottom_bounds = Bounds::from_corners(bounds.bottom_left() - point(Pixels::ZERO, max_border_width.max(max_corner_radius)), bounds.bottom_right());
		let left_bounds = Bounds::from_corners(top_bounds.bottom_left(), bottom_bounds.origin + point(max_border_width, Pixels::ZERO));
		let right_bounds = Bounds::from_corners(top_bounds.bottom_right() - point(max_border_width, Pixels::ZERO), bottom_bounds.top_right());

		let mut background = style.border_color.unwrap_or_default();
		background.a = 0.;
		let quad = quad(bounds, corner_radii, background, border_widths, style.border_color.unwrap_or_default(), style.border_style);

		window.with_content_mask(Some(ContentMask { bounds: top_bounds }), |window| {
			window.paint_quad(quad.clone());
		});
		window.with_content_mask(Some(ContentMask { bounds: right_bounds }), |window| {
			window.paint_quad(quad.clone());
		});
		window.with_content_mask(Some(ContentMask { bounds: bottom_bounds }), |window| {
			window.paint_quad(quad.clone());
		});
		window.with_content_mask(Some(ContentMask { bounds: left_bounds }), |window| {
			window.paint_quad(quad);
		});
	}

	#[cfg(debug_assertions)]
	if style.debug_below {
		cx.remove_global::<DebugBelow>();
	}
}

#[inline]
fn is_border_visible(style: &Style) -> bool {
	style.border_color.is_some_and(|color| !color.is_transparent()) && style.border_widths.any(|length| !length.is_zero())
}

pub fn overflow_mask(style: &Style, bounds: Bounds<Pixels>, rem_size: Pixels) -> Option<ContentMask<Pixels>> {
	match style.overflow {
		Point {
			x: Overflow::Visible,
			y: Overflow::Visible
		} => None,
		_ => {
			let mut min = bounds.origin;
			let mut max = bounds.bottom_right();

			if style.border_color.is_some_and(|color| !color.is_transparent()) {
				min.x += style.border_widths.left.to_pixels(rem_size);
				max.x -= style.border_widths.right.to_pixels(rem_size);
				min.y += style.border_widths.top.to_pixels(rem_size);
				max.y -= style.border_widths.bottom.to_pixels(rem_size);
			}

			let bounds = match (style.overflow.x == Overflow::Visible, style.overflow.y == Overflow::Visible) {
				// x and y both visible
				(true, true) => return None,
				// x visible, y hidden
				(true, false) => Bounds::from_corners(point(min.x, bounds.origin.y), point(max.x, bounds.bottom_right().y)),
				// x hidden, y visible
				(false, true) => Bounds::from_corners(point(bounds.origin.x, min.y), point(bounds.bottom_right().x, max.y)),
				// both hidden
				(false, false) => Bounds::from_corners(min, max)
			};

			Some(ContentMask { bounds })
		}
	}
}

pin_project_lite::pin_project! {
	pub struct Race<F1, F2> {
		#[pin]
		fut1: F1,
		#[pin]
		fut2: F2,
		rng: fastrand::Rng
	}
}

impl<T, F1: Future<Output = T>, F2: Future<Output = T>> Race<F1, F2> {
	pub fn new(fut1: F1, fut2: F2) -> Self {
		Self {
			fut1,
			fut2,
			rng: fastrand::Rng::new()
		}
	}
}

impl<T, F1: Future<Output = T>, F2: Future<Output = T>> Future for Race<F1, F2> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let this = self.project();
		if this.rng.bool() {
			if let Poll::Ready(v) = this.fut1.poll(cx) {
				return Poll::Ready(v);
			}
			if let Poll::Ready(v) = this.fut2.poll(cx) {
				return Poll::Ready(v);
			}
		} else {
			if let Poll::Ready(v) = this.fut2.poll(cx) {
				return Poll::Ready(v);
			}
			if let Poll::Ready(v) = this.fut1.poll(cx) {
				return Poll::Ready(v);
			}
		}
		Poll::Pending
	}
}
