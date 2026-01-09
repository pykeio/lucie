use std::{
	cell::RefCell,
	collections::VecDeque,
	path::Path,
	rc::{Rc, Weak},
	sync::Arc
};

use anyhow::Result;
use futures_channel::oneshot;
use lucie_style::CursorStyle;
use lucie_text::{NoopTextSystem, PlatformTextSystem};
use parking_lot::Mutex;
#[cfg(target_os = "windows")]
use windows::Win32::{
	Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory},
	System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance}
};

use crate::{
	AnyWindowHandle, BackgroundExecutor, ClipboardItem, DummyKeyboardMapper, ForegroundExecutor, Keymap, Platform, PlatformDisplay, PlatformKeyboardLayout,
	PlatformKeyboardMapper, PromptButton, TestDisplay, TestWindow, WindowAppearance, WindowParams
};

/// TestPlatform implements the Platform trait for use in tests.
pub(crate) struct TestPlatform {
	background_executor: BackgroundExecutor,
	foreground_executor: ForegroundExecutor,

	pub(crate) active_window: RefCell<Option<TestWindow>>,
	active_display: Rc<dyn PlatformDisplay>,
	active_cursor: Mutex<CursorStyle>,
	current_clipboard_item: Mutex<Option<ClipboardItem>>,
	#[cfg(any(target_os = "linux", target_os = "freebsd"))]
	current_primary_item: Mutex<Option<ClipboardItem>>,
	prompts: RefCell<VecDeque<TestPrompt>>,
	pub text_system: Arc<dyn PlatformTextSystem>,
	#[cfg(target_os = "windows")]
	bitmap_factory: std::mem::ManuallyDrop<IWICImagingFactory>,
	weak: Weak<Self>
}

struct TestPrompt {
	msg: String,
	detail: Option<String>,
	answers: Vec<String>,
	tx: oneshot::Sender<usize>
}

impl TestPlatform {
	pub fn new(executor: BackgroundExecutor, foreground_executor: ForegroundExecutor) -> Rc<Self> {
		#[cfg(target_os = "windows")]
		let bitmap_factory = unsafe {
			windows::Win32::System::Ole::OleInitialize(None).expect("unable to initialize Windows OLE");
			std::mem::ManuallyDrop::new(CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).expect("Error creating bitmap factory."))
		};

		let text_system = Arc::new(NoopTextSystem);

		Rc::new_cyclic(|weak| TestPlatform {
			background_executor: executor,
			foreground_executor,
			prompts: Default::default(),
			active_cursor: Default::default(),
			active_display: Rc::new(TestDisplay::new()),
			active_window: Default::default(),
			current_clipboard_item: Mutex::new(None),
			#[cfg(any(target_os = "linux", target_os = "freebsd"))]
			current_primary_item: Mutex::new(None),
			weak: weak.clone(),
			#[cfg(target_os = "windows")]
			bitmap_factory,
			text_system
		})
	}

	#[track_caller]
	pub(crate) fn simulate_prompt_answer(&self, response: &str) {
		let prompt = self.prompts.borrow_mut().pop_front().expect("no pending multiple choice prompt");
		self.background_executor().set_waiting_hint(None);
		let Some(ix) = prompt.answers.iter().position(|a| a == response) else {
			panic!("PROMPT: {}\n{:?}\n{:?}\nCannot respond with {}", prompt.msg, prompt.detail, prompt.answers, response)
		};
		prompt.tx.send(ix).ok();
	}

	pub(crate) fn has_pending_prompt(&self) -> bool {
		!self.prompts.borrow().is_empty()
	}

	pub(crate) fn pending_prompt(&self) -> Option<(String, String)> {
		let prompts = self.prompts.borrow();
		let prompt = prompts.front()?;
		Some((prompt.msg.clone(), prompt.detail.clone().unwrap_or_default()))
	}

	pub(crate) fn prompt(&self, msg: &str, detail: Option<&str>, answers: &[PromptButton]) -> oneshot::Receiver<usize> {
		let (tx, rx) = oneshot::channel();
		let answers: Vec<String> = answers.iter().map(|s| s.label().to_string()).collect();
		self.background_executor()
			.set_waiting_hint(Some(format!("PROMPT: {:?} {:?}", msg, detail)));
		self.prompts.borrow_mut().push_back(TestPrompt {
			msg: msg.to_string(),
			detail: detail.map(|s| s.to_string()),
			answers,
			tx
		});
		rx
	}

	pub(crate) fn set_active_window(&self, window: Option<TestWindow>) {
		let executor = self.foreground_executor();
		let previous_window = self.active_window.borrow_mut().take();
		self.active_window.borrow_mut().clone_from(&window);

		executor
			.spawn(async move {
				if let Some(previous_window) = previous_window {
					if let Some(window) = window.as_ref()
						&& Rc::ptr_eq(&previous_window.0, &window.0)
					{
						return;
					}
					previous_window.simulate_active_status_change(false);
				}
				if let Some(window) = window {
					window.simulate_active_status_change(true);
				}
			})
			.detach();
	}
}

impl Platform for TestPlatform {
	fn background_executor(&self) -> BackgroundExecutor {
		self.background_executor.clone()
	}

	fn foreground_executor(&self) -> ForegroundExecutor {
		self.foreground_executor.clone()
	}

	fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
		self.text_system.clone()
	}

	fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
		Box::new(TestKeyboardLayout)
	}

	fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
		Rc::new(DummyKeyboardMapper)
	}

	fn on_keyboard_layout_change(&self, _: Box<dyn FnMut()>) {}

	fn run(&self, _on_finish_launching: Box<dyn FnOnce()>) {
		unimplemented!()
	}

	fn quit(&self) {}

	fn activate(&self, _ignoring_other_apps: bool) {
		//
	}

	fn hide(&self) {
		unimplemented!()
	}

	fn hide_other_apps(&self) {
		unimplemented!()
	}

	fn unhide_other_apps(&self) {
		unimplemented!()
	}

	fn displays(&self) -> Vec<std::rc::Rc<dyn crate::PlatformDisplay>> {
		vec![self.active_display.clone()]
	}

	fn primary_display(&self) -> Option<std::rc::Rc<dyn crate::PlatformDisplay>> {
		Some(self.active_display.clone())
	}

	fn active_window(&self) -> Option<crate::AnyWindowHandle> {
		self.active_window.borrow().as_ref().map(|window| window.0.lock().handle)
	}

	fn open_window(&self, handle: AnyWindowHandle, params: WindowParams) -> anyhow::Result<Box<dyn crate::PlatformWindow>> {
		let window = TestWindow::new(handle, params, self.weak.clone(), self.active_display.clone());
		Ok(Box::new(window))
	}

	fn window_appearance(&self) -> WindowAppearance {
		WindowAppearance::Light
	}

	fn on_quit(&self, _callback: Box<dyn FnMut()>) {}

	fn on_reopen(&self, _callback: Box<dyn FnMut()>) {
		unimplemented!()
	}

	fn set_menus(&self, _menus: Vec<crate::Menu>, _keymap: &Keymap) {}
	fn set_dock_menu(&self, _menu: Vec<crate::MenuItem>, _keymap: &Keymap) {}

	fn add_recent_document(&self, _paths: &Path) {}

	fn on_app_menu_action(&self, _callback: Box<dyn FnMut(&dyn crate::Action)>) {}

	fn on_will_open_app_menu(&self, _callback: Box<dyn FnMut()>) {}

	fn on_validate_app_menu_command(&self, _callback: Box<dyn FnMut(&dyn crate::Action) -> bool>) {}

	fn app_path(&self) -> Result<std::path::PathBuf> {
		unimplemented!()
	}

	fn path_for_auxiliary_executable(&self, _name: &str) -> Result<std::path::PathBuf> {
		unimplemented!()
	}

	fn set_cursor_style(&self, style: CursorStyle) {
		*self.active_cursor.lock() = style;
	}

	fn should_auto_hide_scrollbars(&self) -> bool {
		false
	}

	#[cfg(any(target_os = "linux", target_os = "freebsd"))]
	fn write_to_primary(&self, item: ClipboardItem) {
		*self.current_primary_item.lock() = Some(item);
	}

	fn write_to_clipboard(&self, item: ClipboardItem) {
		*self.current_clipboard_item.lock() = Some(item);
	}

	#[cfg(any(target_os = "linux", target_os = "freebsd"))]
	fn read_from_primary(&self) -> Option<ClipboardItem> {
		self.current_primary_item.lock().clone()
	}

	fn read_from_clipboard(&self) -> Option<ClipboardItem> {
		self.current_clipboard_item.lock().clone()
	}
}

#[cfg(target_os = "windows")]
impl Drop for TestPlatform {
	fn drop(&mut self) {
		unsafe {
			std::mem::ManuallyDrop::drop(&mut self.bitmap_factory);
			windows::Win32::System::Ole::OleUninitialize();
		}
	}
}

struct TestKeyboardLayout;

impl PlatformKeyboardLayout for TestKeyboardLayout {
	fn id(&self) -> &str {
		"zed.keyboard.example"
	}

	fn name(&self) -> &str {
		"zed.keyboard.example"
	}
}
