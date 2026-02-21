use std::ops::Range;

use lucie_common::geometry::{Bounds, ScaledPixels, point, size};

use crate::layout::Layout;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Affinity {
	#[default]
	Downstream,
	Upstream
}

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct Cursor(parley::Cursor);

impl Cursor {
	#[inline]
	pub(crate) fn new(cursor: parley::Cursor) -> Self {
		Self(cursor)
	}

	#[inline]
	pub fn index(&self) -> usize {
		self.0.index()
	}

	#[inline]
	pub fn affinity(&self) -> Affinity {
		match self.0.affinity() {
			parley::Affinity::Downstream => Affinity::Downstream,
			parley::Affinity::Upstream => Affinity::Upstream
		}
	}

	#[inline]
	pub fn to_selection(&self) -> Selection {
		Selection::new(*self, *self)
	}

	#[inline]
	#[must_use]
	pub fn refresh(&self, layout: &Layout) -> Cursor {
		Cursor(self.0.refresh(layout.layout()))
	}

	#[inline]
	pub fn x(&self, layout: &Layout) -> ScaledPixels {
		ScaledPixels(self.0.geometry(layout.layout(), 0.0).x0 as f32)
	}
}

#[derive(Debug, Default, Copy, Clone)]
#[repr(transparent)]
pub struct Selection(parley::Selection);

impl Selection {
	pub fn new(anchor: Cursor, focus: Cursor) -> Self {
		Selection(parley::Selection::new(anchor.0, focus.0))
	}

	pub fn from_range(range: Range<usize>, layout: &Layout) -> Self {
		let start = layout.cursor_at_byte(range.start);
		let end = layout.cursor_at_byte(range.end);
		Selection::new(start, end)
	}

	#[inline]
	pub fn is_collapsed(&self) -> bool {
		self.0.is_collapsed()
	}

	#[inline]
	pub fn anchor(&self) -> Cursor {
		Cursor(self.0.anchor())
	}

	#[inline]
	pub fn focus(&self) -> Cursor {
		Cursor(self.0.focus())
	}

	#[inline]
	pub fn collapse(&self) -> Selection {
		Cursor(self.0.focus()).to_selection()
	}

	#[inline]
	pub fn text_range(&self) -> Range<usize> {
		self.0.text_range()
	}

	#[inline]
	#[must_use]
	pub fn refresh(&self, layout: &Layout) -> Self {
		Selection(self.0.refresh(layout.layout()))
	}

	#[inline]
	pub fn bounds(&self, layout: &Layout) -> Vec<(usize, Bounds<ScaledPixels>)> {
		let mut xs = vec![];
		self.0.geometry_with(layout.layout(), |bb, line| {
			xs.push((
				line,
				Bounds {
					origin: point(ScaledPixels(bb.x0 as f32), ScaledPixels(bb.y0 as f32)),
					size: size(ScaledPixels(bb.width() as f32), ScaledPixels(bb.height() as f32))
				}
			));
		});
		xs
	}
}
