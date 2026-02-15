use lucie_common::geometry::{DevicePixels, Point, ScaledPixels, Size, point, size};

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
	pub offset: Point<ScaledPixels>
}

pub(crate) fn rasterize_outline_glyph(
	outline: skrifa::OutlineGlyph<'_>,
	subpixel_variant: SubpixelVariant,
	run_data: &RunData<'_>,
	font: &FontHandle,
	outlines: &skrifa::OutlineGlyphCollection<'_>,
	hint_cache: Option<&mut HintCache>
) -> Option<RasterizedGlyph> {
	let mut pen = TinySkiaPen::new(subpixel_variant.offset(), 0.0);
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

	let path = pen.path.finish()?;
	let bounds = path.bounds();

	let (x, y) = (bounds.x(), bounds.y());
	let (w, h) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);

	let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
	pixmap.fill_path(&path, &tiny_skia::Paint::default(), tiny_skia::FillRule::Winding, tiny_skia::Transform::from_translate(-x, -y), None);

	Some(RasterizedGlyph {
		size: size(DevicePixels(pixmap.width() as _), DevicePixels(pixmap.height() as _)),
		data: pixmap.data().to_vec(),
		is_monochromatic: false,
		offset: point(ScaledPixels(x.round()), ScaledPixels(y.round()))
	})
}

struct TinySkiaPen {
	pub(crate) path: tiny_skia::PathBuilder,
	x_offset: f32,
	y_offset: f32
}

impl TinySkiaPen {
	pub(crate) fn new(x_offset: f32, y_offset: f32) -> Self {
		Self {
			path: tiny_skia::PathBuilder::new(),
			x_offset,
			y_offset
		}
	}
}

impl skrifa::outline::OutlinePen for TinySkiaPen {
	fn move_to(&mut self, x: f32, y: f32) {
		self.path.move_to(self.x_offset + x, self.y_offset - y);
	}

	fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
		self.path
			.cubic_to(self.x_offset + cx0, self.y_offset - cy0, self.x_offset + cx1, self.y_offset - cy1, self.x_offset + x, self.y_offset - y);
	}

	fn line_to(&mut self, x: f32, y: f32) {
		self.path.line_to(self.x_offset + x, self.y_offset - y);
	}

	fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
		self.path
			.quad_to(self.x_offset + cx0, self.y_offset - cy0, self.x_offset + x, self.y_offset - y);
	}

	fn close(&mut self) {
		self.path.close();
	}
}
