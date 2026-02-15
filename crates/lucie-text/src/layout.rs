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
	pub(crate) has_shaped: bool
}

impl Layout {
	#[inline]
	pub fn new() -> Self {
		Layout {
			layout: parley::Layout::new(),
			alignment: None,
			wrap_width: None,
			has_shaped: false
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

	pub fn set_alignment(&mut self, alignment: Option<TextAlign>) {
		self.alignment = alignment;
	}

	pub fn fit(&mut self, max_width: Option<ScaledPixels>) {
		self.wrap_width = max_width;

		let max_width = max_width.map(|x| x.0);
		self.layout.break_all_lines(max_width);
		self.has_shaped = true;

		if let Some(wrap_width) = self.wrap_width
			&& let Some(alignment) = self.alignment
			&& self.has_shaped
		{
			self.layout.align(
				Some(wrap_width.0),
				match alignment {
					TextAlign::Left => parley::Alignment::Start,
					TextAlign::Center => parley::Alignment::Center,
					TextAlign::Right => parley::Alignment::End
				},
				parley::AlignmentOptions::default()
			);
		}
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
