// Much of this is ripped from Parley itself - Copyright 2025 the Parley Authors, Apache-2.0 license

use std::{mem::swap, ops::Range};

use lucie_common::geometry::{Bounds, Point, ScaledPixels, point, size};

use crate::{layout::Layout, style::Brush};

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Affinity {
	#[default]
	Downstream,
	Upstream
}

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Cursor {
	index: usize,
	affinity: Affinity
}

impl Cursor {
	#[inline]
	pub(crate) fn from_parley(cursor: parley::Cursor) -> Self {
		Self {
			index: cursor.index(),
			affinity: match cursor.affinity() {
				parley::Affinity::Downstream => Affinity::Downstream,
				parley::Affinity::Upstream => Affinity::Upstream
			}
		}
	}

	#[inline]
	pub(crate) fn new(index: usize, affinity: Affinity) -> Self {
		Self { index, affinity }
	}

	#[inline]
	pub fn index(&self) -> usize {
		self.index
	}

	#[inline]
	pub fn affinity(&self) -> Affinity {
		self.affinity
	}

	#[inline]
	pub fn to_selection(&self) -> Selection {
		Selection::new(*self, *self)
	}

	#[inline]
	#[must_use]
	pub fn refresh(&self, layout: &Layout) -> Cursor {
		Self::from_parley(self.to_parley(layout))
	}

	pub(crate) fn to_parley(&self, layout: &Layout) -> parley::Cursor {
		parley::Cursor::from_byte_index(
			layout.layout(),
			self.index,
			match self.affinity {
				Affinity::Downstream => parley::Affinity::Downstream,
				Affinity::Upstream => parley::Affinity::Upstream
			}
		)
	}

	#[inline]
	pub fn position(&self, layout: &Layout) -> Point<ScaledPixels> {
		fn cursor_pos(cluster: &parley::Cluster<'_, Brush>, at_end: bool) -> Point<ScaledPixels> {
			let mut line_x = cluster.visual_offset().unwrap_or_default();
			if at_end {
				line_x += cluster.advance();
			}
			let line = cluster.line();
			let metrics = line.metrics();
			point(ScaledPixels(line_x), ScaledPixels(metrics.block_min_coord))
		}

		fn last_line_cursor_rect(layout: &parley::Layout<Brush>) -> Point<ScaledPixels> {
			if let Some(line) = layout.get(layout.len().saturating_sub(1)) {
				let metrics = line.metrics();
				point(ScaledPixels(metrics.offset), ScaledPixels(metrics.block_min_coord))
			} else {
				Point::default()
			}
		}
		match self.visual_clusters(layout) {
			[Some(left), Some(right)] => {
				if left.is_end_of_line() {
					if left.is_soft_line_break() {
						let (cluster, at_end) =
							if left.is_rtl() && self.affinity == Affinity::Downstream || !left.is_rtl() && self.affinity == Affinity::Upstream {
								(left, true)
							} else {
								(right, false)
							};
						cursor_pos(&cluster, at_end)
					} else {
						cursor_pos(&right, false)
					}
				} else {
					cursor_pos(&left, true)
				}
			}
			[Some(left), None] if left.is_hard_line_break() => last_line_cursor_rect(layout.layout()),
			[Some(left), _] => cursor_pos(&left, true),
			[_, Some(right)] => cursor_pos(&right, false),
			_ => last_line_cursor_rect(layout.layout())
		}
	}

	fn visual_clusters<'a>(&self, layout: &'a Layout) -> [Option<parley::Cluster<'a, Brush>>; 2] {
		if self.affinity == Affinity::Upstream {
			if let Some(cluster) = self.upstream_cluster(layout) {
				if cluster.is_rtl() {
					[cluster.previous_visual(), Some(cluster)]
				} else {
					[Some(cluster.clone()), cluster.next_visual()]
				}
			} else if let Some(cluster) = self.downstream_cluster(layout) {
				if cluster.is_rtl() { [None, Some(cluster)] } else { [Some(cluster), None] }
			} else {
				[None, None]
			}
		} else if let Some(cluster) = self.downstream_cluster(layout) {
			if cluster.is_rtl() {
				[Some(cluster.clone()), cluster.next_visual()]
			} else {
				[cluster.previous_visual(), Some(cluster)]
			}
		} else if let Some(cluster) = self.upstream_cluster(layout) {
			if cluster.is_rtl() { [None, Some(cluster)] } else { [Some(cluster), None] }
		} else {
			[None, None]
		}
	}

	pub(crate) fn upstream_cluster<'a>(&self, layout: &'a Layout) -> Option<parley::Cluster<'a, Brush>> {
		self.index
			.checked_sub(1)
			.and_then(|index| parley::Cluster::from_byte_index(layout.layout(), index))
	}

	pub(crate) fn downstream_cluster<'a>(&self, layout: &'a Layout) -> Option<parley::Cluster<'a, Brush>> {
		parley::Cluster::from_byte_index(layout.layout(), self.index)
	}

	pub(crate) fn line(self, layout: &Layout) -> Option<usize> {
		let pos = self.position(layout);
		layout.line_for_offset(pos.y.0)
	}
}

#[derive(Debug, Default, Copy, Clone)]
pub struct Selection {
	anchor: Cursor,
	focus: Cursor,
	h_pos: Option<f32>
}

impl Selection {
	pub fn new(anchor: Cursor, focus: Cursor) -> Self {
		Selection { anchor, focus, h_pos: None }
	}

	pub fn from_range(range: Range<usize>, layout: &Layout) -> Self {
		let start = layout.cursor_at_byte(range.start);
		let end = layout.cursor_at_byte(range.end);
		Selection::new(start, end)
	}

	#[inline]
	pub fn is_collapsed(&self) -> bool {
		self.anchor.index == self.focus.index
	}

	#[inline]
	pub fn anchor(&self) -> Cursor {
		self.anchor.clone()
	}

	#[inline]
	pub fn focus(&self) -> Cursor {
		self.focus.clone()
	}

	#[inline]
	pub fn collapse(&self) -> Selection {
		self.focus.to_selection()
	}

	#[inline]
	pub fn text_range(&self) -> Range<usize> {
		let start = self.anchor.index().min(self.focus.index());
		let end = self.focus.index().max(self.anchor.index());
		start..end
	}

	#[inline]
	#[must_use]
	pub fn refresh(&self, layout: &Layout) -> Self {
		let anchor = self.anchor.refresh(layout);
		let focus = self.focus.refresh(layout);
		let h_pos = self.h_pos;
		Self { anchor, focus, h_pos }
	}

	#[inline]
	pub fn bounds(&self, layout: &Layout) -> Vec<(usize, Bounds<ScaledPixels>)> {
		let mut xs = vec![];
		self.geometry_with(layout, |bb, line| {
			xs.push((line, bb));
		});
		xs
	}

	fn geometry_with(&self, layout: &Layout, mut f: impl FnMut(Bounds<ScaledPixels>, usize)) {
		const NEWLINE_WHITESPACE_WIDTH_RATIO: f32 = 0.25;
		if self.is_collapsed() {
			return;
		}
		let mut start = self.anchor;
		let mut end = self.focus;
		if start.index > end.index {
			swap(&mut start, &mut end);
		}
		let text_range = start.index..end.index;
		let line_start_ix = start.line(layout).unwrap_or(0);
		let line_end_ix = end.line(layout).unwrap_or(layout.num_lines() + 1);
		for line_ix in line_start_ix..=line_end_ix {
			let Some(line) = layout.get(line_ix) else {
				continue;
			};
			let line = line.parley_line();
			let metrics = line.metrics();
			let line_min = metrics.block_min_coord;
			let line_max = metrics.block_max_coord;
			let newline_whitespace = if line.break_reason() == parley::BreakReason::Explicit {
				(metrics.ascent + metrics.descent) * NEWLINE_WHITESPACE_WIDTH_RATIO
			} else {
				0.0
			};
			if line_ix == line_start_ix || line_ix == line_end_ix {
				let mut start_x = metrics.offset + metrics.inline_min_coord;
				let mut cur_x = start_x;
				let mut cluster_count = 0;
				let mut box_advance = 0.0;
				let mut have_seen_any_runs = false;
				for item in line.items() {
					match item {
						parley::PositionedLayoutItem::GlyphRun(run) => {
							have_seen_any_runs = true;
							for cluster in run.run().visual_clusters() {
								let advance = cluster.advance() + box_advance;
								box_advance = 0.0;
								if text_range.contains(&cluster.text_range().start) {
									cluster_count += 1;
									cur_x += advance;
								} else {
									if cur_x != start_x {
										f(
											Bounds {
												origin: point(ScaledPixels(start_x), ScaledPixels(line_min)),
												size: size(ScaledPixels(cur_x - start_x), ScaledPixels(line_max - line_min))
											},
											line_ix
										);
									}
									cur_x += advance;
									start_x = cur_x;
								}
							}
						}
						parley::PositionedLayoutItem::InlineBox(inline_box) => {
							box_advance += inline_box.width;
							if !have_seen_any_runs {
								cur_x += box_advance;
								box_advance = 0.0;
								start_x = cur_x;
							}
						}
					}
				}
				let mut end_x = cur_x;
				if line_ix != line_end_ix || (cluster_count != 0 && metrics.advance == 0.0) {
					end_x += newline_whitespace;
				}
				if end_x != start_x {
					f(
						Bounds {
							origin: point(ScaledPixels(start_x), ScaledPixels(line_min)),
							size: size(ScaledPixels(end_x - start_x), ScaledPixels(line_max - line_min))
						},
						line_ix
					);
				}
			} else {
				let x = metrics.offset + metrics.inline_min_coord;
				let width = metrics.advance;
				f(
					Bounds {
						origin: point(ScaledPixels(x), ScaledPixels(line_min)),
						size: size(ScaledPixels(width + newline_whitespace), ScaledPixels(line_max - line_min))
					},
					line_ix
				);
			}
		}
	}
}
