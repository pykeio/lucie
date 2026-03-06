use std::hash::{Hash, Hasher};

use lucie_common::geometry::{DevicePixels, Point};
use parley::GlyphClass;
use rapidhash::fast::RapidHasher;

use crate::{FontHandle, run::RunData};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
#[repr(transparent)]
pub struct GlyphId(pub(crate) skrifa::GlyphId);

#[derive(Debug, Clone, Hash)]
pub struct PositionedGlyph<'a> {
	pub id: GlyphId,
	pub run_data: &'a RunData<'a>,
	pub class: GlyphClass,
	pub subpixel_variant: SubpixelVariant,
	pub origin: Point<DevicePixels>
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct GlyphKey(u64);

impl PositionedGlyph<'_> {
	#[inline]
	#[must_use]
	pub fn key(&self, font: &FontHandle) -> GlyphKey {
		let mut hasher = RapidHasher::default_const();
		font.hash(&mut hasher);
		self.id.hash(&mut hasher);
		self.run_data.hash(&mut hasher);
		self.class.hash(&mut hasher);
		self.subpixel_variant.hash(&mut hasher);
		GlyphKey(hasher.finish())
	}
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubpixelVariant {
	#[default]
	Zero,
	OneQuarter,
	Half,
	ThreeQuarters
}

impl SubpixelVariant {
	pub fn from_pos(pos: f32) -> (i32, Self) {
		let trunc = pos.trunc() as i32;
		let fract = pos - trunc as f32;
		if pos.is_sign_negative() {
			if fract > -0.125 {
				(trunc, Self::Zero)
			} else if fract > -0.375 {
				(trunc - 1, Self::ThreeQuarters)
			} else if fract > -0.625 {
				(trunc - 1, Self::Half)
			} else if fract > -0.875 {
				(trunc - 1, Self::OneQuarter)
			} else {
				(trunc - 1, Self::Zero)
			}
		} else {
			if fract < 0.125 {
				(trunc, Self::Zero)
			} else if fract < 0.375 {
				(trunc, Self::OneQuarter)
			} else if fract < 0.625 {
				(trunc, Self::Half)
			} else if fract < 0.875 {
				(trunc, Self::ThreeQuarters)
			} else {
				(trunc + 1, Self::Zero)
			}
		}
	}

	#[inline]
	pub const fn offset(&self) -> f32 {
		match self {
			SubpixelVariant::Zero => 0.0,
			SubpixelVariant::OneQuarter => 0.25,
			SubpixelVariant::Half => 0.5,
			SubpixelVariant::ThreeQuarters => 0.75
		}
	}
}
