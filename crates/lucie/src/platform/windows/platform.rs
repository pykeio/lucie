use std::{
	cell::{Cell, RefCell},
	path::PathBuf,
	rc::{Rc, Weak},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering}
	}
};

use anyhow::{Context as _, Result, anyhow};
use lucie_common::ResultExt;
use lucie_style::CursorStyle;
use parking_lot::RwLock;
use smallvec::SmallVec;
use windows::{
	UI::ViewManagement::UISettings,
	Win32::{
		Foundation::*,
		Graphics::{Direct3D11::ID3D11Device, Gdi::*},
		System::{Com::*, LibraryLoader::*, Ole::*},
		UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*}
	},
	core::*
};

use crate::*;

pub(crate) struct WindowsPlatform {
	inner: Rc<WindowsPlatformInner>,
	raw_window_handles: Arc<RwLock<SmallVec<[SafeHwnd; 4]>>>,
	// The below members will never change throughout the entire lifecycle of the app.
	headless: bool,
	icon: HICON,
	background_executor: BackgroundExecutor,
	foreground_executor: ForegroundExecutor,
	drop_target_helper: Option<IDropTargetHelper>,
	/// Flag to instruct the `VSyncProvider` thread to invalidate the directx devices
	/// as resizing them has failed, causing us to have lost at least the render target.
	invalidate_devices: Arc<AtomicBool>,
	handle: HWND,
	disable_direct_composition: bool
}

struct WindowsPlatformInner {
	state: WindowsPlatformState,
	raw_window_handles: std::sync::Weak<RwLock<SmallVec<[SafeHwnd; 4]>>>,
	// The below members will never change throughout the entire lifecycle of the app.
	validation_number: usize,
	main_receiver: PriorityQueueReceiver<Runnable>,
	dispatcher: Arc<WindowsDispatcher>
}

pub(crate) struct WindowsPlatformState {
	callbacks: PlatformCallbacks,
	menus: RefCell<Vec<OwnedMenu>>,
	jump_list: RefCell<JumpList>,
	// NOTE: standard cursor handles don't need to close.
	pub(crate) current_cursor: Cell<Option<HCURSOR>>,
	directx_devices: RefCell<Option<DirectXDevices>>
}

#[derive(Default)]
struct PlatformCallbacks {
	quit: Cell<Option<Box<dyn FnMut()>>>,
	reopen: Cell<Option<Box<dyn FnMut()>>>,
	app_menu_action: Cell<Option<Box<dyn FnMut(&dyn Action)>>>,
	will_open_app_menu: Cell<Option<Box<dyn FnMut()>>>,
	validate_app_menu_command: Cell<Option<Box<dyn FnMut(&dyn Action) -> bool>>>,
	keyboard_layout_change: Cell<Option<Box<dyn FnMut()>>>
}

impl WindowsPlatformState {
	fn new(directx_devices: Option<DirectXDevices>) -> Self {
		let callbacks = PlatformCallbacks::default();
		let jump_list = JumpList::new();
		let current_cursor = load_cursor(CursorStyle::Arrow);

		Self {
			callbacks,
			jump_list: RefCell::new(jump_list),
			current_cursor: Cell::new(current_cursor),
			directx_devices: RefCell::new(directx_devices),
			menus: RefCell::new(Vec::new())
		}
	}
}

impl WindowsPlatform {
	pub(crate) fn new(headless: bool, liveness: std::sync::Weak<()>) -> Result<Self> {
		unsafe { OleInitialize(None).context("unable to initialize Windows OLE")? };

		let directx_devices = if !headless { Some(DirectXDevices::new().context("Creating DirectX devices")?) } else { None };
		let (main_sender, main_receiver) = PriorityQueueReceiver::new();
		let validation_number = fastrand::usize(..);
		let raw_window_handles = Arc::new(RwLock::new(SmallVec::new()));
		register_platform_window_class();
		let mut context = PlatformWindowCreateContext {
			inner: None,
			raw_window_handles: Arc::downgrade(&raw_window_handles),
			validation_number,
			main_sender: Some(main_sender),
			main_receiver: Some(main_receiver),
			directx_devices,
			dispatcher: None
		};
		let result = unsafe {
			CreateWindowExW(
				WINDOW_EX_STYLE(0),
				PLATFORM_WINDOW_CLASS_NAME,
				None,
				WINDOW_STYLE(0),
				0,
				0,
				0,
				0,
				Some(HWND_MESSAGE),
				None,
				None,
				Some(&raw const context as *const _)
			)
		};
		let inner = context.inner.take().context("CreateWindowExW did not run correctly")??;
		let dispatcher = context.dispatcher.take().context("CreateWindowExW did not run correctly")?;
		let handle = result?;

		let disable_direct_composition = std::env::var(DISABLE_DIRECT_COMPOSITION).is_ok_and(|value| value == "true" || value == "1");
		let background_executor = BackgroundExecutor::new(dispatcher.clone());
		let foreground_executor = ForegroundExecutor::new(dispatcher, liveness);

		let drop_target_helper: Option<IDropTargetHelper> = if !headless {
			Some(unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).context("Error creating drop target helper.")? })
		} else {
			None
		};
		let icon = if !headless { load_icon().unwrap_or_default() } else { HICON::default() };

		Ok(Self {
			inner,
			handle,
			raw_window_handles,
			headless,
			icon,
			background_executor,
			foreground_executor,
			disable_direct_composition,
			drop_target_helper,
			invalidate_devices: Arc::new(AtomicBool::new(false))
		})
	}

	pub fn window_from_hwnd(&self, hwnd: HWND) -> Option<Rc<WindowsWindowInner>> {
		self.raw_window_handles
			.read()
			.iter()
			.find(|entry| entry.as_raw() == hwnd)
			.and_then(|hwnd| window_from_hwnd(hwnd.as_raw()))
	}

	#[inline]
	fn post_message(&self, message: u32, wparam: WPARAM, lparam: LPARAM) {
		self.raw_window_handles.read().iter().for_each(|handle| unsafe {
			PostMessageW(Some(handle.as_raw()), message, wparam, lparam).log_err();
		});
	}

	fn generate_creation_info(&self) -> WindowCreationInfo {
		WindowCreationInfo {
			icon: self.icon,
			executor: self.foreground_executor.clone(),
			current_cursor: self.inner.state.current_cursor.get(),
			drop_target_helper: self.drop_target_helper.clone().unwrap(),
			validation_number: self.inner.validation_number,
			main_receiver: self.inner.main_receiver.clone(),
			platform_window_handle: self.handle,
			disable_direct_composition: self.disable_direct_composition,
			directx_devices: self.inner.state.directx_devices.borrow().clone().unwrap(),
			invalidate_devices: self.invalidate_devices.clone()
		}
	}

	fn set_dock_menus(&self, menus: Vec<MenuItem>) {
		let mut actions = Vec::new();
		menus.into_iter().for_each(|menu| {
			if let Some(dock_menu) = DockMenuItem::new(menu).log_err() {
				actions.push(dock_menu);
			}
		});
		self.inner.state.jump_list.borrow_mut().dock_menus = actions;
		let borrow = self.inner.state.jump_list.borrow();
		let dock_menus = borrow
			.dock_menus
			.iter()
			.map(|menu| (menu.name.clone(), menu.description.clone()))
			.collect::<Vec<_>>();
		let recent_workspaces = borrow.recent_workspaces.clone();
		self.background_executor
			.spawn(async move {
				update_jump_list(&recent_workspaces, &dock_menus).log_err();
			})
			.detach();
	}

	fn update_jump_list(&self, menus: Vec<MenuItem>, entries: Vec<SmallVec<[PathBuf; 2]>>) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
		let mut actions = Vec::new();
		menus.into_iter().for_each(|menu| {
			if let Some(dock_menu) = DockMenuItem::new(menu).log_err() {
				actions.push(dock_menu);
			}
		});
		let mut jump_list = self.inner.state.jump_list.borrow_mut();
		jump_list.dock_menus = actions;
		jump_list.recent_workspaces = entries.into();
		let dock_menus = jump_list
			.dock_menus
			.iter()
			.map(|menu| (menu.name.clone(), menu.description.clone()))
			.collect::<Vec<_>>();
		let recent_workspaces = jump_list.recent_workspaces.clone();
		self.background_executor
			.spawn(async move { update_jump_list(&recent_workspaces, &dock_menus).log_err().unwrap_or_default() })
	}

	fn begin_vsync_thread(&self) {
		let Some(directx_devices) = self.inner.state.directx_devices.borrow().clone() else {
			return;
		};
		let mut directx_device = directx_devices;
		let platform_window: SafeHwnd = self.handle.into();
		let validation_number = self.inner.validation_number;
		let all_windows = Arc::downgrade(&self.raw_window_handles);
		let invalidate_devices = self.invalidate_devices.clone();

		std::thread::Builder::new()
			.name("VSyncProvider".to_owned())
			.spawn(move || {
				let vsync_provider = VSyncProvider::new();
				loop {
					vsync_provider.wait_for_vsync();
					if check_device_lost(&directx_device.device) || invalidate_devices.fetch_and(false, Ordering::Acquire) {
						if let Err(err) = handle_gpu_device_lost(&mut directx_device, platform_window.as_raw(), validation_number, &all_windows) {
							panic!("Device lost: {err}");
						}
					}
					let Some(all_windows) = all_windows.upgrade() else {
						break;
					};
					for hwnd in all_windows.read().iter() {
						unsafe {
							let _ = RedrawWindow(Some(hwnd.as_raw()), None, None, RDW_INVALIDATE);
						}
					}
				}
			})
			.unwrap();
	}
}

fn translate_accelerator(msg: &MSG) -> Option<()> {
	if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
		return None;
	}

	let result = unsafe { SendMessageW(msg.hwnd, WM_LUCIE_KEYDOWN, Some(msg.wParam), Some(msg.lParam)) };
	(result.0 == 0).then_some(())
}

impl Platform for WindowsPlatform {
	fn background_executor(&self) -> BackgroundExecutor {
		self.background_executor.clone()
	}

	fn foreground_executor(&self) -> ForegroundExecutor {
		self.foreground_executor.clone()
	}

	fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
		Box::new(WindowsKeyboardLayout::new().log_err().unwrap_or(WindowsKeyboardLayout::unknown()))
	}

	fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
		Rc::new(WindowsKeyboardMapper::new())
	}

	fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
		self.inner.state.callbacks.keyboard_layout_change.set(Some(callback));
	}

	fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
		on_finish_launching();
		if !self.headless {
			self.begin_vsync_thread();
		}

		let mut msg = MSG::default();
		unsafe {
			while GetMessageW(&mut msg, None, 0, 0).as_bool() {
				if translate_accelerator(&msg).is_none() {
					_ = TranslateMessage(&msg);
					DispatchMessageW(&msg);
				}
			}
		}

		self.inner.with_callback(|callbacks| &callbacks.quit, |callback| callback());
	}

	fn quit(&self) {
		self.foreground_executor().spawn(async { unsafe { PostQuitMessage(0) } }).detach();
	}

	fn activate(&self, _ignoring_other_apps: bool) {}

	fn hide(&self) {}

	// todo(windows)
	fn hide_other_apps(&self) {
		unimplemented!()
	}

	// todo(windows)
	fn unhide_other_apps(&self) {
		unimplemented!()
	}

	fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
		WindowsDisplay::displays()
	}

	fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
		WindowsDisplay::primary_monitor().map(|display| Rc::new(display) as Rc<dyn PlatformDisplay>)
	}

	fn active_window(&self) -> Option<AnyWindowHandle> {
		let active_window_hwnd = unsafe { GetActiveWindow() };
		self.window_from_hwnd(active_window_hwnd).map(|inner| inner.handle)
	}

	fn open_window(&self, handle: AnyWindowHandle, options: WindowParams) -> Result<Box<dyn PlatformWindow>> {
		let window = WindowsWindow::new(handle, options, self.generate_creation_info())?;
		let handle = window.get_raw_handle();
		self.raw_window_handles.write().push(handle.into());

		Ok(Box::new(window))
	}

	fn window_appearance(&self) -> WindowAppearance {
		system_appearance().log_err().unwrap_or_default()
	}

	fn on_quit(&self, callback: Box<dyn FnMut()>) {
		self.inner.state.callbacks.quit.set(Some(callback));
	}

	fn on_reopen(&self, callback: Box<dyn FnMut()>) {
		self.inner.state.callbacks.reopen.set(Some(callback));
	}

	fn set_menus(&self, menus: Vec<Menu>, _keymap: &Keymap) {
		*self.inner.state.menus.borrow_mut() = menus.into_iter().map(|menu| menu.owned()).collect();
	}

	fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
		Some(self.inner.state.menus.borrow().clone())
	}

	fn set_dock_menu(&self, menus: Vec<MenuItem>, _keymap: &Keymap) {
		self.set_dock_menus(menus);
	}

	fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
		self.inner.state.callbacks.app_menu_action.set(Some(callback));
	}

	fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
		self.inner.state.callbacks.will_open_app_menu.set(Some(callback));
	}

	fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
		self.inner.state.callbacks.validate_app_menu_command.set(Some(callback));
	}

	fn app_path(&self) -> Result<PathBuf> {
		Ok(std::env::current_exe()?)
	}

	// todo(windows)
	fn path_for_auxiliary_executable(&self, _name: &str) -> Result<PathBuf> {
		anyhow::bail!("not yet implemented");
	}

	fn set_cursor_style(&self, style: CursorStyle) {
		let hcursor = load_cursor(style);
		if self.inner.state.current_cursor.get().map(|c| c.0) != hcursor.map(|c| c.0) {
			self.post_message(WM_LUCIE_CURSOR_STYLE_CHANGED, WPARAM(0), LPARAM(hcursor.map_or(0, |c| c.0 as isize)));
			self.inner.state.current_cursor.set(hcursor);
		}
	}

	fn should_auto_hide_scrollbars(&self) -> bool {
		should_auto_hide_scrollbars().log_err().unwrap_or(false)
	}

	fn write_to_clipboard(&self, item: ClipboardItem) {
		write_to_clipboard(item);
	}

	fn read_from_clipboard(&self) -> Option<ClipboardItem> {
		read_from_clipboard()
	}

	fn perform_dock_menu_action(&self, action: usize) {
		unsafe {
			PostMessageW(Some(self.handle), WM_LUCIE_DOCK_MENU_ACTION, WPARAM(self.inner.validation_number), LPARAM(action as isize)).log_err();
		}
	}

	fn update_jump_list(&self, menus: Vec<MenuItem>, entries: Vec<SmallVec<[PathBuf; 2]>>) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
		self.update_jump_list(menus, entries)
	}
}

impl WindowsPlatformInner {
	fn new(context: &mut PlatformWindowCreateContext) -> Result<Rc<Self>> {
		let state = WindowsPlatformState::new(context.directx_devices.take());
		Ok(Rc::new(Self {
			state,
			raw_window_handles: context.raw_window_handles.clone(),
			dispatcher: context.dispatcher.as_ref().context("missing dispatcher")?.clone(),
			validation_number: context.validation_number,
			main_receiver: context.main_receiver.take().context("missing main receiver")?
		}))
	}

	/// Calls `project` to project to the corresponding callback field, removes it from callbacks, calls `f` with the
	/// callback and then puts the callback back.
	fn with_callback<T>(&self, project: impl Fn(&PlatformCallbacks) -> &Cell<Option<T>>, f: impl FnOnce(&mut T)) {
		let callback = project(&self.state.callbacks).take();
		if let Some(mut callback) = callback {
			f(&mut callback);
			project(&self.state.callbacks).set(Some(callback));
		}
	}

	fn handle_msg(self: &Rc<Self>, handle: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
		let handled = match msg {
			WM_LUCIE_CLOSE_ONE_WINDOW
			| WM_LUCIE_TASK_DISPATCHED_ON_MAIN_THREAD
			| WM_LUCIE_DOCK_MENU_ACTION
			| WM_LUCIE_KEYBOARD_LAYOUT_CHANGED
			| WM_LUCIE_GPU_DEVICE_LOST => self.handle_lucie_events(msg, wparam, lparam),
			_ => None
		};
		if let Some(result) = handled {
			LRESULT(result)
		} else {
			unsafe { DefWindowProcW(handle, msg, wparam, lparam) }
		}
	}

	fn handle_lucie_events(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
		if wparam.0 != self.validation_number {
			tracing::error!("Wrong validation number while processing message: {message}");
			return None;
		}
		match message {
			WM_LUCIE_CLOSE_ONE_WINDOW => {
				self.close_one_window(HWND(lparam.0 as _));
				Some(0)
			}
			WM_LUCIE_TASK_DISPATCHED_ON_MAIN_THREAD => self.run_foreground_task(),
			WM_LUCIE_DOCK_MENU_ACTION => self.handle_dock_action_event(lparam.0 as _),
			WM_LUCIE_KEYBOARD_LAYOUT_CHANGED => self.handle_keyboard_layout_change(),
			WM_LUCIE_GPU_DEVICE_LOST => self.handle_device_lost(lparam),
			_ => unreachable!()
		}
	}

	fn close_one_window(&self, target_window: HWND) -> bool {
		let Some(all_windows) = self.raw_window_handles.upgrade() else {
			tracing::error!("Failed to upgrade raw window handles");
			return false;
		};
		let mut lock = all_windows.write();
		let index = lock.iter().position(|handle| handle.as_raw() == target_window).unwrap();
		lock.remove(index);

		lock.is_empty()
	}

	#[inline]
	fn run_foreground_task(&self) -> Option<isize> {
		const MAIN_TASK_TIMEOUT: u128 = 10;

		let start = std::time::Instant::now();
		'tasks: loop {
			'timeout_loop: loop {
				if start.elapsed().as_millis() >= MAIN_TASK_TIMEOUT {
					tracing::debug!("foreground task timeout reached");
					// we spent our budget on lucie tasks, we likely have a lot of work queued so drain system events first to stay
					// responsive then quit out of foreground work to allow us to process other lucie events first before returning back
					// to foreground task work if we don't we might not for example process window quit events
					let mut msg = MSG::default();
					let process_message = |msg: &_| {
						if translate_accelerator(msg).is_none() {
							_ = unsafe { TranslateMessage(msg) };
							unsafe { DispatchMessageW(msg) };
						}
					};
					let peek_msg = |msg: &mut _, msg_kind| unsafe { PeekMessageW(msg, None, 0, 0, PM_REMOVE | msg_kind).as_bool() };
					// We need to process a paint message here as otherwise we will re-enter `run_foreground_task` before painting if we
					// have work remaining. The reason for this is that windows prefers custom application message processing over
					// system messages.
					if peek_msg(&mut msg, PM_QS_PAINT) {
						process_message(&msg);
					}
					while peek_msg(&mut msg, PM_QS_INPUT) {
						process_message(&msg);
					}
					// Allow the main loop to process other lucie events before going back into `run_foreground_task`
					unsafe {
						if let Err(_) = PostMessageW(
							Some(self.dispatcher.platform_window_handle.as_raw()),
							WM_LUCIE_TASK_DISPATCHED_ON_MAIN_THREAD,
							WPARAM(self.validation_number),
							LPARAM(0)
						) {
							self.dispatcher.wake_posted.store(false, Ordering::Release);
						};
					}
					break 'tasks;
				}
				let mut main_receiver = self.main_receiver.clone();
				match main_receiver.try_pop() {
					Ok(Some(runnable)) => _ = runnable.run_and_profile(),
					_ => break 'timeout_loop
				}
			}

			// Someone could enqueue a Runnable here. The flag is still true, so they will not PostMessage.
			// We need to check for those Runnables after we clear the flag.
			self.dispatcher.wake_posted.store(false, Ordering::Release);
			let mut main_receiver = self.main_receiver.clone();
			match main_receiver.try_pop() {
				Ok(Some(runnable)) => {
					self.dispatcher.wake_posted.store(true, Ordering::Release);

					runnable.run_and_profile();
				}
				_ => break 'tasks
			}
		}

		Some(0)
	}

	fn handle_dock_action_event(&self, action_idx: usize) -> Option<isize> {
		let Some(action) = self
			.state
			.jump_list
			.borrow()
			.dock_menus
			.get(action_idx)
			.map(|dock_menu| dock_menu.action.boxed_clone())
		else {
			tracing::error!("Dock menu for index {action_idx} not found");
			return Some(1);
		};
		self.with_callback(|callbacks| &callbacks.app_menu_action, |callback| callback(&*action));
		Some(0)
	}

	fn handle_keyboard_layout_change(&self) -> Option<isize> {
		self.with_callback(|callbacks| &callbacks.keyboard_layout_change, |callback| callback());
		Some(0)
	}

	fn handle_device_lost(&self, lparam: LPARAM) -> Option<isize> {
		let directx_devices = lparam.0 as *const DirectXDevices;
		let directx_devices = unsafe { &*directx_devices };
		self.state.directx_devices.borrow_mut().take();
		*self.state.directx_devices.borrow_mut() = Some(directx_devices.clone());

		Some(0)
	}
}

impl Drop for WindowsPlatform {
	fn drop(&mut self) {
		unsafe {
			DestroyWindow(self.handle).context("Destroying platform window").log_err();
			OleUninitialize();
		}
	}
}

pub(crate) struct WindowCreationInfo {
	pub(crate) icon: HICON,
	pub(crate) executor: ForegroundExecutor,
	pub(crate) current_cursor: Option<HCURSOR>,
	pub(crate) drop_target_helper: IDropTargetHelper,
	pub(crate) validation_number: usize,
	pub(crate) main_receiver: PriorityQueueReceiver<Runnable>,
	pub(crate) platform_window_handle: HWND,
	pub(crate) disable_direct_composition: bool,
	pub(crate) directx_devices: DirectXDevices,
	/// Flag to instruct the `VSyncProvider` thread to invalidate the directx devices
	/// as resizing them has failed, causing us to have lost at least the render target.
	pub(crate) invalidate_devices: Arc<AtomicBool>
}

struct PlatformWindowCreateContext {
	inner: Option<Result<Rc<WindowsPlatformInner>>>,
	raw_window_handles: std::sync::Weak<RwLock<SmallVec<[SafeHwnd; 4]>>>,
	validation_number: usize,
	main_sender: Option<PriorityQueueSender<Runnable>>,
	main_receiver: Option<PriorityQueueReceiver<Runnable>>,
	directx_devices: Option<DirectXDevices>,
	dispatcher: Option<Arc<WindowsDispatcher>>
}

fn load_icon() -> Result<HICON> {
	let module = unsafe { GetModuleHandleW(None).context("unable to get module handle")? };
	let handle = unsafe {
		LoadImageW(Some(module.into()), windows::core::PCWSTR(1 as _), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE | LR_SHARED).context("unable to load icon file")?
	};
	Ok(HICON(handle.0))
}

#[inline]
fn should_auto_hide_scrollbars() -> Result<bool> {
	let ui_settings = UISettings::new()?;
	Ok(ui_settings.AutoHideScrollBars()?)
}

fn check_device_lost(device: &ID3D11Device) -> bool {
	let device_state = unsafe { device.GetDeviceRemovedReason() };
	match device_state {
		Ok(_) => false,
		Err(err) => {
			tracing::error!("DirectX device lost detected: {:?}", err);
			true
		}
	}
}

fn handle_gpu_device_lost(
	directx_devices: &mut DirectXDevices,
	platform_window: HWND,
	validation_number: usize,
	all_windows: &std::sync::Weak<RwLock<SmallVec<[SafeHwnd; 4]>>>
) -> Result<()> {
	// Here we wait a bit to ensure the system has time to recover from the device lost state.
	// If we don't wait, the final drawing result will be blank.
	std::thread::sleep(std::time::Duration::from_millis(350));

	*directx_devices = try_to_recover_from_device_lost(|| DirectXDevices::new().context("Failed to recreate new DirectX devices after device lost"))?;
	tracing::info!("DirectX devices successfully recreated.");

	let lparam = LPARAM(directx_devices as *const _ as _);
	unsafe {
		SendMessageW(platform_window, WM_LUCIE_GPU_DEVICE_LOST, Some(WPARAM(validation_number)), Some(lparam));
	}

	if let Some(all_windows) = all_windows.upgrade() {
		for window in all_windows.read().iter() {
			unsafe {
				SendMessageW(window.as_raw(), WM_LUCIE_GPU_DEVICE_LOST, Some(WPARAM(validation_number)), Some(lparam));
			}
		}
		std::thread::sleep(std::time::Duration::from_millis(200));
		for window in all_windows.read().iter() {
			unsafe {
				SendMessageW(window.as_raw(), WM_LUCIE_FORCE_UPDATE_WINDOW, Some(WPARAM(validation_number)), None);
			}
		}
	}
	Ok(())
}

const PLATFORM_WINDOW_CLASS_NAME: PCWSTR = w!("Zed::PlatformWindow");

fn register_platform_window_class() {
	let wc = WNDCLASSW {
		lpfnWndProc: Some(window_procedure),
		lpszClassName: PCWSTR(PLATFORM_WINDOW_CLASS_NAME.as_ptr()),
		..Default::default()
	};
	unsafe { RegisterClassW(&wc) };
}

unsafe extern "system" fn window_procedure(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	if msg == WM_NCCREATE {
		let params = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
		let creation_context = params.lpCreateParams as *mut PlatformWindowCreateContext;
		let creation_context = unsafe { &mut *creation_context };

		let Some(main_sender) = creation_context.main_sender.take() else {
			creation_context.inner = Some(Err(anyhow!("missing main sender")));
			return LRESULT(0);
		};
		creation_context.dispatcher = Some(Arc::new(WindowsDispatcher::new(main_sender, hwnd, creation_context.validation_number)));

		return match WindowsPlatformInner::new(creation_context) {
			Ok(inner) => {
				let weak = Box::new(Rc::downgrade(&inner));
				unsafe { set_window_long(hwnd, GWLP_USERDATA, Box::into_raw(weak) as isize) };
				creation_context.inner = Some(Ok(inner));
				unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
			}
			Err(error) => {
				creation_context.inner = Some(Err(error));
				LRESULT(0)
			}
		};
	}

	let ptr = unsafe { get_window_long(hwnd, GWLP_USERDATA) } as *mut Weak<WindowsPlatformInner>;
	if ptr.is_null() {
		return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
	}
	let inner = unsafe { &*ptr };
	let result = if let Some(inner) = inner.upgrade() {
		inner.handle_msg(hwnd, msg, wparam, lparam)
	} else {
		unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
	};

	if msg == WM_NCDESTROY {
		unsafe { set_window_long(hwnd, GWLP_USERDATA, 0) };
		unsafe { drop(Box::from_raw(ptr)) };
	}

	result
}

#[cfg(test)]
mod tests {
	use crate::{ClipboardItem, read_from_clipboard, write_to_clipboard};

	#[test]
	fn test_clipboard() {
		let item = ClipboardItem::new_string("你好，我是张小白".to_string());
		write_to_clipboard(item.clone());
		assert_eq!(read_from_clipboard(), Some(item));

		let item = ClipboardItem::new_string("12345".to_string());
		write_to_clipboard(item.clone());
		assert_eq!(read_from_clipboard(), Some(item));

		let item = ClipboardItem::new_string_with_metadata("abcdef".to_string(), "[3,4]".to_string());
		write_to_clipboard(item.clone());
		assert_eq!(read_from_clipboard(), Some(item));
	}
}
