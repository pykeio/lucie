use std::{
	sync::atomic::{AtomicBool, Ordering},
	thread::{ThreadId, current}
};

use anyhow::Context;
use lucie_common::ResultExt;
use windows::Win32::{
	Foundation::{LPARAM, WPARAM},
	System::Threading::{
		GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_IDLE, THREAD_PRIORITY_NORMAL,
		THREAD_PRIORITY_TIME_CRITICAL
	},
	UI::WindowsAndMessaging::PostMessageW
};

use crate::{
	GLOBAL_THREAD_TIMINGS, HWND, PlatformDispatcher, PriorityQueueSender, Runnable, SafeHwnd, THREAD_TIMINGS, ThreadPriority, ThreadTaskTimings,
	WM_LUCIE_TASK_DISPATCHED_ON_MAIN_THREAD
};

pub(crate) struct WindowsDispatcher {
	pub(crate) wake_posted: AtomicBool,
	main_sender: PriorityQueueSender<Runnable>,
	main_thread_id: ThreadId,
	pub(crate) platform_window_handle: SafeHwnd,
	validation_number: usize
}

impl WindowsDispatcher {
	pub(crate) fn new(main_sender: PriorityQueueSender<Runnable>, platform_window_handle: HWND, validation_number: usize) -> Self {
		let main_thread_id = current().id();
		let platform_window_handle = platform_window_handle.into();

		WindowsDispatcher {
			main_sender,
			main_thread_id,
			platform_window_handle,
			validation_number,
			wake_posted: AtomicBool::new(false)
		}
	}
}

impl PlatformDispatcher for WindowsDispatcher {
	fn get_all_timings(&self) -> Vec<ThreadTaskTimings> {
		let global_thread_timings = GLOBAL_THREAD_TIMINGS.lock();
		ThreadTaskTimings::convert(&global_thread_timings)
	}

	fn get_current_thread_timings(&self) -> Vec<crate::TaskTiming> {
		THREAD_TIMINGS.with(|timings| {
			let timings = timings.lock();
			let timings = &timings.timings;

			let mut vec = Vec::with_capacity(timings.len());

			let (s1, s2) = timings.as_slices();
			vec.extend_from_slice(s1);
			vec.extend_from_slice(s2);
			vec
		})
	}

	fn is_main_thread(&self) -> bool {
		current().id() == self.main_thread_id
	}

	fn dispatch_on_main_thread(&self, runnable: Runnable) {
		match self.main_sender.send(runnable.priority(), runnable) {
			Ok(_) => {
				if !self.wake_posted.swap(true, Ordering::AcqRel) {
					unsafe {
						PostMessageW(
							Some(self.platform_window_handle.as_raw()),
							WM_LUCIE_TASK_DISPATCHED_ON_MAIN_THREAD,
							WPARAM(self.validation_number),
							LPARAM(0)
						)
						.log_err();
					}
				}
			}
			Err(runnable) => {
				// NOTE: Runnable may wrap a Future that is !Send.
				//
				// This is usually safe because we only poll it on the main thread.
				// However if the send fails, we know that:
				// 1. main_receiver has been dropped (which implies the app is shutting down)
				// 2. we are on a background thread.
				// It is not safe to drop something !Send on the wrong thread, and
				// the app will exit soon anyway, so we must forget the runnable.
				std::mem::forget(runnable);
			}
		}
	}

	fn set_thread_priority(&self, priority: ThreadPriority) {
		let thread_handle = unsafe { GetCurrentThread() };
		let thread_priority = match priority {
			ThreadPriority::Critical => THREAD_PRIORITY_TIME_CRITICAL,
			ThreadPriority::High => THREAD_PRIORITY_HIGHEST,
			ThreadPriority::Normal => THREAD_PRIORITY_NORMAL,
			ThreadPriority::Low => THREAD_PRIORITY_BELOW_NORMAL,
			ThreadPriority::Background => THREAD_PRIORITY_IDLE
		};

		unsafe { SetThreadPriority(thread_handle, thread_priority) }
			.context("thread priority")
			.log_err();
	}
}
