use std::{
	cell::Cell,
	convert::TryInto,
	ffi::{CStr, OsStr, c_void},
	os::{raw::c_char, unix::ffi::OsStrExt},
	path::{Path, PathBuf},
	ptr::{self, null_mut},
	rc::Rc,
	slice, str,
	sync::{Arc, OnceLock}
};

use anyhow::{Context as _, anyhow};
use block::ConcreteBlock;
use cocoa::{
	appkit::{
		NSApplication, NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular, NSEventModifierFlags, NSMenu, NSMenuItem, NSModalResponse,
		NSOpenPanel, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeRTF, NSPasteboardTypeRTFD, NSPasteboardTypeString, NSPasteboardTypeTIFF, NSSavePanel,
		NSVisualEffectState, NSVisualEffectView, NSWindow
	},
	base::{BOOL, NO, YES, id, nil, selector},
	foundation::{NSArray, NSAutoreleasePool, NSBundle, NSData, NSInteger, NSProcessInfo, NSRange, NSString, NSUInteger, NSURL}
};
use core_foundation::{
	base::{CFRelease, CFType, CFTypeRef, OSStatus, TCFType},
	boolean::CFBoolean,
	data::CFData,
	dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionary},
	runloop::CFRunLoopRun,
	string::{CFString, CFStringRef}
};
use ctor::ctor;
use futures_channel::oneshot;
use itertools::Itertools;
use lucie_common::ResultExt as _;
use objc::{
	class,
	declare::ClassDecl,
	msg_send,
	runtime::{Class, Object, Sel},
	sel, sel_impl
};
use parking_lot::Mutex;
use semver::Version;

use super::{
	BoolExt, MacKeyboardLayout, MacKeyboardMapper,
	attributed_string::{NSAttributedString, NSMutableAttributedString},
	events::key_to_native,
	renderer
};
use crate::{
	Action, AnyWindowHandle, BackgroundExecutor, ClipboardEntry, ClipboardItem, ClipboardString, CursorStyle, ForegroundExecutor, Image, ImageFormat,
	KeyContext, Keymap, MacDispatcher, MacDisplay, MacWindow, Menu, MenuItem, OsMenu, OwnedMenu, PathPromptOptions, Platform, PlatformDisplay,
	PlatformKeyboardLayout, PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow, Result, SystemMenuType, Task, WindowAppearance, WindowParams, hash
};

#[allow(non_upper_case_globals)]
const NSUTF8StringEncoding: NSUInteger = 4;

const MAC_PLATFORM_IVAR: &str = "platform";
static mut APP_CLASS: *const Class = ptr::null();
static mut APP_DELEGATE_CLASS: *const Class = ptr::null();

#[ctor]
unsafe fn build_classes() {
	unsafe {
		APP_CLASS = {
			let mut decl = ClassDecl::new("LucieApplication", class!(NSApplication)).unwrap();
			decl.add_ivar::<*mut c_void>(MAC_PLATFORM_IVAR);
			decl.register()
		}
	};
	unsafe {
		APP_DELEGATE_CLASS = unsafe {
			let mut decl = ClassDecl::new("LucieApplicationDelegate", class!(NSResponder)).unwrap();
			decl.add_ivar::<*mut c_void>(MAC_PLATFORM_IVAR);
			decl.add_method(sel!(applicationWillFinishLaunching:), will_finish_launching as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(applicationDidFinishLaunching:), did_finish_launching as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(applicationShouldHandleReopen:hasVisibleWindows:), should_handle_reopen as extern "C" fn(&mut Object, Sel, id, bool));
			decl.add_method(sel!(applicationWillTerminate:), will_terminate as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(handleLucieMenuItem:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			// Add menu item handlers so that OS save panels have the correct key commands
			decl.add_method(sel!(cut:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(copy:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(paste:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(selectAll:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(undo:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(redo:), handle_menu_item as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(validateMenuItem:), validate_menu_item as extern "C" fn(&mut Object, Sel, id) -> bool);
			decl.add_method(sel!(menuWillOpen:), menu_will_open as extern "C" fn(&mut Object, Sel, id));
			decl.add_method(sel!(applicationDockMenu:), handle_dock_menu as extern "C" fn(&mut Object, Sel, id) -> id);

			decl.add_method(sel!(onKeyboardLayoutChange:), on_keyboard_layout_change as extern "C" fn(&mut Object, Sel, id));

			decl.register()
		}
	}
}

pub(crate) struct MacPlatform(Mutex<MacPlatformState>);

pub(crate) struct MacPlatformState {
	background_executor: BackgroundExecutor,
	foreground_executor: ForegroundExecutor,
	text_system: Arc<dyn PlatformTextSystem>,
	renderer_context: renderer::Context,
	headless: bool,
	pasteboard: id,
	text_hash_pasteboard_type: id,
	metadata_pasteboard_type: id,
	reopen: Option<Box<dyn FnMut()>>,
	on_keyboard_layout_change: Option<Box<dyn FnMut()>>,
	quit: Option<Box<dyn FnMut()>>,
	menu_command: Option<Box<dyn FnMut(&dyn Action)>>,
	validate_menu_command: Option<Box<dyn FnMut(&dyn Action) -> bool>>,
	will_open_menu: Option<Box<dyn FnMut()>>,
	menu_actions: Vec<Box<dyn Action>>,
	finish_launching: Option<Box<dyn FnOnce()>>,
	dock_menu: Option<id>,
	menus: Option<Vec<OwnedMenu>>,
	keyboard_mapper: Rc<MacKeyboardMapper>
}

impl Default for MacPlatform {
	fn default() -> Self {
		Self::new(false)
	}
}

impl MacPlatform {
	pub(crate) fn new(headless: bool) -> Self {
		let dispatcher = Arc::new(MacDispatcher);

		#[cfg(feature = "font-kit")]
		let text_system = Arc::new(crate::MacTextSystem::new());

		#[cfg(not(feature = "font-kit"))]
		let text_system = Arc::new(crate::NoopTextSystem::new());

		let keyboard_layout = MacKeyboardLayout::new();
		let keyboard_mapper = Rc::new(MacKeyboardMapper::new(keyboard_layout.id()));

		Self(Mutex::new(MacPlatformState {
			headless,
			text_system,
			background_executor: BackgroundExecutor::new(dispatcher.clone()),
			foreground_executor: ForegroundExecutor::new(dispatcher),
			renderer_context: renderer::Context::default(),
			pasteboard: unsafe { NSPasteboard::generalPasteboard(nil) },
			text_hash_pasteboard_type: unsafe { ns_string("zed-text-hash") },
			metadata_pasteboard_type: unsafe { ns_string("zed-metadata") },
			reopen: None,
			quit: None,
			menu_command: None,
			validate_menu_command: None,
			will_open_menu: None,
			menu_actions: Default::default(),
			finish_launching: None,
			dock_menu: None,
			on_keyboard_layout_change: None,
			menus: None,
			keyboard_mapper
		}))
	}

	unsafe fn read_from_pasteboard(&self, pasteboard: *mut Object, kind: id) -> Option<&[u8]> {
		unsafe {
			let data = pasteboard.dataForType(kind);
			if data == nil {
				None
			} else {
				Some(slice::from_raw_parts(data.bytes() as *mut u8, data.length() as usize))
			}
		}
	}

	unsafe fn create_menu_bar(&self, menus: &Vec<Menu>, delegate: id, actions: &mut Vec<Box<dyn Action>>, keymap: &Keymap) -> id {
		unsafe {
			let application_menu = NSMenu::new(nil).autorelease();
			application_menu.setDelegate_(delegate);

			for menu_config in menus {
				let menu = NSMenu::new(nil).autorelease();
				let menu_title = ns_string(&menu_config.name);
				menu.setTitle_(menu_title);
				menu.setDelegate_(delegate);

				for item_config in &menu_config.items {
					menu.addItem_(Self::create_menu_item(item_config, delegate, actions, keymap));
				}

				let menu_item = NSMenuItem::new(nil).autorelease();
				menu_item.setTitle_(menu_title);
				menu_item.setSubmenu_(menu);
				application_menu.addItem_(menu_item);

				if menu_config.name == "Window" {
					let app: id = msg_send![APP_CLASS, sharedApplication];
					app.setWindowsMenu_(menu);
				}
			}

			application_menu
		}
	}

	unsafe fn create_dock_menu(&self, menu_items: Vec<MenuItem>, delegate: id, actions: &mut Vec<Box<dyn Action>>, keymap: &Keymap) -> id {
		unsafe {
			let dock_menu = NSMenu::new(nil);
			dock_menu.setDelegate_(delegate);
			for item_config in menu_items {
				dock_menu.addItem_(Self::create_menu_item(&item_config, delegate, actions, keymap));
			}

			dock_menu
		}
	}

	unsafe fn create_menu_item(item: &MenuItem, delegate: id, actions: &mut Vec<Box<dyn Action>>, keymap: &Keymap) -> id {
		static DEFAULT_CONTEXT: OnceLock<Vec<KeyContext>> = OnceLock::new();

		unsafe {
			match item {
				MenuItem::Separator => NSMenuItem::separatorItem(nil),
				MenuItem::Action { name, action, os_action, checked } => {
					// Note that this is intentionally using earlier bindings, whereas typically
					// later ones take display precedence. See the discussion on
					// https://github.com/zed-industries/zed/issues/23621
					let keystrokes = keymap
						.bindings_for_action(action.as_ref())
						.find_or_first(|binding| {
							binding.predicate().is_none_or(|predicate| {
								predicate.eval(DEFAULT_CONTEXT.get_or_init(|| {
									let mut workspace_context = KeyContext::new_with_defaults();
									workspace_context.add("Workspace");
									let mut pane_context = KeyContext::new_with_defaults();
									pane_context.add("Pane");
									let mut editor_context = KeyContext::new_with_defaults();
									editor_context.add("Editor");

									pane_context.extend(&editor_context);
									workspace_context.extend(&pane_context);
									vec![workspace_context]
								}))
							})
						})
						.map(|binding| binding.keystrokes());

					let selector = match os_action {
						Some(crate::OsAction::Cut) => selector("cut:"),
						Some(crate::OsAction::Copy) => selector("copy:"),
						Some(crate::OsAction::Paste) => selector("paste:"),
						Some(crate::OsAction::SelectAll) => selector("selectAll:"),
						// "undo:" and "redo:" are always disabled in our case, as
						// we don't have a NSTextView/NSTextField to enable them on.
						Some(crate::OsAction::Undo) => selector("handleLucieMenuItem:"),
						Some(crate::OsAction::Redo) => selector("handleLucieMenuItem:"),
						None => selector("handleLucieMenuItem:")
					};

					let item;
					if let Some(keystrokes) = keystrokes {
						if keystrokes.len() == 1 {
							let keystroke = &keystrokes[0];
							let mut mask = NSEventModifierFlags::empty();
							for (modifier, flag) in &[
								(keystroke.modifiers().platform, NSEventModifierFlags::NSCommandKeyMask),
								(keystroke.modifiers().control, NSEventModifierFlags::NSControlKeyMask),
								(keystroke.modifiers().alt, NSEventModifierFlags::NSAlternateKeyMask),
								(keystroke.modifiers().shift, NSEventModifierFlags::NSShiftKeyMask)
							] {
								if *modifier {
									mask |= *flag;
								}
							}

							item = NSMenuItem::alloc(nil)
								.initWithTitle_action_keyEquivalent_(ns_string(name), selector, ns_string(key_to_native(keystroke.key()).as_ref()))
								.autorelease();
							if Self::os_version() >= Version::new(12, 0, 0) {
								let _: () = msg_send![item, setAllowsAutomaticKeyEquivalentLocalization: NO];
							}
							item.setKeyEquivalentModifierMask_(mask);
						} else {
							item = NSMenuItem::alloc(nil)
								.initWithTitle_action_keyEquivalent_(ns_string(name), selector, ns_string(""))
								.autorelease();
						}
					} else {
						item = NSMenuItem::alloc(nil)
							.initWithTitle_action_keyEquivalent_(ns_string(name), selector, ns_string(""))
							.autorelease();
					}

					if *checked {
						item.setState_(NSVisualEffectState::Active);
					}

					let tag = actions.len() as NSInteger;
					let _: () = msg_send![item, setTag: tag];
					actions.push(action.boxed_clone());
					item
				}
				MenuItem::Submenu(Menu { name, items }) => {
					let item = NSMenuItem::new(nil).autorelease();
					let submenu = NSMenu::new(nil).autorelease();
					submenu.setDelegate_(delegate);
					for item in items {
						submenu.addItem_(Self::create_menu_item(item, delegate, actions, keymap));
					}
					item.setSubmenu_(submenu);
					item.setTitle_(ns_string(name));
					item
				}
				MenuItem::SystemMenu(OsMenu { name, menu_type }) => {
					let item = NSMenuItem::new(nil).autorelease();
					let submenu = NSMenu::new(nil).autorelease();
					submenu.setDelegate_(delegate);
					item.setSubmenu_(submenu);
					item.setTitle_(ns_string(name));

					match menu_type {
						SystemMenuType::Services => {
							let app: id = msg_send![APP_CLASS, sharedApplication];
							app.setServicesMenu_(item);
						}
					}

					item
				}
			}
		}
	}

	fn os_version() -> Version {
		let version = unsafe {
			let process_info = NSProcessInfo::processInfo(nil);
			process_info.operatingSystemVersion()
		};
		Version::new(version.majorVersion, version.minorVersion, version.patchVersion)
	}
}

impl Platform for MacPlatform {
	fn background_executor(&self) -> BackgroundExecutor {
		self.0.lock().background_executor.clone()
	}

	fn foreground_executor(&self) -> crate::ForegroundExecutor {
		self.0.lock().foreground_executor.clone()
	}

	fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
		self.0.lock().text_system.clone()
	}

	fn run(&self, on_finish_launching: Box<dyn FnOnce()>) {
		let mut state = self.0.lock();
		if state.headless {
			drop(state);
			on_finish_launching();
			unsafe { CFRunLoopRun() };
		} else {
			state.finish_launching = Some(on_finish_launching);
			drop(state);
		}

		unsafe {
			let app: id = msg_send![APP_CLASS, sharedApplication];
			let app_delegate: id = msg_send![APP_DELEGATE_CLASS, new];
			app.setDelegate_(app_delegate);

			let self_ptr = self as *const Self as *const c_void;
			(*app).set_ivar(MAC_PLATFORM_IVAR, self_ptr);
			(*app_delegate).set_ivar(MAC_PLATFORM_IVAR, self_ptr);

			let pool = NSAutoreleasePool::new(nil);
			app.run();
			pool.drain();

			(*app).set_ivar(MAC_PLATFORM_IVAR, null_mut::<c_void>());
			(*NSWindow::delegate(app)).set_ivar(MAC_PLATFORM_IVAR, null_mut::<c_void>());
		}
	}

	fn quit(&self) {
		// Quitting the app causes us to close windows, which invokes `Window::on_close` callbacks
		// synchronously before this method terminates. If we call `Platform::quit` while holding a
		// borrow of the app state (which most of the time we will do), we will end up
		// double-borrowing the app state in the `on_close` callbacks for our open windows. To solve
		// this, we make quitting the application asynchronous so that we aren't holding borrows to
		// the app state on the stack when we actually terminate the app.

		use super::dispatcher::{dispatch_get_main_queue, dispatch_sys::dispatch_async_f};

		unsafe {
			dispatch_async_f(dispatch_get_main_queue(), ptr::null_mut(), Some(quit));
		}

		unsafe extern "C" fn quit(_: *mut c_void) {
			unsafe {
				let app = NSApplication::sharedApplication(nil);
				let _: () = msg_send![app, terminate: nil];
			}
		}
	}

	fn activate(&self, ignoring_other_apps: bool) {
		unsafe {
			let app = NSApplication::sharedApplication(nil);
			app.activateIgnoringOtherApps_(ignoring_other_apps.to_objc());
		}
	}

	fn hide(&self) {
		unsafe {
			let app = NSApplication::sharedApplication(nil);
			let _: () = msg_send![app, hide: nil];
		}
	}

	fn hide_other_apps(&self) {
		unsafe {
			let app = NSApplication::sharedApplication(nil);
			let _: () = msg_send![app, hideOtherApplications: nil];
		}
	}

	fn unhide_other_apps(&self) {
		unsafe {
			let app = NSApplication::sharedApplication(nil);
			let _: () = msg_send![app, unhideAllApplications: nil];
		}
	}

	fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
		Some(Rc::new(MacDisplay::primary()))
	}

	fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
		MacDisplay::all().map(|screen| Rc::new(screen) as Rc<_>).collect()
	}

	fn active_window(&self) -> Option<AnyWindowHandle> {
		MacWindow::active_window()
	}

	// Returns the windows ordered front-to-back, meaning that the active
	// window is the first one in the returned vec.
	fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
		Some(MacWindow::ordered_windows())
	}

	fn open_window(&self, handle: AnyWindowHandle, options: WindowParams) -> Result<Box<dyn PlatformWindow>> {
		let renderer_context = self.0.lock().renderer_context.clone();
		Ok(Box::new(MacWindow::open(handle, options, self.foreground_executor(), renderer_context)))
	}

	fn window_appearance(&self) -> WindowAppearance {
		unsafe {
			let app = NSApplication::sharedApplication(nil);
			let appearance: id = msg_send![app, effectiveAppearance];
			WindowAppearance::from_native(appearance)
		}
	}

	fn on_quit(&self, callback: Box<dyn FnMut()>) {
		self.0.lock().quit = Some(callback);
	}

	fn on_reopen(&self, callback: Box<dyn FnMut()>) {
		self.0.lock().reopen = Some(callback);
	}

	fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
		self.0.lock().on_keyboard_layout_change = Some(callback);
	}

	fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
		self.0.lock().menu_command = Some(callback);
	}

	fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
		self.0.lock().will_open_menu = Some(callback);
	}

	fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
		self.0.lock().validate_menu_command = Some(callback);
	}

	fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
		Box::new(MacKeyboardLayout::new())
	}

	fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
		self.0.lock().keyboard_mapper.clone()
	}

	fn app_path(&self) -> Result<PathBuf> {
		unsafe {
			let bundle: id = NSBundle::mainBundle();
			anyhow::ensure!(!bundle.is_null(), "app is not running inside a bundle");
			Ok(path_from_objc(msg_send![bundle, bundlePath]))
		}
	}

	fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap) {
		unsafe {
			let app: id = msg_send![APP_CLASS, sharedApplication];
			let mut state = self.0.lock();
			let actions = &mut state.menu_actions;
			let menu = self.create_menu_bar(&menus, NSWindow::delegate(app), actions, keymap);
			drop(state);
			app.setMainMenu_(menu);
		}
		self.0.lock().menus = Some(menus.into_iter().map(|menu| menu.owned()).collect());
	}

	fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
		self.0.lock().menus.clone()
	}

	fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap) {
		unsafe {
			let app: id = msg_send![APP_CLASS, sharedApplication];
			let mut state = self.0.lock();
			let actions = &mut state.menu_actions;
			let new = self.create_dock_menu(menu, NSWindow::delegate(app), actions, keymap);
			if let Some(old) = state.dock_menu.replace(new) {
				CFRelease(old as _)
			}
		}
	}

	fn add_recent_document(&self, path: &Path) {
		if let Some(path_str) = path.to_str() {
			unsafe {
				let document_controller: id = msg_send![class!(NSDocumentController), sharedDocumentController];
				let url: id = NSURL::fileURLWithPath_(nil, ns_string(path_str));
				let _: () = msg_send![document_controller, noteNewRecentDocumentURL:url];
			}
		}
	}

	fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
		unsafe {
			let bundle: id = NSBundle::mainBundle();
			anyhow::ensure!(!bundle.is_null(), "app is not running inside a bundle");
			let name = ns_string(name);
			let url: id = msg_send![bundle, URLForAuxiliaryExecutable: name];
			anyhow::ensure!(!url.is_null(), "resource not found");
			ns_url_to_path(url)
		}
	}

	/// Match cursor style to one of the styles available
	/// in macOS's [NSCursor](https://developer.apple.com/documentation/appkit/nscursor).
	fn set_cursor_style(&self, style: CursorStyle) {
		unsafe {
			if style == CursorStyle::None {
				let _: () = msg_send![class!(NSCursor), setHiddenUntilMouseMoves:YES];
				return;
			}

			let new_cursor: id = match style {
				CursorStyle::Arrow => msg_send![class!(NSCursor), arrowCursor],
				CursorStyle::IBeam => msg_send![class!(NSCursor), IBeamCursor],
				CursorStyle::Crosshair => msg_send![class!(NSCursor), crosshairCursor],
				CursorStyle::ClosedHand => msg_send![class!(NSCursor), closedHandCursor],
				CursorStyle::OpenHand => msg_send![class!(NSCursor), openHandCursor],
				CursorStyle::PointingHand => msg_send![class!(NSCursor), pointingHandCursor],
				CursorStyle::ResizeLeftRight => msg_send![class!(NSCursor), resizeLeftRightCursor],
				CursorStyle::ResizeUpDown => msg_send![class!(NSCursor), resizeUpDownCursor],
				CursorStyle::ResizeLeft => msg_send![class!(NSCursor), resizeLeftCursor],
				CursorStyle::ResizeRight => msg_send![class!(NSCursor), resizeRightCursor],
				CursorStyle::ResizeColumn => msg_send![class!(NSCursor), resizeLeftRightCursor],
				CursorStyle::ResizeRow => msg_send![class!(NSCursor), resizeUpDownCursor],
				CursorStyle::ResizeUp => msg_send![class!(NSCursor), resizeUpCursor],
				CursorStyle::ResizeDown => msg_send![class!(NSCursor), resizeDownCursor],

				// Undocumented, private class methods:
				// https://stackoverflow.com/questions/27242353/cocoa-predefined-resize-mouse-cursor
				CursorStyle::ResizeUpLeftDownRight => {
					msg_send![class!(NSCursor), _windowResizeNorthWestSouthEastCursor]
				}
				CursorStyle::ResizeUpRightDownLeft => {
					msg_send![class!(NSCursor), _windowResizeNorthEastSouthWestCursor]
				}

				CursorStyle::IBeamCursorForVerticalLayout => {
					msg_send![class!(NSCursor), IBeamCursorForVerticalLayout]
				}
				CursorStyle::OperationNotAllowed => {
					msg_send![class!(NSCursor), operationNotAllowedCursor]
				}
				CursorStyle::DragLink => msg_send![class!(NSCursor), dragLinkCursor],
				CursorStyle::DragCopy => msg_send![class!(NSCursor), dragCopyCursor],
				CursorStyle::ContextualMenu => msg_send![class!(NSCursor), contextualMenuCursor],
				CursorStyle::None => unreachable!()
			};

			let old_cursor: id = msg_send![class!(NSCursor), currentCursor];
			if new_cursor != old_cursor {
				let _: () = msg_send![new_cursor, set];
			}
		}
	}

	fn should_auto_hide_scrollbars(&self) -> bool {
		#[allow(non_upper_case_globals)]
		const NSScrollerStyleOverlay: NSInteger = 1;

		unsafe {
			let style: NSInteger = msg_send![class!(NSScroller), preferredScrollerStyle];
			style == NSScrollerStyleOverlay
		}
	}

	fn write_to_clipboard(&self, item: ClipboardItem) {
		use crate::ClipboardEntry;

		unsafe {
			// We only want to use NSAttributedString if there are multiple entries to write.
			if item.entries.len() <= 1 {
				match item.entries.first() {
					Some(entry) => match entry {
						ClipboardEntry::String(string) => {
							self.write_plaintext_to_clipboard(string);
						}
						ClipboardEntry::Image(image) => {
							self.write_image_to_clipboard(image);
						}
						ClipboardEntry::ExternalPaths(_) => {}
					},
					None => {
						// Writing an empty list of entries just clears the clipboard.
						let state = self.0.lock();
						state.pasteboard.clearContents();
					}
				}
			} else {
				let mut any_images = false;
				let attributed_string = {
					let mut buf = NSMutableAttributedString::alloc(nil)
                        // TODO can we skip this? Or at least part of it?
                        .init_attributed_string(NSString::alloc(nil).init_str(""));

					for entry in item.entries {
						if let ClipboardEntry::String(ClipboardString { text, metadata: _ }) = entry {
							let to_append = NSAttributedString::alloc(nil).init_attributed_string(NSString::alloc(nil).init_str(&text));

							buf.appendAttributedString_(to_append);
						}
					}

					buf
				};

				let state = self.0.lock();
				state.pasteboard.clearContents();

				// Only set rich text clipboard types if we actually have 1+ images to include.
				if any_images {
					let rtfd_data = attributed_string.RTFDFromRange_documentAttributes_(NSRange::new(0, msg_send![attributed_string, length]), nil);
					if rtfd_data != nil {
						state.pasteboard.setData_forType(rtfd_data, NSPasteboardTypeRTFD);
					}

					let rtf_data = attributed_string.RTFFromRange_documentAttributes_(NSRange::new(0, attributed_string.length()), nil);
					if rtf_data != nil {
						state.pasteboard.setData_forType(rtf_data, NSPasteboardTypeRTF);
					}
				}

				let plain_text = attributed_string.string();
				state.pasteboard.setString_forType(plain_text, NSPasteboardTypeString);
			}
		}
	}

	fn read_from_clipboard(&self) -> Option<ClipboardItem> {
		let state = self.0.lock();
		let pasteboard = state.pasteboard;

		// First, see if it's a string.
		unsafe {
			let types: id = pasteboard.types();
			let string_type: id = ns_string("public.utf8-plain-text");

			if msg_send![types, containsObject: string_type] {
				let data = pasteboard.dataForType(string_type);
				if data == nil {
					return None;
				} else if data.bytes().is_null() {
					// https://developer.apple.com/documentation/foundation/nsdata/1410616-bytes?language=objc
					// "If the length of the NSData object is 0, this property returns nil."
					return Some(self.read_string_from_clipboard(&state, &[]));
				} else {
					let bytes = slice::from_raw_parts(data.bytes() as *mut u8, data.length() as usize);

					return Some(self.read_string_from_clipboard(&state, bytes));
				}
			}

			// If it wasn't a string, try the various supported image types.
			for format in ImageFormat::iter() {
				if let Some(item) = try_clipboard_image(pasteboard, format) {
					return Some(item);
				}
			}
		}

		// If it wasn't a string or a supported image type, give up.
		None
	}
}

impl MacPlatform {
	unsafe fn read_string_from_clipboard(&self, state: &MacPlatformState, text_bytes: &[u8]) -> ClipboardItem {
		unsafe {
			let text = String::from_utf8_lossy(text_bytes).to_string();
			let metadata = self
				.read_from_pasteboard(state.pasteboard, state.text_hash_pasteboard_type)
				.and_then(|hash_bytes| {
					let hash_bytes = hash_bytes.try_into().ok()?;
					let hash = u64::from_be_bytes(hash_bytes);
					let metadata = self.read_from_pasteboard(state.pasteboard, state.metadata_pasteboard_type)?;

					if hash == ClipboardString::text_hash(&text) {
						String::from_utf8(metadata.to_vec()).ok()
					} else {
						None
					}
				});

			ClipboardItem {
				entries: vec![ClipboardEntry::String(ClipboardString { text, metadata })]
			}
		}
	}

	unsafe fn write_plaintext_to_clipboard(&self, string: &ClipboardString) {
		unsafe {
			let state = self.0.lock();
			state.pasteboard.clearContents();

			let text_bytes = NSData::dataWithBytes_length_(nil, string.text.as_ptr() as *const c_void, string.text.len() as u64);
			state.pasteboard.setData_forType(text_bytes, NSPasteboardTypeString);

			if let Some(metadata) = string.metadata.as_ref() {
				let hash_bytes = ClipboardString::text_hash(&string.text).to_be_bytes();
				let hash_bytes = NSData::dataWithBytes_length_(nil, hash_bytes.as_ptr() as *const c_void, hash_bytes.len() as u64);
				state.pasteboard.setData_forType(hash_bytes, state.text_hash_pasteboard_type);

				let metadata_bytes = NSData::dataWithBytes_length_(nil, metadata.as_ptr() as *const c_void, metadata.len() as u64);
				state.pasteboard.setData_forType(metadata_bytes, state.metadata_pasteboard_type);
			}
		}
	}

	unsafe fn write_image_to_clipboard(&self, image: &Image) {
		unsafe {
			let state = self.0.lock();
			state.pasteboard.clearContents();

			let bytes = NSData::dataWithBytes_length_(nil, image.bytes.as_ptr() as *const c_void, image.bytes.len() as u64);

			state.pasteboard.setData_forType(bytes, Into::<UTType>::into(image.format).inner_mut());
		}
	}
}

fn try_clipboard_image(pasteboard: id, format: ImageFormat) -> Option<ClipboardItem> {
	let mut ut_type: UTType = format.into();

	unsafe {
		let types: id = pasteboard.types();
		if msg_send![types, containsObject: ut_type.inner()] {
			let data = pasteboard.dataForType(ut_type.inner_mut());
			if data == nil {
				None
			} else {
				let bytes = Vec::from(slice::from_raw_parts(data.bytes() as *mut u8, data.length() as usize));
				let id = hash(&bytes);

				Some(ClipboardItem {
					entries: vec![ClipboardEntry::Image(Image { format, bytes, id })]
				})
			}
		} else {
			None
		}
	}
}

unsafe fn path_from_objc(path: id) -> PathBuf {
	let len = msg_send![path, lengthOfBytesUsingEncoding: NSUTF8StringEncoding];
	let bytes = unsafe { path.UTF8String() as *const u8 };
	let path = str::from_utf8(unsafe { slice::from_raw_parts(bytes, len) }).unwrap();
	PathBuf::from(path)
}

unsafe fn get_mac_platform(object: &mut Object) -> &MacPlatform {
	unsafe {
		let platform_ptr: *mut c_void = *object.get_ivar(MAC_PLATFORM_IVAR);
		assert!(!platform_ptr.is_null());
		&*(platform_ptr as *const MacPlatform)
	}
}

extern "C" fn will_finish_launching(_this: &mut Object, _: Sel, _: id) {
	unsafe {
		let user_defaults: id = msg_send![class!(NSUserDefaults), standardUserDefaults];

		// The autofill heuristic controller causes slowdown and high CPU usage.
		// We don't know exactly why. This disables the full heuristic controller.
		//
		// Adapted from: https://github.com/ghostty-org/ghostty/pull/8625
		let name = ns_string("NSAutoFillHeuristicControllerEnabled");
		let existing_value: id = msg_send![user_defaults, objectForKey: name];
		if existing_value == nil {
			let false_value: id = msg_send![class!(NSNumber), numberWithBool:false];
			let _: () = msg_send![user_defaults, setObject: false_value forKey: name];
		}
	}
}

extern "C" fn did_finish_launching(this: &mut Object, _: Sel, _: id) {
	unsafe {
		let app: id = msg_send![APP_CLASS, sharedApplication];
		app.setActivationPolicy_(NSApplicationActivationPolicyRegular);

		let notification_center: *mut Object = msg_send![class!(NSNotificationCenter), defaultCenter];
		let name = ns_string("NSTextInputContextKeyboardSelectionDidChangeNotification");
		let _: () = msg_send![notification_center, addObserver: this as id
			selector: sel!(onKeyboardLayoutChange:)
			name: name
			object: nil
		];

		let platform = get_mac_platform(this);
		let callback = platform.0.lock().finish_launching.take();
		if let Some(callback) = callback {
			callback();
		}
	}
}

extern "C" fn should_handle_reopen(this: &mut Object, _: Sel, _: id, has_open_windows: bool) {
	if !has_open_windows {
		let platform = unsafe { get_mac_platform(this) };
		let mut lock = platform.0.lock();
		if let Some(mut callback) = lock.reopen.take() {
			drop(lock);
			callback();
			platform.0.lock().reopen.get_or_insert(callback);
		}
	}
}

extern "C" fn will_terminate(this: &mut Object, _: Sel, _: id) {
	let platform = unsafe { get_mac_platform(this) };
	let mut lock = platform.0.lock();
	if let Some(mut callback) = lock.quit.take() {
		drop(lock);
		callback();
		platform.0.lock().quit.get_or_insert(callback);
	}
}

extern "C" fn on_keyboard_layout_change(this: &mut Object, _: Sel, _: id) {
	let platform = unsafe { get_mac_platform(this) };
	let mut lock = platform.0.lock();
	let keyboard_layout = MacKeyboardLayout::new();
	lock.keyboard_mapper = Rc::new(MacKeyboardMapper::new(keyboard_layout.id()));
	if let Some(mut callback) = lock.on_keyboard_layout_change.take() {
		drop(lock);
		callback();
		platform.0.lock().on_keyboard_layout_change.get_or_insert(callback);
	}
}

extern "C" fn handle_menu_item(this: &mut Object, _: Sel, item: id) {
	unsafe {
		let platform = get_mac_platform(this);
		let mut lock = platform.0.lock();
		if let Some(mut callback) = lock.menu_command.take() {
			let tag: NSInteger = msg_send![item, tag];
			let index = tag as usize;
			if let Some(action) = lock.menu_actions.get(index) {
				let action = action.boxed_clone();
				drop(lock);
				callback(&*action);
			}
			platform.0.lock().menu_command.get_or_insert(callback);
		}
	}
}

extern "C" fn validate_menu_item(this: &mut Object, _: Sel, item: id) -> bool {
	unsafe {
		let mut result = false;
		let platform = get_mac_platform(this);
		let mut lock = platform.0.lock();
		if let Some(mut callback) = lock.validate_menu_command.take() {
			let tag: NSInteger = msg_send![item, tag];
			let index = tag as usize;
			if let Some(action) = lock.menu_actions.get(index) {
				let action = action.boxed_clone();
				drop(lock);
				result = callback(action.as_ref());
			}
			platform.0.lock().validate_menu_command.get_or_insert(callback);
		}
		result
	}
}

extern "C" fn menu_will_open(this: &mut Object, _: Sel, _: id) {
	unsafe {
		let platform = get_mac_platform(this);
		let mut lock = platform.0.lock();
		if let Some(mut callback) = lock.will_open_menu.take() {
			drop(lock);
			callback();
			platform.0.lock().will_open_menu.get_or_insert(callback);
		}
	}
}

extern "C" fn handle_dock_menu(this: &mut Object, _: Sel, _: id) -> id {
	unsafe {
		let platform = get_mac_platform(this);
		let mut state = platform.0.lock();
		if let Some(id) = state.dock_menu { id } else { nil }
	}
}

unsafe fn ns_string(string: &str) -> id {
	unsafe { NSString::alloc(nil).init_str(string).autorelease() }
}

unsafe fn ns_url_to_path(url: id) -> Result<PathBuf> {
	let path: *mut c_char = msg_send![url, fileSystemRepresentation];
	anyhow::ensure!(!path.is_null(), "url is not a file path: {}", unsafe { CStr::from_ptr(url.absoluteString().UTF8String()).to_string_lossy() });
	Ok(PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(path).to_bytes() })))
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
	pub(super) fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut Object;
	pub(super) fn TISGetInputSourceProperty(inputSource: *mut Object, propertyKey: *const c_void) -> *mut Object;

	pub(super) fn UCKeyTranslate(
		keyLayoutPtr: *const ::std::os::raw::c_void,
		virtualKeyCode: u16,
		keyAction: u16,
		modifierKeyState: u32,
		keyboardType: u32,
		keyTranslateOptions: u32,
		deadKeyState: *mut u32,
		maxStringLength: usize,
		actualStringLength: *mut usize,
		unicodeString: *mut u16
	) -> u32;
	pub(super) fn LMGetKbdType() -> u16;
	pub(super) static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
	pub(super) static kTISPropertyInputSourceID: CFStringRef;
	pub(super) static kTISPropertyLocalizedName: CFStringRef;
}

mod security {
	#![allow(non_upper_case_globals)]
	use super::*;

	#[link(name = "Security", kind = "framework")]
	unsafe extern "C" {
		pub static kSecClass: CFStringRef;
		pub static kSecClassInternetPassword: CFStringRef;
		pub static kSecAttrServer: CFStringRef;
		pub static kSecAttrAccount: CFStringRef;
		pub static kSecValueData: CFStringRef;
		pub static kSecReturnAttributes: CFStringRef;
		pub static kSecReturnData: CFStringRef;

		pub fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
		pub fn SecItemUpdate(query: CFDictionaryRef, attributes: CFDictionaryRef) -> OSStatus;
		pub fn SecItemDelete(query: CFDictionaryRef) -> OSStatus;
		pub fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
	}

	pub const errSecSuccess: OSStatus = 0;
	pub const errSecUserCanceled: OSStatus = -128;
	pub const errSecItemNotFound: OSStatus = -25300;
}

impl From<ImageFormat> for UTType {
	fn from(value: ImageFormat) -> Self {
		match value {
			ImageFormat::Png => Self::png(),
			ImageFormat::Jpeg => Self::jpeg(),
			ImageFormat::Tiff => Self::tiff(),
			ImageFormat::Webp => Self::webp(),
			ImageFormat::Gif => Self::gif(),
			ImageFormat::Bmp => Self::bmp(),
			ImageFormat::Svg => Self::svg(),
			ImageFormat::Ico => Self::ico()
		}
	}
}

// See https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/
struct UTType(id);

impl UTType {
	pub fn png() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/png
		Self(unsafe { NSPasteboardTypePNG }) // This is a rare case where there's a built-in NSPasteboardType
	}

	pub fn jpeg() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/jpeg
		Self(unsafe { ns_string("public.jpeg") })
	}

	pub fn gif() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/gif
		Self(unsafe { ns_string("com.compuserve.gif") })
	}

	pub fn webp() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/webp
		Self(unsafe { ns_string("org.webmproject.webp") })
	}

	pub fn bmp() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/bmp
		Self(unsafe { ns_string("com.microsoft.bmp") })
	}

	pub fn svg() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/svg
		Self(unsafe { ns_string("public.svg-image") })
	}

	pub fn ico() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/ico
		Self(unsafe { ns_string("com.microsoft.ico") })
	}

	pub fn tiff() -> Self {
		// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/tiff
		Self(unsafe { NSPasteboardTypeTIFF }) // This is a rare case where there's a built-in NSPasteboardType
	}

	fn inner(&self) -> *const Object {
		self.0
	}

	fn inner_mut(&self) -> *mut Object {
		self.0 as *mut _
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ClipboardItem;

	#[test]
	fn test_clipboard() {
		let platform = build_platform();
		assert_eq!(platform.read_from_clipboard(), None);

		let item = ClipboardItem::new_string("1".to_string());
		platform.write_to_clipboard(item.clone());
		assert_eq!(platform.read_from_clipboard(), Some(item));

		let item = ClipboardItem {
			entries: vec![ClipboardEntry::String(ClipboardString::new("2".to_string()).with_metadata("[3,4]".to_string()))]
		};
		platform.write_to_clipboard(item.clone());
		assert_eq!(platform.read_from_clipboard(), Some(item));

		let text_from_other_app = "text from other app";
		unsafe {
			let bytes = NSData::dataWithBytes_length_(nil, text_from_other_app.as_ptr() as *const c_void, text_from_other_app.len() as u64);
			platform.0.lock().pasteboard.setData_forType(bytes, NSPasteboardTypeString);
		}
		assert_eq!(platform.read_from_clipboard(), Some(ClipboardItem::new_string(text_from_other_app.to_string())));
	}

	fn build_platform() -> MacPlatform {
		let platform = MacPlatform::new(false);
		platform.0.lock().pasteboard = unsafe { NSPasteboard::pasteboardWithUniqueName(nil) };
		platform
	}
}
