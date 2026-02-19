use std::ops::Range;

use lucie_common::geometry::{Bounds, Point, ScaledPixels, Size, size};
use lucie_style::TextAlign;

use crate::{
	TextPainter, TextSystem,
	run::{GlyphRun, Renderable, Run},
	select::Cursor,
	style::Brush
};

#[derive(Clone)]
pub struct Layout {
	layout: parley::Layout<Brush>,
	pub(crate) alignment: Option<TextAlign>,
	pub(crate) wrap_width: Option<ScaledPixels>,
	pub(crate) has_shaped: bool,
	needs_realign: bool
}

impl Layout {
	#[inline]
	pub fn new() -> Self {
		Layout {
			layout: parley::Layout::new(),
			alignment: None,
			wrap_width: None,
			has_shaped: false,
			needs_realign: false
		}
	}

	#[inline]
	pub(crate) fn layout(&self) -> &parley::Layout<Brush> {
		&self.layout
	}

	#[inline]
	pub(crate) fn layout_mut(&mut self) -> &mut parley::Layout<Brush> {
		&mut self.layout
	}

	pub fn align(&mut self, alignment: Option<TextAlign>) {
		debug_assert!(self.has_shaped);
		if alignment != self.alignment || self.needs_realign {
			self.layout.align(
				self.wrap_width.map(|x| x.0),
				match alignment {
					Some(TextAlign::Left) | None => parley::Alignment::Start,
					Some(TextAlign::Center) => parley::Alignment::Center,
					Some(TextAlign::Right) => parley::Alignment::End
				},
				parley::AlignmentOptions::default()
			);
			self.needs_realign = false;
		}
	}

	pub fn fit(&mut self, max_width: Option<ScaledPixels>) {
		if max_width == self.wrap_width && self.has_shaped {
			return;
		}
		if self.num_lines() == 1
			&& let Some(wrap_width) = self.wrap_width
			&& (max_width.is_none() || max_width.is_some_and(|mw| mw >= wrap_width))
		{
			// don't bother re-wrapping if the old text wasn't wrapped within a smaller bound than the new bound
			// but still store wrap_width so align works
			self.wrap_width = max_width;
			self.needs_realign = true;
			return;
		}

		self.wrap_width = max_width;

		let max_width = max_width.map(|x| x.0);
		self.layout.break_all_lines(max_width);
		self.has_shaped = true;
		self.needs_realign = true;
	}

	pub fn num_lines(&self) -> usize {
		self.layout.len()
	}

	pub fn width(&self) -> ScaledPixels {
		ScaledPixels(self.layout.width())
	}

	pub fn height(&self) -> ScaledPixels {
		ScaledPixels(self.layout.height())
	}

	pub fn size(&self) -> Size<ScaledPixels> {
		size(ScaledPixels(self.layout.width()), ScaledPixels(self.layout.height()))
	}

	pub fn lines(&self) -> impl Iterator<Item = Line<'_>> + '_ + Clone {
		self.layout.lines().map(|line| Line { line })
	}

	pub fn cursor_at(&self, point: Point<ScaledPixels>) -> Cursor {
		Cursor::new(parley::Cursor::from_point(&self.layout, point.x.0, point.y.0))
	}

	pub fn cursor_at_byte(&self, byte: usize) -> Cursor {
		Cursor::new(parley::Cursor::from_byte_index(&self.layout, byte, parley::Affinity::Downstream))
	}
}

#[derive(Clone)]
pub struct Line<'a> {
	line: parley::Line<'a, Brush>
}

impl<'a> Line<'a> {
	/// Returns the range of text for the line.
	pub fn text_range(&self) -> Range<usize> {
		self.line.text_range()
	}

	pub fn width(&self) -> ScaledPixels {
		ScaledPixels(self.line.metrics().advance)
	}

	pub fn height(&self) -> ScaledPixels {
		ScaledPixels(self.line.metrics().line_height)
	}

	pub fn runs(&self) -> impl Iterator<Item = Run<'a>> + 'a + Clone {
		self.line.runs().map(Run::new)
	}

	pub fn renderables(&self) -> impl Iterator<Item = Renderable<'a>> + 'a + Clone {
		self.line.items().map(|item| match item {
			parley::PositionedLayoutItem::GlyphRun(run) => Renderable::GlyphRun(GlyphRun::new(run)),
			parley::PositionedLayoutItem::InlineBox(b) => Renderable::InlineBox {
				id: b.id,
				x: ScaledPixels(b.x),
				y: ScaledPixels(b.y),
				width: ScaledPixels(b.width),
				height: ScaledPixels(b.height)
			}
		})
	}

	pub fn paint<P: TextPainter>(&self, text_system: &TextSystem, painter: &mut P, origin: Point<ScaledPixels>) -> Result<(), P::Error> {
		let mut painter = painter.create_layer(Bounds::new(origin, size(self.width(), self.height())));
		for renderable in self.renderables() {
			match renderable {
				Renderable::GlyphRun(run) => {
					let font = text_system.font_cache().get(run.data().font());
					for glyph in run.positioned_glyphs(origin) {
						painter.paint_glyph(glyph, &font, run.data(), run.style().brush.color)?;
					}
				}
				Renderable::InlineBox { .. } => unimplemented!("inline boxes not implemented")
			}
		}
		Ok(())
	}
}
