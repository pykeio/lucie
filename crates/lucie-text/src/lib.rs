use std::{
	mem::{self, ManuallyDrop},
	ops::{DerefMut, RangeBounds},
	ptr,
	sync::Arc
};

use lucie_common::{
	color::Hsla,
	geometry::{Bounds, Pixels, Point, ScaledPixels, Size}
};
use lucie_style::{StrikethroughStyle, TextStyle, TextStyleRefinement, UnderlineStyle};
use parking_lot::Mutex;

mod font;
mod glyph;
mod hinting;
mod layout;
mod rasterize;
mod run;
mod select;
mod style;

pub use self::{
	font::{FontCache, FontHandle, LoadedFont},
	glyph::{GlyphId, GlyphKey, PositionedGlyph, SubpixelVariant},
	hinting::HintCache,
	layout::{Layout, Line},
	rasterize::RasterizedGlyph,
	run::{GlyphRun, Renderable, Run, RunData},
	select::{Affinity, Cursor, Selection}
};
use crate::style::{Brush, style_to_parley};

struct ParleyContext {
	pub(crate) font: Mutex<parley::FontContext>,
	pub(crate) layout: Mutex<parley::LayoutContext<Brush>>
}

pub struct TextSystem {
	parley_ctx: Arc<ParleyContext>,
	font_cache: FontCache,
	hint_cache: Mutex<Option<HintCache>>
}

impl TextSystem {
	pub fn new() -> Self {
		Self {
			parley_ctx: Arc::new(ParleyContext {
				font: Mutex::new(parley::FontContext::new()),
				layout: Mutex::new(parley::LayoutContext::new())
			}),
			font_cache: FontCache::new(),
			hint_cache: Mutex::new(Some(HintCache::new()))
		}
	}

	/// Get a list of all available font names from the operating system.
	pub fn all_font_names(&self) -> Vec<String> {
		let mut names: Vec<String> = self.parley_ctx.font.lock().collection.family_names().map(String::from).collect();
		names.push("system-ui".to_string());
		names.sort();
		names.dedup();
		names
	}

	/// Add a font's data to the text system.
	pub fn add_font(&self, font: Vec<u8>) {
		let mut font_context = self.parley_ctx.font.lock();
		font_context.collection.register_fonts(font.into(), None);
	}

	pub fn finish_frame(&self) {}

	pub fn font_cache(&self) -> &FontCache {
		&self.font_cache
	}

	pub fn hint_cache(&self) -> impl DerefMut<Target = Option<HintCache>> + '_ {
		self.hint_cache.lock()
	}

	pub fn ranged_builder<'style, 'text, 'this: 'style>(
		&'this self,
		text: &'text str,
		rem_size: Pixels,
		dpr: f32,
		base_style: &'style TextStyle
	) -> RangedBuilder<'style> {
		let builder = {
			let mut layout = self.parley_ctx.layout.lock();
			let mut font = self.parley_ctx.font.lock();

			let mut builder = unsafe {
				mem::transmute::<parley::RangedBuilder<'_, Brush>, parley::RangedBuilder<'this, Brush>>(layout.ranged_builder(&mut font, text, dpr, false))
			};
			builder.push_default(parley::StyleProperty::OverflowWrap(parley::OverflowWrap::BreakWord));
			style_to_parley(
				&TextStyleRefinement {
					color: Some(base_style.color),
					font_family: Some(base_style.font_family.clone()),
					font_features: Some(base_style.font_features.clone()),
					font_fallbacks: base_style.font_fallbacks.clone(),
					font_size: Some(base_style.font_size),
					line_height: Some(base_style.line_height),
					font_weight: Some(base_style.font_weight),
					font_style: Some(base_style.font_style),
					background_color: base_style.background_color,
					underline: base_style.underline.clone(),
					strikethrough: base_style.strikethrough.clone(),
					white_space: Some(base_style.white_space),
					..Default::default()
				},
				rem_size,
				|property| {
					builder.push_default(property);
				}
			);

			mem::forget((font, layout));
			builder
		};
		RangedBuilder {
			builder,
			rem_size,
			_parley_ctx: self.parley_ctx.clone()
		}
	}
}

pub struct RangedBuilder<'ctx> {
	builder: parley::RangedBuilder<'ctx, Brush>,
	rem_size: Pixels,
	_parley_ctx: Arc<ParleyContext>
}

impl<'s> RangedBuilder<'s> {
	pub fn push_style(&mut self, range: impl RangeBounds<usize> + Clone, refinement: &TextStyleRefinement) {
		style_to_parley(refinement, self.rem_size, |property| {
			self.builder.push(property, range.clone());
		});
	}

	pub fn push_inline_box(&mut self, id: u64, char_index: usize, size: Size<ScaledPixels>) {
		self.builder.push_inline_box(parley::InlineBox {
			id,
			index: char_index,
			width: size.width.0,
			height: size.height.0
		});
	}

	pub fn build_into(self, layout: &mut Layout, text: impl AsRef<str>) {
		let mut this = ManuallyDrop::new(self);
		let builder = unsafe { ptr::read(&mut this.builder) };
		builder.build_into(layout.layout_mut(), text.as_ref());
		unsafe { this.release() };
	}

	pub fn build(self, text: impl AsRef<str>) -> Layout {
		let mut layout = Layout::new();
		self.build_into(&mut layout, text.as_ref());
		layout
	}

	pub fn build_into_from_runs(self, layout: &mut Layout, text: &str, runs: &[lucie_style::TextRun]) {
		build_from_runs(text, runs, self, layout);
	}

	unsafe fn release(&self) {
		unsafe {
			self._parley_ctx.font.force_unlock();
			self._parley_ctx.layout.force_unlock();
		};
	}
}

impl<'ctx> Drop for RangedBuilder<'ctx> {
	fn drop(&mut self) {
		unsafe { self.release() };
	}
}

fn build_from_runs(text: &str, runs: &[lucie_style::TextRun], mut builder: RangedBuilder, layout: &mut Layout) {
	let mut run_start = 0;
	for run in runs {
		builder.push_style(
			run_start..run_start + run.len,
			&TextStyleRefinement {
				font_family: Some(run.font.family.clone()),
				font_weight: Some(run.font.weight),
				font_fallbacks: run.font.fallbacks.clone(),
				color: Some(run.color),
				background_color: run.background_color,
				..Default::default()
			}
		);
		run_start += run.len;
	}

	builder.build_into(layout, text);
}

pub trait TextPainter {
	type Error;

	fn create_layer<'s>(&'s mut self, bounds: Bounds<ScaledPixels>) -> impl DerefMut<Target = Self> + 's;

	fn paint_underline(&mut self, origin: Point<ScaledPixels>, width: ScaledPixels, style: &UnderlineStyle);
	fn paint_strikethrough(&mut self, origin: Point<ScaledPixels>, width: ScaledPixels, style: &StrikethroughStyle);

	fn paint_glyph(&mut self, glyph: PositionedGlyph<'_>, font: &LoadedFont, run_data: &RunData<'_>, color: Hsla) -> Result<(), Self::Error>;

	fn paint_background(&mut self, bounds: Bounds<ScaledPixels>, background: Hsla);
}
