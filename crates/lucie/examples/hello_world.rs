use lucie::{App, Application, Context, SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size};

struct HelloWorld {
	text: SharedString
}

impl Render for HelloWorld {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div()
			.flex()
			.flex_col()
			.gap_3()
			.bg(rgb(0x505050))
			.size(px(500.0))
			.justify_center()
			.items_center()
			.shadow_lg()
			.border_1()
			.border_color(rgb(0x0000ff))
			.text_xl()
			.text_color(rgb(0xffffff))
			.child(format!("Hello, {}!", &self.text))
			.child(
				div()
					.flex()
					.gap_2()
					.child(
						div()
							.size_8()
							.bg(lucie::red())
							.border_1()
							.border_dashed()
							.rounded_md()
							.border_color(lucie::white())
					)
					.child(
						div()
							.size_8()
							.bg(lucie::green())
							.border_1()
							.border_dashed()
							.rounded_md()
							.border_color(lucie::white())
					)
					.child(
						div()
							.size_8()
							.bg(lucie::blue())
							.border_1()
							.border_dashed()
							.rounded_md()
							.border_color(lucie::white())
					)
					.child(
						div()
							.size_8()
							.bg(lucie::yellow())
							.border_1()
							.border_dashed()
							.rounded_md()
							.border_color(lucie::white())
					)
					.child(
						div()
							.size_8()
							.bg(lucie::black())
							.border_1()
							.border_dashed()
							.rounded_md()
							.rounded_md()
							.border_color(lucie::white())
					)
					.child(
						div()
							.size_8()
							.bg(lucie::white())
							.border_1()
							.border_dashed()
							.rounded_md()
							.border_color(lucie::black())
					)
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
			|_, cx| cx.new(|_| HelloWorld { text: "World".into() })
		)
		.unwrap();
		cx.activate(true);
	});
}
