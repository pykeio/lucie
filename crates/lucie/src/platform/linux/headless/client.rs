use std::{cell::RefCell, rc::Rc};

use calloop::{EventLoop, LoopHandle};
use lucie_common::ResultExt as _;

use crate::{
	AnyWindowHandle, CursorStyle, DisplayId, LinuxKeyboardLayout, PlatformDisplay, PlatformKeyboardLayout, WindowParams,
	platform::{LinuxCommon, PlatformWindow, linux::LinuxClient}
};

pub struct HeadlessClientState {
	pub(crate) _loop_handle: LoopHandle<'static, HeadlessClient>,
	pub(crate) event_loop: Option<calloop::EventLoop<'static, HeadlessClient>>,
	pub(crate) common: LinuxCommon
}

#[derive(Clone)]
pub(crate) struct HeadlessClient(Rc<RefCell<HeadlessClientState>>);

impl HeadlessClient {
	pub(crate) fn new() -> Self {
		let event_loop = EventLoop::try_new().unwrap();

		let (common, main_receiver) = LinuxCommon::new(event_loop.get_signal());

		let handle = event_loop.handle();

		handle
			.insert_source(main_receiver, |event, _, _: &mut HeadlessClient| {
				if let calloop::channel::Event::Msg(runnable) = event {
					match runnable {
						crate::RunnableVariant::Meta(runnable) => runnable.run(),
						crate::RunnableVariant::Compat(runnable) => runnable.run()
					};
				}
			})
			.ok();

		HeadlessClient(Rc::new(RefCell::new(HeadlessClientState {
			event_loop: Some(event_loop),
			_loop_handle: handle,
			common
		})))
	}
}

impl LinuxClient for HeadlessClient {
	fn with_common<R>(&self, f: impl FnOnce(&mut LinuxCommon) -> R) -> R {
		f(&mut self.0.borrow_mut().common)
	}

	fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
		Box::new(LinuxKeyboardLayout::new("unknown".into()))
	}

	fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
		vec![]
	}

	fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
		None
	}

	fn display(&self, _id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
		None
	}

	fn active_window(&self) -> Option<AnyWindowHandle> {
		None
	}

	fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
		None
	}

	fn open_window(&self, _handle: AnyWindowHandle, _params: WindowParams) -> anyhow::Result<Box<dyn PlatformWindow>> {
		anyhow::bail!("neither DISPLAY nor WAYLAND_DISPLAY is set. You can run in headless mode");
	}

	fn compositor_name(&self) -> &'static str {
		"headless"
	}

	fn set_cursor_style(&self, _style: CursorStyle) {}

	fn write_to_primary(&self, _item: crate::ClipboardItem) {}

	fn write_to_clipboard(&self, _item: crate::ClipboardItem) {}

	fn read_from_primary(&self) -> Option<crate::ClipboardItem> {
		None
	}

	fn read_from_clipboard(&self) -> Option<crate::ClipboardItem> {
		None
	}

	fn run(&self) {
		let mut event_loop = self.0.borrow_mut().event_loop.take().expect("App is already running");

		event_loop.run(None, &mut self.clone(), |_| {}).log_err();
	}
}
