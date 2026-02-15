use std::{
	hash::{Hash, Hasher},
	slice
};

use lucie_common::geometry::{DevicePixels, Point, ScaledPixels, point};
use parley::GlyphClass;

use crate::{
	font::FontHandle,
	glyph::{GlyphId, PositionedGlyph, SubpixelVariant},
	style::Brush
};

pub struct Run<'a> {
	#[expect(unused)]
	run: parley::Run<'a, Brush>,
	data: RunData<'a>
}

impl<'a> Run<'a> {
	pub(crate) fn new(run: parley::Run<'a, Brush>) -> Self {
		let data = RunData::new(&run);
		Self { run, data }
	}

	#[inline]
	pub fn data(&self) -> &RunData<'a> {
		&self.data
	}
}

#[derive(Debug, Clone)]
pub struct RunData<'a> {
	font_handle: FontHandle,
	size: skrifa::instance::Size,
	normalized_coords: &'a [skrifa::instance::NormalizedCoord]
}

impl Hash for RunData<'_> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.font_handle.hash(state);
		self.size.ppem().map(f32::to_bits).hash(state);
		self.normalized_coords.hash(state);
	}
}

impl<'a> RunData<'a> {
	pub(crate) fn new(run: &parley::Run<'a, Brush>) -> Self {
		let normalized_coords = run.normalized_coords();
		const { assert!(size_of::<skrifa::instance::NormalizedCoord>() == size_of::<i16>()) };
		Self {
			font_handle: FontHandle::new(run.font().clone()),
			size: skrifa::instance::Size::new(run.font_size()),
			normalized_coords: unsafe { slice::from_raw_parts(normalized_coords.as_ptr().cast(), normalized_coords.len()) }
		}
	}

	#[inline]
	pub fn font(&self) -> &FontHandle {
		&self.font_handle
	}

	#[inline]
	pub(crate) fn size(&self) -> skrifa::instance::Size {
		self.size
	}

	#[inline]
	pub(crate) fn normalized_coords(&self) -> &[skrifa::instance::NormalizedCoord] {
		self.normalized_coords
	}
}

pub enum Renderable<'a> {
	GlyphRun(GlyphRun<'a>),
	InlineBox {
		id: u64,
		x: ScaledPixels,
		y: ScaledPixels,
		width: ScaledPixels,
		height: ScaledPixels
	}
}

pub struct GlyphRun<'a> {
	run: parley::GlyphRun<'a, Brush>,
	data: RunData<'a>
}

impl<'a> GlyphRun<'a> {
	pub(crate) fn new(run: parley::GlyphRun<'a, Brush>) -> Self {
		let data = RunData::new(run.run());
		Self { run, data }
	}

	#[inline]
	pub fn data(&self) -> &RunData<'a> {
		&self.data
	}

	#[inline]
	pub fn x(&self) -> ScaledPixels {
		ScaledPixels(self.run.offset())
	}

	#[inline]
	pub fn y(&self) -> ScaledPixels {
		ScaledPixels(self.run.baseline())
	}

	#[inline]
	pub fn style(&self) -> &parley::Style<Brush> {
		self.run.style()
	}

	#[inline]
	pub fn positioned_glyphs<'s>(&'s self, origin: Point<ScaledPixels>) -> impl Iterator<Item = PositionedGlyph<'s>> + 's + Clone {
		self.run.positioned_glyphs().map(move |glyph| {
			let (x, y) = (glyph.x + origin.x.0, glyph.y + origin.y.0);
			let (x, subpixel_variant) = if glyph.class == GlyphClass::Unclassified {
				SubpixelVariant::from_pos(x)
			} else {
				// CJK or other logographic script means there's gonna be a lot of different glyphs, so multiplying that by 4 by
				// allowing subpixel variants wouldn't be a good idea.
				(x as i32, SubpixelVariant::Zero)
			};
			let y = y as i32;
			PositionedGlyph {
				id: GlyphId(skrifa::GlyphId::new(glyph.id)),
				run_data: &self.data,
				class: glyph.class,
				subpixel_variant,
				origin: point(DevicePixels(x), DevicePixels(y))
			}
		})
	}
}
