use std::path::PathBuf;

use lucie::{App, Application, Context, Render, Window, WindowOptions, div, img, prelude::*};

struct GifViewer {
	gif_path: PathBuf
}

impl GifViewer {
	fn new(gif_path: PathBuf) -> Self {
		Self { gif_path }
	}
}

impl Render for GifViewer {
	fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
		div()
			.size_full()
			.child(img(self.gif_path.clone()).size_full().object_fit(lucie::ObjectFit::Contain).id("gif"))
	}
}

fn main() {
	tracing_subscriber::fmt::init();
	Application::new().run(|cx: &mut App| {
		let gif_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/image/black-cat-typing.gif");

		cx.open_window(WindowOptions { focus: true, ..Default::default() }, |_, cx| cx.new(|_| GifViewer::new(gif_path)))
			.unwrap();
		cx.activate(true);
	});
}
