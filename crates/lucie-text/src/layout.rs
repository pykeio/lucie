use std::{cmp::Ordering, hint::select_unpredictable, num::NonZeroU32, ops::Range};

use lucie_common::geometry::{Bounds, Pixels, Point, ScaledPixels, Size, point, px, size};
use lucie_style::{StrikethroughStyle, TextAlign, TextStyle, UnderlineStyle};

use crate::{
	Affinity, TextPainter, TextSystem, apply_base_text_style,
	run::{GlyphRun, Renderable, Run},
	select::Cursor,
	style::{Brush, TruncateFrom}
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShapeMode {
	Fit {
		wrap_width: Option<ScaledPixels>
	},
	Truncate {
		max_width: ScaledPixels,
		line_clamp: Option<NonZeroU32>,
		from: TruncateFrom
	}
}

impl ShapeMode {
	#[inline]
	pub fn max_width(&self) -> Option<ScaledPixels> {
		match self {
			Self::Fit { wrap_width } => *wrap_width,
			Self::Truncate { max_width, .. } => Some(*max_width)
		}
	}
}

#[derive(Clone)]
pub struct Layout {
	layout: parley::Layout<Brush>,
	alignment: Option<TextAlign>,
	shape_mode: Option<ShapeMode>,
	needs_realign: bool,
	pub(crate) rem_size: Pixels,
	truncation: Option<Truncation>,
	pub(crate) text_len: usize
}

impl Layout {
	#[inline]
	pub fn new() -> Self {
		Layout {
			layout: parley::Layout::new(),
			alignment: None,
			shape_mode: None,
			needs_realign: false,
			rem_size: px(0.0),
			truncation: None,
			text_len: 0
		}
	}

	pub fn clear(&mut self) {
		self.alignment = None;
		self.shape_mode = None;
		self.needs_realign = false;
		self.rem_size = px(0.0);
		self.truncation = None;
		self.text_len = 0;
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
		if self.shape_mode.is_none() {
			panic!("layout must be fit/truncated before align")
		}

		if alignment != self.alignment || self.needs_realign {
			self.layout.align(
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
		if let Some(shape_mode) = &self.shape_mode
			&& *shape_mode == (ShapeMode::Fit { wrap_width: max_width })
		{
			return;
		}

		self.needs_realign = true;

		if let Some(ShapeMode::Fit { wrap_width: Some(wrap_width) }) = self.shape_mode
			&& self.num_lines() == 1
			&& (max_width.is_none() || max_width.is_some_and(|mw| mw >= wrap_width))
		{
			// don't bother re-wrapping if the old text didn't wrap within a smaller bound than the new bound
			// (but still update shape_mode so align works)
			self.shape_mode = Some(ShapeMode::Fit { wrap_width: max_width });
			return;
		}

		self.layout.break_all_lines(max_width.map(|x| x.0));
		self.shape_mode = Some(ShapeMode::Fit { wrap_width: max_width });
	}

	pub fn truncate(
		&mut self,
		text_system: &TextSystem,
		base_style: &TextStyle,
		max_width: ScaledPixels,
		line_clamp: Option<NonZeroU32>,
		affix: &str,
		from: TruncateFrom
	) {
		if let Some(shape_mode) = &self.shape_mode
			&& *shape_mode == (ShapeMode::Truncate { max_width, line_clamp, from })
			&& self.truncation.as_ref().is_none_or(|tr| tr.affix == affix)
		{
			return;
		}

		self.needs_realign = true;

		if let Some(ShapeMode::Truncate {
			max_width: old_max_width,
			line_clamp: old_line_clamp,
			..
		}) = self.shape_mode
			&& (max_width == old_max_width && line_clamp == old_line_clamp)
		{
			return;
		}

		let mut breaker = self.layout.break_lines();
		breaker.state_mut().set_layout_max_advance(max_width.0);
		for _ in 0..line_clamp.map_or(1, |x| x.get()) - 1 {
			breaker.state_mut().set_line_max_advance(max_width.0);
			if breaker.break_next().is_none() {
				break;
			}
		}
		breaker.break_remaining(f32::MAX);

		self.shape_mode = Some(ShapeMode::Truncate { max_width, line_clamp, from });

		if self.width() > max_width {
			self.truncation.get_or_insert_with(Truncation::default).update(
				text_system,
				base_style,
				line_clamp,
				max_width,
				affix,
				from,
				self.rem_size,
				self.layout.scale(),
				&self.layout
			);
		} else {
			self.truncation = None;
		}
	}

	pub fn num_lines(&self) -> usize {
		self.layout.len()
	}

	pub fn width(&self) -> ScaledPixels {
		ScaledPixels(self.layout.width())
	}

	pub fn max_width(&self) -> Option<ScaledPixels> {
		self.shape_mode.as_ref().and_then(|s| s.max_width())
	}

	pub fn height(&self) -> ScaledPixels {
		ScaledPixels(self.layout.height())
	}

	pub fn size(&self) -> Size<ScaledPixels> {
		size(ScaledPixels(self.layout.width()), ScaledPixels(self.layout.height()))
	}

	pub fn lines(&self) -> impl ExactSizeIterator<Item = Line<'_>> + '_ + Clone {
		self.layout.lines().enumerate().map(|(idx, line)| Line {
			line,
			idx: idx as u32,
			truncation: self.truncation.as_ref().filter(|t| t.line() == idx)
		})
	}

	pub fn get(&self, index: usize) -> Option<Line<'_>> {
		self.layout.get(index).map(|line| Line {
			line,
			idx: index as _,
			truncation: self.truncation.as_ref().filter(|t| t.line() == index)
		})
	}

	pub fn cursor_at(&self, point: Point<ScaledPixels>, exact: bool) -> Option<Cursor> {
		let cluster = if exact {
			parley::Cluster::from_point_exact(&self.layout, point.x.0, point.y.0)
		} else {
			parley::Cluster::from_point(&self.layout, point.x.0, point.y.0)
		};
		let (index, affinity) = if let Some((cluster, side)) = cluster {
			let is_leading = side == parley::ClusterSide::Left;
			if cluster.is_rtl() {
				if is_leading {
					(cluster.text_range().end, Affinity::Upstream)
				} else {
					(cluster.text_range().start, Affinity::Downstream)
				}
			} else {
				if is_leading || cluster.is_line_break() == Some(parley::BreakReason::Explicit) {
					(cluster.text_range().start, Affinity::Downstream)
				} else {
					(cluster.text_range().end, Affinity::Upstream)
				}
			}
		} else if !exact {
			(self.text_len, Affinity::Downstream)
		} else {
			return None;
		};
		Some(Cursor::new(index, affinity))
	}

	pub fn cursor_at_byte(&self, byte: usize) -> Cursor {
		Cursor::from_parley(parley::Cursor::from_byte_index(&self.layout, byte, parley::Affinity::Downstream))
	}

	pub(crate) fn line_for_offset(&self, offset: f32) -> Option<usize> {
		if offset < 0.0 {
			return Some(0);
		}

		let lines = self.lines();
		let mut size = lines.len();
		if size == 0 {
			return None;
		}

		let mut base = 0;
		while size > 1 {
			let half = size / 2;
			let mid = base + half;
			let line = self.get(mid)?;
			let cmp = if offset < line.line.metrics().block_min_coord {
				Ordering::Greater
			} else if offset >= line.line.metrics().block_max_coord {
				Ordering::Less
			} else {
				Ordering::Equal
			};
			base = select_unpredictable(cmp == Ordering::Greater, base, mid);
			size -= half;
		}

		let line = self.get(base)?;
		let cmp = if offset < line.line.metrics().block_min_coord {
			Ordering::Greater
		} else if offset >= line.line.metrics().block_max_coord {
			Ordering::Less
		} else {
			Ordering::Equal
		};
		Some(if cmp == Ordering::Equal {
			base
		} else {
			(base + (cmp == Ordering::Less) as usize).saturating_sub(1)
		})
	}
}

#[derive(Default, Clone)]
struct Truncation {
	affix: String,
	line: u32,
	item: Option<u32>,
	from: TruncateFrom,
	start_advance: ScaledPixels,
	rem_size: Pixels,
	scale: f32,
	layout: parley::Layout<Brush>
}

impl Truncation {
	#[inline]
	pub(crate) fn line(&self) -> usize {
		self.line as usize
	}

	#[inline]
	pub(crate) fn item(&self) -> Option<u32> {
		self.item
	}

	pub(crate) fn update(
		&mut self,
		text_system: &TextSystem,
		base_style: &TextStyle,
		line_clamp: Option<NonZeroU32>,
		max_width: ScaledPixels,
		affix: &str,
		from: TruncateFrom,
		rem_size: Pixels,
		scale: f32,
		layout: &parley::Layout<Brush>
	) {
		if affix != self.affix || self.rem_size != rem_size || self.scale != scale {
			self.update_layout(text_system, base_style, affix, rem_size, scale);
		}

		let truncate_width = self.layout.width();

		self.line = line_clamp.map_or(1, |x| x.get()) - 1;
		self.from = from;
		self.item = None;
		self.start_advance = ScaledPixels(0.0);

		let Some(line) = layout.lines().nth(self.line as usize) else {
			return;
		};

		match self.from {
			TruncateFrom::End => {
				let mut width = 0.0;
				let mut item_idx = 0;
				for item in line.items() {
					match item {
						parley::PositionedLayoutItem::GlyphRun(run) => {
							for glyph in run.glyphs() {
								if width + truncate_width < max_width.0 {
									self.item = Some(item_idx);
								}

								width += glyph.advance;
								item_idx += 1;

								if width.floor() > max_width.0 {
									return;
								}
							}
						}
						parley::PositionedLayoutItem::InlineBox(b) => {
							if width + truncate_width < max_width.0 {
								self.item = Some(item_idx);
							}

							width += b.width;
							item_idx += 1;

							if width.floor() > max_width.0 {
								return;
							}
						}
					}
				}
				self.item = None;
			}
			TruncateFrom::Start => {
				let mut item_widths: Vec<f32> = Vec::new();
				for item in line.items() {
					match item {
						parley::PositionedLayoutItem::GlyphRun(run) => {
							for glyph in run.glyphs() {
								item_widths.push(glyph.advance);
							}
						}
						parley::PositionedLayoutItem::InlineBox(b) => {
							item_widths.push(b.width);
						}
					}
				}

				let available = max_width.0 - truncate_width;
				let total: f32 = item_widths.iter().sum();
				let mut skipped = 0.0;

				for (idx, &w) in item_widths.iter().enumerate() {
					if total - skipped <= available {
						self.item = Some(idx as u32);
						self.start_advance = ScaledPixels(skipped);
						return;
					}
					skipped += w;
				}

				self.item = None;
				self.start_advance = ScaledPixels(0.0);
			}
		}
	}

	fn update_layout(&mut self, text_system: &TextSystem, base_style: &TextStyle, affix: &str, rem_size: Pixels, scale: f32) {
		let mut layout = text_system.parley_ctx.layout.lock();
		let mut font = text_system.parley_ctx.font.lock();

		let mut builder = layout.ranged_builder(&mut *font, affix, scale, false);
		apply_base_text_style(base_style, rem_size, &mut builder);
		builder.build_into(&mut self.layout, affix);
		self.layout.break_all_lines(None);

		self.affix = affix.to_owned();
		self.rem_size = rem_size;
		self.scale = scale;
	}

	pub(crate) fn run(&self) -> Option<GlyphRun<'_>> {
		self.layout.lines().next().and_then(|x| x.items().next()).and_then(|r| match r {
			parley::PositionedLayoutItem::GlyphRun(run) => Some(GlyphRun::new(run)),
			_ => None
		})
	}
}

#[derive(Clone)]
pub struct Line<'a> {
	line: parley::Line<'a, Brush>,
	idx: u32,
	truncation: Option<&'a Truncation>
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

		let is_start_truncation = self.truncation.as_ref().is_some_and(|t| t.from == TruncateFrom::Start);
		if is_start_truncation
			&& let Some(truncation) = self.truncation
			&& truncation.item().is_some()
			&& let Some(truncation_run) = truncation.run()
		{
			let font = text_system.font_cache().get(&truncation_run.data().font());
			let affix_origin = Point {
				x: origin.x,
				y: origin.y + ScaledPixels(self.line.metrics().line_height * self.idx as f32)
			};
			for glyph in truncation_run.positioned_glyphs(affix_origin) {
				painter.paint_glyph(glyph, &font, truncation_run.data(), truncation_run.style().brush.color)?;
			}
		}

		let render_origin = if is_start_truncation {
			let (trunc_width, skip_advance) = self
				.truncation
				.map(|t| (ScaledPixels(t.layout.width()), t.start_advance))
				.unwrap_or((ScaledPixels(0.0), ScaledPixels(0.0)));
			Point {
				x: origin.x + trunc_width - skip_advance,
				y: origin.y
			}
		} else {
			origin
		};

		let mut item_idx = 0;
		let mut advance = ScaledPixels(0.0);
		let mut truncated = false;

		'outer: for renderable in self.renderables() {
			match renderable {
				Renderable::GlyphRun(run) => {
					let font = text_system.font_cache().get(run.data().font());
					let mut run_rendered_any = false;

					for glyph in run.positioned_glyphs(render_origin) {
						if !is_start_truncation {
							if self.truncation.as_ref().is_some_and(|t| t.item().is_some_and(|i| i == item_idx)) {
								truncated = true;
								break 'outer;
							}
						} else {
							if self.truncation.as_ref().is_some_and(|t| t.item().is_some_and(|i| item_idx < i)) {
								item_idx += 1;
								continue;
							}
						}

						item_idx += 1;
						advance += glyph.advance;
						run_rendered_any = true;
						painter.paint_glyph(glyph, &font, run.data(), run.style().brush.color)?;
					}

					if !is_start_truncation || run_rendered_any {
						if let Some(underline) = run.style().underline.as_ref() {
							painter.paint_underline(
								render_origin + point(run.x(), run.y() - ScaledPixels(underline.offset.unwrap_or_default())),
								run.width(),
								&UnderlineStyle {
									color: Some(underline.brush.color),
									thickness: px(underline.size.unwrap_or(1.0)),
									wavy: underline.brush.wavy
								}
							);
						}

						if let Some(strikethrough) = run.style().strikethrough.as_ref() {
							painter.paint_strikethrough(
								render_origin + point(run.x(), run.y() - ScaledPixels(strikethrough.offset.unwrap_or_default())),
								run.width(),
								&StrikethroughStyle {
									color: Some(strikethrough.brush.color),
									thickness: px(strikethrough.size.unwrap_or(1.0))
								}
							);
						}
					}
				}
				Renderable::InlineBox { .. } => unimplemented!("inline boxes not implemented")
			}
		}

		if truncated
			&& let Some(truncation) = self.truncation
			&& let Some(truncation_run) = truncation.run()
		{
			let font = text_system.font_cache().get(&truncation_run.data().font());
			let affix_origin = Point {
				x: origin.x + advance,
				y: origin.y + ScaledPixels(self.line.metrics().line_height * self.idx as f32)
			};
			for glyph in truncation_run.positioned_glyphs(affix_origin) {
				painter.paint_glyph(glyph, &font, truncation_run.data(), truncation_run.style().brush.color)?;
			}
		}

		Ok(())
	}

	#[inline]
	pub(crate) fn parley_line(&self) -> &parley::Line<'_, Brush> {
		&self.line
	}
}
