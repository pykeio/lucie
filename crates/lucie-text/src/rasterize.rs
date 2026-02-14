use lucie_common::geometry::{DevicePixels, Point, Size, point, size};

use crate::{
	SubpixelVariant,
	font::FontHandle,
	hinting::{HintCache, HintKey},
	run::RunData
};

pub struct RasterizedGlyph {
	pub size: Size<DevicePixels>,
	pub data: Vec<u8>,
	/// If `true`, `data` has only alpha channel (1 byte/pixel); otherwise RGBA (4 bytes/pixel).
	pub is_monochromatic: bool,
	/// Offset to draw glyph at
	pub offset: Point<DevicePixels>
}

pub(crate) fn rasterize_outline_glyph(
	outline: skrifa::OutlineGlyph<'_>,
	subpixel_variant: SubpixelVariant,
	run_data: &RunData<'_>,
	font: &FontHandle,
	outlines: &skrifa::OutlineGlyphCollection<'_>,
	hint_cache: Option<&mut HintCache>
) -> Option<RasterizedGlyph> {
	let mut pen = TinySkiaPen::default();
	outline
		.draw(
			if let Some(hint_cache) = hint_cache
				&& let Some(instance) = hint_cache.get(outlines, HintKey::new(font, run_data))
			{
				skrifa::outline::DrawSettings::hinted(instance, false)
			} else {
				skrifa::outline::DrawSettings::unhinted(run_data.size(), run_data.normalized_coords())
			},
			&mut pen
		)
		.unwrap();

	let subpixel_offset = subpixel_variant.offset();

	let path = pen.path.finish()?;
	let bounds = path.bounds().round_out().unwrap();
	let (mut w, h) = (bounds.width() as u32, bounds.height() as u32);
	if subpixel_offset > 0. {
		w += 1;
	}

	let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
	pixmap.fill_path(
		&path,
		&tiny_skia::Paint::default(),
		tiny_skia::FillRule::Winding,
		tiny_skia::Transform::from_translate(-(bounds.left() as f32 - subpixel_offset), -bounds.top() as f32 - bounds.height() as f32).post_scale(1.0, -1.0),
		None
	);

	Some(RasterizedGlyph {
		size: size(DevicePixels(pixmap.width() as _), DevicePixels(pixmap.height() as _)),
		data: pixmap.data().to_vec(),
		is_monochromatic: false,
		offset: point(DevicePixels(bounds.left()), DevicePixels(-(pixmap.height() as i32) - bounds.top()))
	})
}

#[derive(Default)]
struct TinySkiaPen {
	pub(crate) path: tiny_skia::PathBuilder
}

impl skrifa::outline::OutlinePen for TinySkiaPen {
	fn move_to(&mut self, x: f32, y: f32) {
		self.path.move_to(x, y);
	}

	fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
		self.path.cubic_to(cx0, cy0, cx1, cy1, x, y);
	}

	fn line_to(&mut self, x: f32, y: f32) {
		self.path.line_to(x, y);
	}

	fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
		self.path.quad_to(cx0, cy0, x, y);
	}

	fn close(&mut self) {
		self.path.close();
	}
}
