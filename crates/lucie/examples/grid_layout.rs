use lucie::{App, Application, Context, Hsla, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size};

// https://en.wikipedia.org/wiki/Holy_grail_(web_design)
struct HolyGrailExample {}

impl Render for HolyGrailExample {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		let block = |color: Hsla| {
			div()
				.size_full()
				.bg(color)
				.border_1()
				.border_dashed()
				.rounded_md()
				.border_color(lucie::white())
				.items_center()
		};

		div()
			.gap_1()
			.grid()
			.bg(rgb(0x505050))
			.size(px(500.0))
			.shadow_lg()
			.border_1()
			.size_full()
			.grid_cols(5)
			.grid_rows(5)
			.child(block(lucie::white()).row_span(1).col_span_full().child("Header"))
			.child(block(lucie::red()).col_span(1).h_56().child("Table of contents"))
			.child(block(lucie::green()).col_span(3).row_span(3).child("Content"))
			.child(block(lucie::blue()).col_span(1).row_span(3).child("AD :(").text_color(lucie::white()))
			.child(
				block(lucie::black())
					.row_span(1)
					.col_span_full()
					.text_color(lucie::white())
					.child("Footer")
			)
	}
}

fn main() {
	Application::new().run(|cx: &mut App| {
		cx.open_window(
			WindowOptions {
				window_bounds: Some(WindowBounds::centered(size(px(500.), px(500.0)), cx)),
				..Default::default()
			},
			|_, cx| cx.new(|_| HolyGrailExample {})
		)
		.unwrap();
		cx.activate(true);
	});
}
