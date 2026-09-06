use std::{
	borrow::Cow,
	cell::{Cell, RefCell},
	hash::{Hash, Hasher},
	mem,
	ops::Range,
	rc::Rc
};

use lucie_common::{
	SharedString,
	geometry::{Bounds, Pixels, Point, ScaledPixels}
};
use lucie_style::{CursorStyle, HighlightStyle, TextOverflow, TextRun, TextStyle, WhiteSpace};
use lucie_text::TruncateFrom;
use rapidhash::fast::RapidHasher;

use crate::{
	ActiveTooltip, AnyView, App, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, IntoElement, LayoutId, MouseDownEvent,
	MouseMoveEvent, MouseUpEvent, TooltipId, Window, register_tooltip_mouse_handlers, set_tooltip_on_window
};

impl Element for &'static str {
	type RequestLayoutState = TextLayout;
	type PrepaintState = ();

	fn id(&self) -> Option<ElementId> {
		None
	}

	fn request_layout(&mut self, _id: Option<&GlobalElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
		let mut state = TextLayout::default();
		let layout_id = state.layout(SharedString::from(*self), None, window, cx);
		(layout_id, state)
	}

	fn prepaint(
		&mut self,
		_id: Option<&GlobalElementId>,
		_bounds: Bounds<Pixels>,
		_text_layout: &mut Self::RequestLayoutState,
		_window: &mut Window,
		_cx: &mut App
	) {
	}

	fn paint(&mut self, _id: Option<&GlobalElementId>, bounds: Bounds<Pixels>, text_layout: &mut TextLayout, _: &mut (), window: &mut Window, cx: &mut App) {
		text_layout.paint(bounds, window, cx)
	}
}

impl IntoElement for &'static str {
	type Element = Self;

	fn into_element(self) -> Self::Element {
		self
	}
}

impl IntoElement for String {
	type Element = SharedString;

	fn into_element(self) -> Self::Element {
		self.into()
	}
}

impl IntoElement for Cow<'static, str> {
	type Element = SharedString;

	fn into_element(self) -> Self::Element {
		self.into()
	}
}

impl Element for SharedString {
	type RequestLayoutState = TextLayout;
	type PrepaintState = ();

	fn id(&self) -> Option<ElementId> {
		None
	}

	fn request_layout(&mut self, _id: Option<&GlobalElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
		let mut state = TextLayout::default();
		let layout_id = state.layout(self.clone(), None, window, cx);
		(layout_id, state)
	}

	fn prepaint(
		&mut self,
		_id: Option<&GlobalElementId>,
		_bounds: Bounds<Pixels>,
		_text_layout: &mut Self::RequestLayoutState,
		_window: &mut Window,
		_cx: &mut App
	) {
	}

	fn paint(
		&mut self,
		_id: Option<&GlobalElementId>,
		bounds: Bounds<Pixels>,
		text_layout: &mut Self::RequestLayoutState,
		_: &mut Self::PrepaintState,
		window: &mut Window,
		cx: &mut App
	) {
		text_layout.paint(bounds, window, cx)
	}
}

impl IntoElement for SharedString {
	type Element = Self;

	fn into_element(self) -> Self::Element {
		self
	}
}

/// Renders text with runs of different styles.
///
/// Callers are responsible for setting the correct style for each run.
/// For text with a uniform style, you can usually avoid calling this constructor
/// and just pass text directly.
pub struct StyledText {
	text: SharedString,
	runs: Option<Vec<TextRun>>,
	delayed_highlights: Option<Vec<(Range<usize>, HighlightStyle)>>,
	layout: TextLayout
}

impl StyledText {
	/// Construct a new styled text element from the given string.
	pub fn new(text: impl Into<SharedString>) -> Self {
		StyledText {
			text: text.into(),
			runs: None,
			delayed_highlights: None,
			layout: TextLayout::default()
		}
	}

	/// Get the layout for this element. This can be used to map indices to pixels and vice versa.
	pub fn layout(&self) -> &TextLayout {
		&self.layout
	}

	/// Set the styling attributes for the given text, as well as
	/// as any ranges of text that have had their style customized.
	pub fn with_default_highlights(mut self, default_style: &TextStyle, highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>) -> Self {
		debug_assert!(self.delayed_highlights.is_none(), "Can't use `with_default_highlights` and `with_highlights`");
		let runs = Self::compute_runs(&self.text, default_style, highlights);
		self.with_runs(runs)
	}

	/// Set the styling attributes for the given text, as well as
	/// as any ranges of text that have had their style customized.
	pub fn with_highlights(mut self, highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>) -> Self {
		debug_assert!(self.runs.is_none(), "Can't use `with_highlights` and `with_default_highlights`");
		self.delayed_highlights = Some(
			highlights
				.into_iter()
				.inspect(|(run, _)| {
					debug_assert!(self.text.is_char_boundary(run.start));
					debug_assert!(self.text.is_char_boundary(run.end));
				})
				.collect::<Vec<_>>()
		);
		self
	}

	fn compute_runs(text: &str, default_style: &TextStyle, highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>) -> Vec<TextRun> {
		let mut runs = Vec::new();
		let mut ix = 0;
		for (range, highlight) in highlights {
			if ix < range.start {
				debug_assert!(text.is_char_boundary(range.start));
				runs.push(default_style.clone().to_run(range.start - ix));
			}
			debug_assert!(text.is_char_boundary(range.end));
			runs.push(default_style.clone().highlight(highlight).to_run(range.len()));
			ix = range.end;
		}
		if ix < text.len() {
			runs.push(default_style.to_run(text.len() - ix));
		}
		runs
	}

	/// Set the text runs for this piece of text.
	pub fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
		let mut text = &**self.text;
		for run in &runs {
			text = text.get(run.len..).expect("invalid text run");
		}
		assert!(text.is_empty(), "invalid text run");
		self.runs = Some(runs);
		self
	}
}

impl Element for StyledText {
	type RequestLayoutState = ();
	type PrepaintState = ();

	fn id(&self) -> Option<ElementId> {
		None
	}

	fn request_layout(&mut self, _id: Option<&GlobalElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
		let runs = self.runs.take().or_else(|| {
			self.delayed_highlights
				.take()
				.map(|delayed_highlights| Self::compute_runs(&self.text, &window.text_style(), delayed_highlights))
		});

		let layout_id = self.layout.layout(self.text.clone(), runs, window, cx);
		(layout_id, ())
	}

	fn prepaint(&mut self, _id: Option<&GlobalElementId>, _bounds: Bounds<Pixels>, _: &mut Self::RequestLayoutState, _window: &mut Window, _cx: &mut App) {}

	fn paint(
		&mut self,
		_id: Option<&GlobalElementId>,
		bounds: Bounds<Pixels>,
		_: &mut Self::RequestLayoutState,
		_: &mut Self::PrepaintState,
		window: &mut Window,
		cx: &mut App
	) {
		self.layout.paint(bounds, window, cx)
	}
}

impl IntoElement for StyledText {
	type Element = Self;

	fn into_element(self) -> Self::Element {
		self
	}
}

/// The Layout for TextElement. This can be used to map indices to pixels and vice versa.
#[derive(Default, Clone)]
pub struct TextLayout(Rc<RefCell<Option<TextLayoutInner>>>);

struct TextLayoutInner {
	text: SharedString,
	layout: Rc<RefCell<lucie_text::Layout>>
}

impl TextLayout {
	fn layout(&self, text: SharedString, runs: Option<Vec<TextRun>>, window: &mut Window, _: &mut App) -> LayoutId {
		let text_style = window.text_style();
		let font_size = text_style.font_size.to_pixels(window.rem_size());
		let scale_factor = window.scale_factor();

		let runs = if let Some(runs) = runs { runs } else { vec![text_style.to_run(text.len())] };
		window.request_measured_layout(Default::default(), {
			let element_state = self.clone();

			move |known_dimensions, available_space, window, cx| {
				let wrap_width = if text_style.white_space == WhiteSpace::Normal {
					known_dimensions.width.or(match available_space.width {
						crate::AvailableSpace::Definite(x) => Some(x),
						_ => None
					})
				} else {
					None
				};

				window.with_element_state(&global_id(&text, &text_style, scale_factor, Some(&runs)), |state, _| {
					let text_system = cx.text_system();
					let mut layout = state.unwrap_or_else(|| {
						let mut builder = cx.text_system().ranged_builder(&text, font_size, scale_factor, &text_style);
						builder.push_runs(&runs);
						// TODO: would be nice to reuse Layout allocations from dead layouts
						Rc::new(RefCell::new(builder.build(&text)))
					});

					{
						let layout_ref = layout.borrow();
						if layout_ref.num_lines() > 0 && (wrap_width.is_none() || wrap_width.map(|p| p.scale(scale_factor)) == layout_ref.max_width()) {
							let size = layout_ref.size().map(|x| Pixels(x.0 / scale_factor));
							drop(layout_ref);
							element_state.0.borrow_mut().replace(TextLayoutInner {
								text: text.clone(),
								layout: Rc::clone(&layout)
							});

							return (size, layout);
						}
					}

					let size = {
						let mut layout = layout.borrow_mut();
						if let Some(overflow) = &text_style.text_overflow {
							let (affix, from) = match overflow {
								TextOverflow::Truncate(affix) => (affix, TruncateFrom::End),
								TextOverflow::TruncateStart(affix) => (affix, TruncateFrom::Start)
							};

							layout.truncate(
								&*text_system,
								&text_style,
								known_dimensions
									.width
									.or(match available_space.width {
										crate::AvailableSpace::Definite(x) => Some(x),
										_ => None
									})
									.map(|p| p.scale(scale_factor))
									.unwrap_or(ScaledPixels(f32::MAX)),
								text_style.line_clamp,
								affix,
								from
							);
						} else {
							layout.fit(wrap_width.map(|p| p.scale(scale_factor)));
						}
						layout.align(Some(text_style.text_align));
						layout.size().map(|x| Pixels(x.0 / scale_factor))
					};

					element_state.0.borrow_mut().replace(TextLayoutInner {
						text: text.clone(),
						layout: Rc::clone(&layout)
					});

					(size, layout)
				})
			}
		})
	}

	fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, _cx: &mut App) {
		let element_state = self.0.borrow();
		let element_state = element_state.as_ref().expect("measurement has not been performed yet");

		let scale_factor = window.scale_factor();
		let line_origin = bounds.origin.scale(scale_factor);
		let text_system = window.text_system().clone();
		let layout = element_state.layout.borrow();
		for line in layout.lines() {
			line.paint(&text_system, window, line_origin).unwrap();
		}
	}

	/// Get the byte index into the input of the pixel position.
	pub fn index_for_position(&self, position: Point<ScaledPixels>, exact: bool) -> Option<usize> {
		let element_state = self.0.borrow();
		let element_state = element_state.as_ref().expect("measurement has not been performed yet");
		element_state.layout.borrow().cursor_at(position, exact).map(|c| c.index())
	}

	/// Get the pixel position for the given byte index.
	pub fn position_for_index(&self, index: usize) -> Point<ScaledPixels> {
		let element_state = self.0.borrow();
		let element_state = element_state.as_ref().expect("measurement has not been performed yet");
		let layout = element_state.layout.borrow();
		layout.cursor_at_byte(index).position(&*layout)
	}

	/// The UTF-8 length of the underlying text.
	pub fn len(&self) -> usize {
		self.0.borrow().as_ref().unwrap().text.len()
	}

	/// The text for this layout.
	pub fn text(&self) -> SharedString {
		self.0.borrow().as_ref().unwrap().text.clone()
	}

	/// The text for this layout (with soft-wraps as newlines)
	pub fn wrapped_text(&self) -> String {
		let mut accumulator = String::new();
		let inner = self.0.borrow();
		let inner = inner.as_ref().unwrap();
		let layout = inner.layout.borrow();
		for wrapped in layout.lines() {
			accumulator.push_str(&inner.text[wrapped.text_range()]);
			accumulator.push('\n');
		}
		// Remove trailing newline
		accumulator.pop();
		accumulator
	}
}

/// A text element that can be interacted with.
pub struct InteractiveText {
	element_id: ElementId,
	text: StyledText,
	click_listener: Option<Box<dyn Fn(&[Range<usize>], InteractiveTextClickEvent, &mut Window, &mut App)>>,
	hover_listener: Option<Box<dyn Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App)>>,
	tooltip_builder: Option<Rc<dyn Fn(usize, &mut Window, &mut App) -> Option<AnyView>>>,
	tooltip_id: Option<TooltipId>,
	clickable_ranges: Vec<Range<usize>>
}

struct InteractiveTextClickEvent {
	mouse_down_index: usize,
	mouse_up_index: usize
}

#[doc(hidden)]
#[derive(Default)]
pub struct InteractiveTextState {
	mouse_down_index: Rc<Cell<Option<usize>>>,
	hovered_index: Rc<Cell<Option<usize>>>,
	active_tooltip: Rc<RefCell<Option<ActiveTooltip>>>
}

/// InteractiveTest is a wrapper around StyledText that adds mouse interactions.
impl InteractiveText {
	/// Creates a new InteractiveText from the given text.
	pub fn new(id: impl Into<ElementId>, text: StyledText) -> Self {
		Self {
			element_id: id.into(),
			text,
			click_listener: None,
			hover_listener: None,
			tooltip_builder: None,
			tooltip_id: None,
			clickable_ranges: Vec::new()
		}
	}

	/// on_click is called when the user clicks on one of the given ranges, passing the index of
	/// the clicked range.
	pub fn on_click(mut self, ranges: Vec<Range<usize>>, listener: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
		self.click_listener = Some(Box::new(move |ranges, event, window, cx| {
			for (range_ix, range) in ranges.iter().enumerate() {
				if range.contains(&event.mouse_down_index) && range.contains(&event.mouse_up_index) {
					listener(range_ix, window, cx);
				}
			}
		}));
		self.clickable_ranges = ranges;
		self
	}

	/// on_hover is called when the mouse moves over a character within the text, passing the
	/// index of the hovered character, or None if the mouse leaves the text.
	pub fn on_hover(mut self, listener: impl Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App) + 'static) -> Self {
		self.hover_listener = Some(Box::new(listener));
		self
	}

	/// tooltip lets you specify a tooltip for a given character index in the string.
	pub fn tooltip(mut self, builder: impl Fn(usize, &mut Window, &mut App) -> Option<AnyView> + 'static) -> Self {
		self.tooltip_builder = Some(Rc::new(builder));
		self
	}
}

impl Element for InteractiveText {
	type RequestLayoutState = ();
	type PrepaintState = Hitbox;

	fn id(&self) -> Option<ElementId> {
		Some(self.element_id.clone())
	}

	fn request_layout(&mut self, _id: Option<&GlobalElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
		self.text.request_layout(None, window, cx)
	}

	fn prepaint(
		&mut self,
		global_id: Option<&GlobalElementId>,
		bounds: Bounds<Pixels>,
		state: &mut Self::RequestLayoutState,
		window: &mut Window,
		cx: &mut App
	) -> Hitbox {
		window.with_optional_element_state::<InteractiveTextState, _>(global_id, |interactive_state, window| {
			let mut interactive_state = interactive_state.map(|interactive_state| interactive_state.unwrap_or_default());

			if let Some(interactive_state) = interactive_state.as_mut() {
				if self.tooltip_builder.is_some() {
					self.tooltip_id = set_tooltip_on_window(&interactive_state.active_tooltip, window);
				} else {
					// If there is no longer a tooltip builder, remove the active tooltip.
					interactive_state.active_tooltip.take();
				}
			}

			self.text.prepaint(None, bounds, state, window, cx);
			let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
			(hitbox, interactive_state)
		})
	}

	fn paint(
		&mut self,
		global_id: Option<&GlobalElementId>,
		bounds: Bounds<Pixels>,
		_: &mut Self::RequestLayoutState,
		hitbox: &mut Hitbox,
		window: &mut Window,
		cx: &mut App
	) {
		let current_view = window.current_view();
		let scale_factor = window.scale_factor();
		let text_layout = self.text.layout().clone();
		window.with_element_state::<InteractiveTextState, _>(global_id.unwrap(), |interactive_state, window| {
			let mut interactive_state = interactive_state.unwrap_or_default();
			if let Some(click_listener) = self.click_listener.take() {
				let mouse_position = window.mouse_position().scale(scale_factor);
				if let Some(ix) = text_layout.index_for_position(mouse_position, true)
					&& self.clickable_ranges.iter().any(|range| range.contains(&ix))
				{
					window.set_cursor_style(CursorStyle::PointingHand, hitbox)
				}

				let text_layout = text_layout.clone();
				let mouse_down = interactive_state.mouse_down_index.clone();
				if let Some(mouse_down_index) = mouse_down.get() {
					let hitbox = hitbox.clone();
					let clickable_ranges = mem::take(&mut self.clickable_ranges);
					window.on_mouse_event(move |event: &MouseUpEvent, phase, window: &mut Window, cx| {
						if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
							if let Some(mouse_up_index) = text_layout.index_for_position(event.position.scale(scale_factor), true) {
								click_listener(&clickable_ranges, InteractiveTextClickEvent { mouse_down_index, mouse_up_index }, window, cx)
							}

							mouse_down.take();
							window.refresh();
						}
					});
				} else {
					let hitbox = hitbox.clone();
					window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
						if phase == DispatchPhase::Bubble
							&& hitbox.is_hovered(window)
							&& let Some(mouse_down_index) = text_layout.index_for_position(event.position.scale(scale_factor), true)
						{
							mouse_down.set(Some(mouse_down_index));
							window.refresh();
						}
					});
				}
			}

			window.on_mouse_event({
				let mut hover_listener = self.hover_listener.take();
				let hitbox = hitbox.clone();
				let text_layout = text_layout.clone();
				let hovered_index = interactive_state.hovered_index.clone();
				move |event: &MouseMoveEvent, phase, window, cx| {
					if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
						let current = hovered_index.get();
						let updated = text_layout.index_for_position(event.position.scale(scale_factor), true);
						if current != updated {
							hovered_index.set(updated);
							if let Some(hover_listener) = hover_listener.as_ref() {
								hover_listener(updated, event.clone(), window, cx);
							}
							cx.notify(current_view);
						}
					}
				}
			});

			if let Some(tooltip_builder) = self.tooltip_builder.clone() {
				let active_tooltip = interactive_state.active_tooltip.clone();
				let build_tooltip = Rc::new({
					let tooltip_is_hoverable = false;
					let text_layout = text_layout.clone();
					move |window: &mut Window, cx: &mut App| {
						text_layout
							.index_for_position(window.mouse_position().scale(scale_factor), true)
							.and_then(|position| tooltip_builder(position, window, cx))
							.map(|view| (view, tooltip_is_hoverable))
					}
				});

				// Use bounds instead of testing hitbox since this is called during prepaint.
				let check_is_hovered_during_prepaint = Rc::new({
					let source_bounds = hitbox.bounds;
					let text_layout = text_layout.clone();
					let pending_mouse_down = interactive_state.mouse_down_index.clone();
					move |window: &Window| {
						text_layout
							.index_for_position(window.mouse_position().scale(scale_factor), true)
							.is_some() && source_bounds.contains(&window.mouse_position())
							&& pending_mouse_down.get().is_none()
					}
				});

				let check_is_hovered = Rc::new({
					let hitbox = hitbox.clone();
					let text_layout = text_layout.clone();
					let pending_mouse_down = interactive_state.mouse_down_index.clone();
					move |window: &Window| {
						text_layout
							.index_for_position(window.mouse_position().scale(scale_factor), true)
							.is_some() && hitbox.is_hovered(window)
							&& pending_mouse_down.get().is_none()
					}
				});

				register_tooltip_mouse_handlers(&active_tooltip, self.tooltip_id, build_tooltip, check_is_hovered, check_is_hovered_during_prepaint, window);
			}

			self.text.paint(None, bounds, &mut (), &mut (), window, cx);

			((), interactive_state)
		});
	}
}

impl IntoElement for InteractiveText {
	type Element = Self;

	fn into_element(self) -> Self::Element {
		self
	}
}

fn global_id(text: &str, text_style: &TextStyle, scale_factor: f32, runs: Option<&[TextRun]>) -> GlobalElementId {
	let mut hasher = const { RapidHasher::new(1282529) };
	scale_factor.to_bits().hash(&mut hasher);
	text_style.font_size.hash(&mut hasher);
	text_style.white_space.hash(&mut hasher);
	text_style.line_clamp.hash(&mut hasher);
	text_style.text_overflow.hash(&mut hasher);
	text.hash(&mut hasher);
	if let Some(runs) = runs {
		for run in runs {
			run.hash(&mut hasher);
		}
	}
	GlobalElementId::from_raw(hasher.finish())
}
