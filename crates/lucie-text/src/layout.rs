use std::ops::Range;

use lucie_common::geometry::{Bounds, Pixels, Point, ScaledPixels, Size, px, size};
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
	alignment: Option<TextAlign>
}

impl Layout {
	#[inline]
	pub fn new() -> Self {
		Layout {
			layout: parley::Layout::new(),
			alignment: None
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

	pub fn fit(&mut self, max_width: Option<Pixels>) {
		let max_width = max_width.map(|x| x.0);
		self.layout.break_all_lines(max_width);
		if let Some(alignment) = self.alignment {
			self.layout.align(
				max_width,
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

	pub fn width(&self) -> Pixels {
		px(self.layout.width())
	}

	pub fn height(&self) -> Pixels {
		px(self.layout.height())
	}

	pub fn size(&self) -> Size<Pixels> {
		size(px(self.layout.width()), px(self.layout.height()))
	}

	pub fn lines(&self) -> impl Iterator<Item = Line<'_>> + '_ + Clone {
		self.layout.lines().map(|line| Line { line })
	}

	pub fn cursor_at(&self, point: Point<Pixels>) -> Cursor {
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

	pub fn height(&self) -> Pixels {
		px(self.line.metrics().line_height)
	}

	pub fn runs(&self) -> impl Iterator<Item = Run<'a>> + 'a + Clone {
		self.line.runs().map(Run::new)
	}

	pub fn renderables(&self) -> impl Iterator<Item = Renderable<'a>> + 'a + Clone {
		self.line.items().map(|item| match item {
			parley::PositionedLayoutItem::GlyphRun(run) => Renderable::GlyphRun(GlyphRun::new(run)),
			parley::PositionedLayoutItem::InlineBox(b) => Renderable::InlineBox {
				id: b.id,
				x: px(b.x),
				y: px(b.y),
				width: px(b.width),
				height: px(b.height)
			}
		})
	}

	pub fn paint<P: TextPainter>(&self, text_system: &TextSystem, painter: &mut P, origin: Point<ScaledPixels>) -> Result<(), P::Error> {
		let mut painter = painter.create_layer(Bounds::new(Point::default(), size(px(1_000.0), px(1_000_000.0))));
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
