use lucie_common::geometry::{DevicePixels, Point, ScaledPixels, Size, point, size};
use read_fonts::tables::cpal::ColorRecord;
use skrifa::color::ColorStop;
use tiny_skia::Pixmap;

use crate::{
	SubpixelVariant,
	font::FontHandle,
	hinting::{HintCache, HintKey},
	run::RunData
};

#[derive(Clone)]
pub struct RasterizedGlyph {
	pub size: Size<DevicePixels>,
	pub data: Vec<u8>,
	/// If `true`, `data` has only alpha channel (1 byte/pixel); otherwise RGBA (4 bytes/pixel).
	pub is_monochromatic: bool,
	/// Offset to draw glyph at
	pub offset: Point<ScaledPixels>
}

impl RasterizedGlyph {
	pub fn to_polychromatic(&self) -> RasterizedGlyph {
		if !self.is_monochromatic {
			return self.clone();
		}

		let mut data = Vec::with_capacity(self.data.len() * 4);
		for px in &self.data {
			data.extend_from_slice(&[0, 0, 0, *px]);
		}
		Self {
			size: self.size,
			data,
			is_monochromatic: false,
			offset: self.offset
		}
	}
}

pub(crate) fn rasterize_outline_glyph(
	outline: skrifa::OutlineGlyph<'_>,
	subpixel_variant: SubpixelVariant,
	run_data: &RunData<'_>,
	font: &FontHandle,
	outlines: &skrifa::OutlineGlyphCollection<'_>,
	hint_cache: Option<&mut HintCache>
) -> Option<RasterizedGlyph> {
	let mut pen = OutlineGlyphPen::new(tiny_skia::Point { x: subpixel_variant.offset(), y: 0.0 }, tiny_skia::Transform::from_scale(1.0, -1.0));
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

	let mut data = vec![0; w as usize * h as usize];
	for (i, pixel) in pixmap.pixels().iter().enumerate() {
		data[i] = pixel.alpha();
	}

	Some(RasterizedGlyph {
		size: size(DevicePixels(pixmap.width() as _), DevicePixels(pixmap.height() as _)),
		data,
		is_monochromatic: true,
		offset: point(ScaledPixels(x.round()), ScaledPixels(y.round()))
	})
}

struct OutlineGlyphPen {
	pub(crate) path: tiny_skia::PathBuilder,
	offset: tiny_skia::Point,
	transform: tiny_skia::Transform
}

impl OutlineGlyphPen {
	pub(crate) fn new(offset: tiny_skia::Point, transform: tiny_skia::Transform) -> Self {
		Self {
			path: tiny_skia::PathBuilder::new(),
			offset,
			transform
		}
	}

	fn transform_point(&self, x: f32, y: f32) -> tiny_skia::Point {
		tiny_skia::Point {
			x: self.transform.sx * x + self.transform.ky * y + self.transform.tx + self.offset.x,
			y: self.transform.kx * x + self.transform.sy * y + self.transform.ty + self.offset.y
		}
	}
}

impl skrifa::outline::OutlinePen for OutlineGlyphPen {
	fn move_to(&mut self, x: f32, y: f32) {
		let p = self.transform_point(x, y);
		self.path.move_to(p.x, p.y);
	}

	fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
		let c0 = self.transform_point(cx0, cy0);
		let c1 = self.transform_point(cx1, cy1);
		let p = self.transform_point(x, y);
		self.path.cubic_to(c0.x, c0.y, c1.x, c1.y, p.x, p.y);
	}

	fn line_to(&mut self, x: f32, y: f32) {
		let p = self.transform_point(x, y);
		self.path.line_to(p.x, p.y);
	}

	fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
		let c = self.transform_point(cx0, cy0);
		let p = self.transform_point(x, y);
		self.path.quad_to(c.x, c.y, p.x, p.y);
	}

	fn close(&mut self) {
		self.path.close();
	}
}

pub(crate) fn rasterize_color_glyph(
	glyph: skrifa::color::ColorGlyph<'_>,
	units_per_em: f32,
	run_data: &RunData<'_>,
	outlines: &skrifa::OutlineGlyphCollection<'_>,
	colors: &[ColorRecord]
) -> Option<RasterizedGlyph> {
	let mut pen = ColorGlyphPen::new(outlines, colors, run_data, units_per_em);
	glyph.paint(run_data.normalized_coords(), &mut pen).unwrap();

	let (pixmap, offset) = pen.finish()?;

	Some(RasterizedGlyph {
		size: size(DevicePixels(pixmap.width() as _), DevicePixels(pixmap.height() as _)),
		data: pixmap.take(),
		is_monochromatic: false,
		offset
	})
}

struct ColorGlyphPen<'c, 'r> {
	outlines: &'c skrifa::OutlineGlyphCollection<'c>,
	colors: &'c [ColorRecord],
	run_data: &'r RunData<'r>,
	transforms: Vec<tiny_skia::Transform>,
	paths: Vec<tiny_skia::Path>,
	bounds: tiny_skia::Rect,
	fills: Vec<(Vec<tiny_skia::Path>, tiny_skia::Shader<'static>)>,
	scale: f32
}

impl<'c, 'r> ColorGlyphPen<'c, 'r> {
	pub fn new(outlines: &'c skrifa::OutlineGlyphCollection<'c>, color_records: &'c [ColorRecord], run_data: &'r RunData<'r>, units_per_em: f32) -> Self {
		Self {
			outlines,
			colors: color_records,
			run_data,
			transforms: Vec::new(),
			paths: Vec::new(),
			bounds: tiny_skia::Rect::from_xywh(0.0, 0.0, 0.0, 0.0).unwrap(),
			fills: Vec::new(),
			scale: run_data.size().ppem().unwrap_or_default() / units_per_em
		}
	}

	pub fn finish(self) -> Option<(Pixmap, Point<ScaledPixels>)> {
		let bounds = self.bounds;

		let (x, y) = (bounds.x(), bounds.y());
		let (w, h) = ((bounds.width().ceil() * self.scale) as u32, (bounds.height().ceil() * self.scale) as u32);

		let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
		for (paths, paint) in self.fills {
			let path = paths.last().unwrap();
			pixmap.fill_path(
				path,
				&tiny_skia::Paint { shader: paint, ..Default::default() },
				tiny_skia::FillRule::Winding,
				tiny_skia::Transform::from_translate(-x, -y).post_scale(self.scale, self.scale),
				None
			);
		}

		let offset = point(ScaledPixels(x * self.scale).round(), ScaledPixels(y * self.scale).round());

		Some((pixmap, offset))
	}
}

impl skrifa::color::ColorPainter for ColorGlyphPen<'_, '_> {
	fn push_transform(&mut self, transform: skrifa::color::Transform) {
		let mut transform = tiny_skia::Transform::from_row(transform.xx, transform.yx, transform.xy, transform.yy, transform.dx, transform.dy);
		if let Some(prev) = self.transforms.last() {
			transform = prev.pre_concat(transform);
		}
		self.transforms.push(transform);
	}

	fn pop_transform(&mut self) {
		self.transforms.pop();
	}

	fn push_clip_glyph(&mut self, glyph_id: skrifa::GlyphId) {
		let mut pen = OutlineGlyphPen::new(tiny_skia::Point::default(), tiny_skia::Transform::identity());
		let Some(outline) = self.outlines.get(glyph_id) else {
			return;
		};
		outline
			.draw(skrifa::outline::DrawSettings::unhinted(skrifa::prelude::Size::unscaled(), self.run_data.normalized_coords()), &mut pen)
			.unwrap();

		if let Some(path) = pen.path.finish() {
			let path = path
				.transform(self.transforms.last().copied().unwrap_or_default().post_scale(1.0, -1.0))
				.unwrap();
			self.bounds = self.bounds.join(&path.bounds()).unwrap();
			self.paths.push(path);
		}
	}

	fn push_clip_box(&mut self, clip_box: skrifa::raw::types::BoundingBox<f32>) {
		let mut path = tiny_skia::PathBuilder::new();
		path.move_to(clip_box.x_min, clip_box.y_min);
		path.line_to(clip_box.x_max, clip_box.y_min);
		path.line_to(clip_box.x_max, clip_box.y_max);
		path.line_to(clip_box.x_min, clip_box.y_max);
		path.close();
		if let Some(path) = path.finish() {
			let path = path
				.transform(self.transforms.last().copied().unwrap_or_default().post_scale(1.0, -1.0))
				.unwrap();
			self.bounds = self.bounds.join(&path.bounds()).unwrap();
			self.paths.push(path);
		}
	}

	fn pop_clip(&mut self) {
		self.paths.pop();
	}

	fn fill(&mut self, brush: skrifa::color::Brush<'_>) {
		let current_transform = self.transforms.last().copied().unwrap_or_default().post_scale(1.0, -1.0);
		let paint = match brush {
			skrifa::color::Brush::Solid { palette_index, alpha } => tiny_skia::Shader::SolidColor(resolve_color(&self.colors, palette_index, alpha)),
			skrifa::color::Brush::LinearGradient { p0, p1, color_stops, extend } => tiny_skia::LinearGradient::new(
				tiny_skia::Point::from_xy(p0.x, p0.y),
				tiny_skia::Point::from_xy(p1.x, p1.y),
				resolve_color_stops(self.colors, color_stops),
				match extend {
					skrifa::color::Extend::Reflect => tiny_skia::SpreadMode::Reflect,
					skrifa::color::Extend::Repeat => tiny_skia::SpreadMode::Repeat,
					_ => tiny_skia::SpreadMode::Pad
				},
				current_transform
			)
			.unwrap(),
			skrifa::color::Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => tiny_skia::RadialGradient::new(
				tiny_skia::Point::from_xy(c0.x, c0.y),
				r0,
				tiny_skia::Point::from_xy(c1.x, c1.y),
				r1,
				resolve_color_stops(self.colors, color_stops),
				match extend {
					skrifa::color::Extend::Reflect => tiny_skia::SpreadMode::Reflect,
					skrifa::color::Extend::Repeat => tiny_skia::SpreadMode::Repeat,
					_ => tiny_skia::SpreadMode::Pad
				},
				current_transform
			)
			.unwrap(),
			skrifa::color::Brush::SweepGradient {
				c0,
				start_angle,
				end_angle,
				color_stops,
				extend
			} => tiny_skia::SweepGradient::new(
				tiny_skia::Point::from_xy(c0.x, c0.y),
				start_angle,
				end_angle,
				resolve_color_stops(self.colors, color_stops),
				match extend {
					skrifa::color::Extend::Reflect => tiny_skia::SpreadMode::Reflect,
					skrifa::color::Extend::Repeat => tiny_skia::SpreadMode::Repeat,
					_ => tiny_skia::SpreadMode::Pad
				},
				current_transform
			)
			.unwrap()
		};
		self.fills.push((self.paths.clone(), paint));
	}

	fn push_layer(&mut self, _composite_mode: skrifa::color::CompositeMode) {
		unimplemented!("layers");
	}
}

fn resolve_color(colors: &[ColorRecord], palette_index: u16, alpha: f32) -> tiny_skia::Color {
	if palette_index == 0xFFFF {
		tiny_skia::Color::BLACK
	} else {
		let Some(color) = colors.get(palette_index as usize) else {
			return tiny_skia::Color::BLACK;
		};

		tiny_skia::Color::from_rgba8(color.red, color.green, color.blue, (alpha * 255.).clamp(0.0, 255.0) as u8)
	}
}

fn resolve_color_stops(colors: &[ColorRecord], stops: &[ColorStop]) -> Vec<tiny_skia::GradientStop> {
	let mut sk_stops = Vec::with_capacity(0);
	for stop in stops {
		sk_stops.push(tiny_skia::GradientStop::new(stop.offset, resolve_color(colors, stop.palette_index, stop.alpha)));
	}
	sk_stops
}
