use std::rc::Rc;

pub use lucie::Platform;

pub fn application() -> lucie::Application {
	lucie::Application::new(current_platform(false))
}

pub fn headless() -> lucie::Application {
	lucie::Application::new(current_platform(true))
}

pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
	cfg_select! {
		windows => Rc::new(lucie_windows::WindowsPlatform::new(headless).expect("failed to initialize platform")),
		_ => compile_error!("Lucie does not currently support this platform")
	}
}
