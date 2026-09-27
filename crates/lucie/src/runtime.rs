use std::{
	cell::Cell,
	fmt::Debug,
	marker::PhantomData,
	mem::{self, ManuallyDrop},
	ops::Deref,
	panic::Location,
	pin::Pin,
	rc::Rc,
	sync::{
		Arc,
		atomic::{AtomicU32, Ordering}
	},
	task::{Context, Poll},
	thread::{self, ThreadId},
	time::Duration
};

use futures_util::FutureExt;
use tokio::{
	runtime::{Handle, Runtime as TokioRuntime},
	sync::mpsc,
	task::JoinHandle
};

use crate::{PlatformDispatcher, Runnable, RunnableMeta, ThreadPriority};

/// An async multithreaded runtime based on [`tokio`] used for background tasks that shouldn't block the main thread.
///
/// To perform work on the main thread, see [`Dispatcher`].
#[derive(Clone)]
pub struct Runtime {
	normal_runtime: Arc<TimedShutdownRuntime>,
	low_runtime: Arc<TimedShutdownRuntime>,

	#[cfg(any(test, feature = "test-support"))]
	dispatcher: Arc<dyn PlatformDispatcher>
}

struct TimedShutdownRuntime {
	runtime: ManuallyDrop<TokioRuntime>,
	timeout_ms: AtomicU32
}
impl TimedShutdownRuntime {
	fn new(runtime: TokioRuntime) -> Self {
		Self {
			runtime: ManuallyDrop::new(runtime),
			timeout_ms: AtomicU32::new(0)
		}
	}
}
impl Deref for TimedShutdownRuntime {
	type Target = TokioRuntime;

	fn deref(&self) -> &Self::Target {
		&*self.runtime
	}
}
impl Drop for TimedShutdownRuntime {
	fn drop(&mut self) {
		unsafe { ManuallyDrop::take(&mut self.runtime) }.shutdown_timeout(Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed) as _));
	}
}

/// The `Dispatcher` sends tasks (called *[`Process`]es* to distinguish them from normal background [`Task`]s) to be
/// executed on the main thread.
///
/// Unlike [`Runtime`] tasks, dispatched processes *cannot use `tokio` APIs* like timers, filesystem, or networking.
/// They can, however, send & receive messages to/from tasks (which *can* use those APIs) over
/// [`mpsc`]/[`oneshot`]/[`broadcast`] channels.
///
/// [`mpsc`]: tokio::sync::mpsc
/// [`oneshot`]: tokio::sync::oneshot
/// [`broadcast`]: tokio::sync::broadcast
// This is intentionally `!Send` via the `not_send` marker field. This is because
// `Dispatcher::spawn` does not require `Send` but checks at runtime that the future is
// only polled from the same thread it was spawned from. These checks would fail when dispatching processes
// from runtime threads.
#[derive(Clone)]
pub struct Dispatcher {
	#[doc(hidden)]
	pub platform: Arc<dyn PlatformDispatcher>,
	liveness: std::sync::Weak<()>,
	not_send: PhantomData<Rc<()>>
}

thread_local! {
	static CURRENT_TASKS_PRIORITY: Cell<TaskPriority> = const { Cell::new(TaskPriority::Normal) };
}

/// Task priority
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskPriority {
	/// Normal priority with most threads available; used for tasks that might be somewhat compute-bound.
	#[default]
	Normal,
	/// Low priority for less time-critical tasks like I/O.
	Low
}

impl TaskPriority {
	/// Sets the priority any spawn call from the runnable about to be run will use
	pub(crate) fn set_as_default_for_spawns(&self) {
		CURRENT_TASKS_PRIORITY.set(*self);
	}

	/// Inherits the priority from the currently running task (or the default [`Self::Normal`] if not called from a
	/// task).
	pub fn inherit() -> Self {
		CURRENT_TASKS_PRIORITY.get()
	}

	pub(crate) const fn probability(&self) -> u32 {
		match self {
			TaskPriority::Normal => 70,
			TaskPriority::Low => 30
		}
	}
}

pin_project_lite::pin_project! {
	#[derive(Debug)]
	pub struct Task<T> {
		#[pin]
		inner: JoinHandle<T>
	}
}

impl<T> Task<T> {
	pub fn fallible(self) -> JoinHandle<T> {
		self.inner
	}
}

impl<T> Future for Task<T> {
	type Output = T;

	#[inline(always)]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let this = self.project();
		match this.inner.poll(cx) {
			Poll::Ready(Ok(res)) => Poll::Ready(res),
			Poll::Ready(Err(e)) => panic!("{e}"),
			Poll::Pending => Poll::Pending
		}
	}
}

/// A task spawned by the [`Dispatcher`] scheduled to be run on the main thread.
#[must_use]
#[derive(Debug)]
pub struct Process<T>(async_task::Task<T, RunnableMeta>);

impl<T> Process<T> {
	/// Runs the process to completion, without blocking.
	pub fn detach(self) {
		self.0.detach();
	}

	/// Converts this task into a fallible task that returns `Option<T>`.
	///
	/// Unlike the standard `Task<T>`, a [`FallibleTask`] will return `None`
	/// if the app was dropped while the task is executing.
	///
	/// # Example
	///
	/// ```ignore
	/// // Background task that gracefully handles app shutdown:
	/// cx.spawn(async move {
	///     let result = foreground_task.fallible().await;
	///     if let Some(value) = result {
	///         // Process the value
	///     }
	///     // If None, app was shut down - just exit gracefully
	/// });
	/// ```
	pub fn fallible(self) -> FallibleForegroundTask<T> {
		FallibleForegroundTask(self.0.fallible())
	}
}

impl<T> Future for Process<T> {
	type Output = T;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		Pin::new(&mut unsafe { self.get_unchecked_mut() }.0).poll(cx)
	}
}

/// A task that returns `Option<T>` instead of panicking when cancelled.
#[must_use]
#[derive(Debug)]
pub struct FallibleForegroundTask<T>(async_task::FallibleTask<T, RunnableMeta>);

impl<T> FallibleForegroundTask<T> {
	/// Detaching a task runs it to completion in the background.
	pub fn detach(self) {
		self.0.detach();
	}
}

impl<T> Future for FallibleForegroundTask<T> {
	type Output = Option<T>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		Pin::new(&mut unsafe { self.get_unchecked_mut() }.0).poll(cx)
	}
}

impl Runtime {
	#[doc(hidden)]
	pub fn new(dispatcher: Arc<dyn PlatformDispatcher>) -> Self {
		#[cfg(any(test, feature = "test-support"))]
		{
			if dispatcher.as_test().is_some() {
				let runtime = Arc::new(TimedShutdownRuntime::new(tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()));
				return Self {
					normal_runtime: Arc::clone(&runtime),
					low_runtime: runtime,

					#[cfg(any(test, feature = "test-support"))]
					dispatcher
				};
			}
		}

		let cpus = num_cpus::get();
		let (normal_count, low_count) = if cpus < 4 {
			((cpus.saturating_sub(1).max(1)), 1)
		} else if cpus < 8 {
			(cpus - 2, 2)
		} else {
			((cpus - 3).min(8), 3)
		};

		let normal_runtime = Arc::new(TimedShutdownRuntime::new(
			tokio::runtime::Builder::new_multi_thread()
				.name("lucie")
				.enable_all()
				.thread_name("luciert")
				.worker_threads(normal_count)
				.build()
				.unwrap()
		));
		let low_runtime = Arc::new(TimedShutdownRuntime::new(
			tokio::runtime::Builder::new_multi_thread()
				.name("lucie-low")
				.enable_all()
				.thread_name("luciert-low")
				.worker_threads(low_count)
				.on_thread_start({
					let dispatcher = Arc::clone(&dispatcher);
					move || {
						dispatcher.set_thread_priority(ThreadPriority::Low);
					}
				})
				.build()
				.unwrap()
		));

		Self {
			normal_runtime,
			low_runtime,

			#[cfg(any(test, feature = "test-support"))]
			dispatcher
		}
	}

	/// Enqueues the given future to be run to completion on a background thread.
	#[track_caller]
	pub fn spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
	where
		R: Send + 'static
	{
		self.spawn_with_priority(TaskPriority::inherit(), future)
	}

	/// Enqueues the given future to be run to completion on a background thread.
	#[track_caller]
	pub fn spawn_with_priority<R>(&self, priority: TaskPriority, future: impl Future<Output = R> + Send + 'static) -> Task<R>
	where
		R: Send + 'static
	{
		let runtime = match priority {
			TaskPriority::Normal => &self.normal_runtime,
			TaskPriority::Low => &self.low_runtime
		};
		Task { inner: runtime.spawn(future) }
	}

	/// Block the current thread until the given future resolves.
	/// Consider using `block_with_timeout` instead.
	pub fn block<R>(&self, priority: TaskPriority, future: impl Future<Output = R>) -> R {
		#[cfg(any(test, feature = "test-support"))]
		{
			let Ok(v) = self.block_with_timeout(
				priority,
				Duration::from_secs(option_env!("LUCIE_TEST_TIMEOUT").and_then(|s| s.parse::<u64>().ok()).unwrap_or(180)),
				future
			) else {
				panic!("task timed out");
			};
			return v;
		}

		#[cfg(not(any(test, feature = "test-support")))]
		{
			let runtime = match priority {
				TaskPriority::Normal => &self.normal_runtime,
				TaskPriority::Low => &self.low_runtime
			};
			runtime.block_on(future)
		}
	}

	/// Block the current thread until the given future resolves
	/// or `duration` has elapsed.
	pub fn block_with_timeout<Fut: Future>(&self, priority: TaskPriority, duration: Duration, future: Fut) -> Result<Fut::Output, tokio::time::error::Elapsed> {
		let runtime = match priority {
			TaskPriority::Normal => &self.normal_runtime,
			TaskPriority::Low => &self.low_runtime
		};
		let _enter = runtime.enter();
		runtime.block_on(tokio::time::timeout(duration, future))
	}

	#[cfg(any(test, feature = "test-support"))]
	#[doc(hidden)]
	pub fn block_test<Fut: Future>(&self, future: Fut) -> Result<Fut::Output, tokio::time::error::Elapsed> {
		// Tests are single-threaded; since we're blocking w/ tokio, we need some way to tick dispatched processes. We
		// do that by hijacking `future`'s poll and ticking the dispatcher while the future is still pending.
		// Since `future` is the test entrypoint, it's always pending until the test finishes.
		pin_project_lite::pin_project! {
			struct DispatcherTicker<Fut> {
				dispatcher: Arc<dyn PlatformDispatcher>,
				#[pin]
				future: Fut
			}
		}

		impl<Fut: Future> Future for DispatcherTicker<Fut> {
			type Output = Fut::Output;

			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				let this = self.project();
				let Some(dispatcher) = this.dispatcher.as_test() else {
					return this.future.poll(cx);
				};

				match this.future.poll(cx) {
					Poll::Ready(v) => Poll::Ready(v),
					Poll::Pending => {
						if !dispatcher.tick() {
							return Poll::Pending;
						}

						// Ask tokio to immediately poll us again. This is exactly as horrible as it sounds.
						// I actually think this whole thing could be replaced with a `Builder::on_after_task_poll` hook
						// but that's an unstable API...
						cx.waker().wake_by_ref();
						Poll::Pending
					}
				}
			}
		}

		let duration = Duration::from_secs(option_env!("LUCIE_TEST_TIMEOUT").and_then(|s| s.parse::<u64>().ok()).unwrap_or(180));
		let _enter = self.normal_runtime.enter();
		self.normal_runtime.block_on(tokio::time::timeout(
			duration,
			DispatcherTicker {
				dispatcher: self.dispatcher.clone(),
				future
			}
		))
	}

	/// Scoped lets you start a number of tasks and waits
	/// for all of them to complete before returning.
	pub async fn scoped<'scope, F>(&self, priority: TaskPriority, scheduler: F)
	where
		F: FnOnce(&mut Scope<'scope>)
	{
		let mut scope = Scope::new(self.normal_runtime.handle().clone(), priority);
		(scheduler)(&mut scope);
		let spawned = mem::take(&mut scope.futures)
			.into_iter()
			.map(|f| self.spawn_with_priority(scope.priority, f))
			.collect::<Vec<_>>();
		for task in spawned {
			task.await;
		}
	}

	/// Returns a task that will complete after the given duration.
	/// Depending on other concurrent tasks the elapsed duration may be longer
	/// than requested.
	pub fn timer(&self, duration: Duration) -> impl Future<Output = ()> + use<> {
		let _enter = self.normal_runtime.enter();
		tokio::time::sleep(duration)
	}

	/// Sets the maximum amount of time in milliseconds to wait for running tasks to finish when the app closes.
	///
	/// The default is `0`, meaning all tasks are aborted immediately on app exit.
	pub fn set_shutdown_timeout(&self, timeout_ms: u32) {
		self.normal_runtime.timeout_ms.store(timeout_ms, Ordering::Release);
	}
}

impl Dispatcher {
	#[doc(hidden)]
	pub fn new(platform: Arc<dyn PlatformDispatcher>, liveness: std::sync::Weak<()>) -> Self {
		Self {
			platform,
			liveness,
			not_send: PhantomData
		}
	}

	/// Schedules the given task to run on the main thread at some point in the future.
	#[track_caller]
	pub fn dispatch<R>(&self, future: impl Future<Output = R> + 'static) -> Process<R>
	where
		R: 'static
	{
		self.inner_dispatch(self.liveness.clone(), TaskPriority::inherit(), future.boxed_local())
	}

	/// Schedules the given task to run on the main thread at some point in the future, with a given priority level.
	#[track_caller]
	pub fn dispatch_with_priority<R>(&self, priority: TaskPriority, future: impl Future<Output = R> + 'static) -> Process<R>
	where
		R: 'static
	{
		self.inner_dispatch(self.liveness.clone(), priority, future.boxed_local())
	}

	#[track_caller]
	pub(crate) fn inner_dispatch<R>(&self, liveness: std::sync::Weak<()>, priority: TaskPriority, future: impl Future<Output = R> + 'static) -> Process<R>
	where
		R: 'static
	{
		let dispatcher = self.platform.clone();
		let location = core::panic::Location::caller();

		let (runnable, task) = spawn_local_with_source_location(
			future,
			move |runnable| dispatcher.dispatch_on_main_thread(Runnable(runnable)),
			RunnableMeta {
				location,
				liveness: Some(liveness),
				priority
			}
		);
		runnable.schedule();
		Process(task)
	}

	/// in tests, run all tasks that are ready to run. If after doing so
	/// the test still has outstanding tasks, this will panic. (See also [`Self::allow_parking`])
	#[cfg(any(test, feature = "test-support"))]
	pub fn run_until_parked(&self) {
		self.platform.as_test().unwrap().run_until_parked()
	}
}

/// Variant of `async_task::spawn_local` that includes the source location of the spawn in panics.
///
/// Copy-modified from:
/// <https://github.com/smol-rs/async-task/blob/ca9dbe1db9c422fd765847fa91306e30a6bb58a9/src/runnable.rs#L405>
#[track_caller]
fn spawn_local_with_source_location<Fut, S, M>(future: Fut, schedule: S, metadata: M) -> (async_task::Runnable<M>, async_task::Task<Fut::Output, M>)
where
	Fut: Future + 'static,
	Fut::Output: 'static,
	S: async_task::Schedule<M> + Send + Sync + 'static,
	M: 'static
{
	#[inline]
	fn thread_id() -> ThreadId {
		std::thread_local! {
			static ID: ThreadId = thread::current().id();
		}
		ID.try_with(|id| *id).unwrap_or_else(|_| thread::current().id())
	}

	struct Checked<F> {
		id: ThreadId,
		inner: ManuallyDrop<F>,
		location: &'static Location<'static>
	}

	impl<F> Drop for Checked<F> {
		fn drop(&mut self) {
			assert!(self.id == thread_id(), "local task dropped by a thread that didn't spawn it. Task spawned at {}", self.location);
			unsafe { ManuallyDrop::drop(&mut self.inner) };
		}
	}

	impl<F: Future> Future for Checked<F> {
		type Output = F::Output;

		fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
			assert!(self.id == thread_id(), "local task polled by a thread that didn't spawn it. Task spawned at {}", self.location);
			unsafe { self.map_unchecked_mut(|c| &mut *c.inner).poll(cx) }
		}
	}

	// Wrap the future into one that checks which thread it's on.
	let future = Checked {
		id: thread_id(),
		inner: ManuallyDrop::new(future),
		location: Location::caller()
	};

	unsafe { async_task::Builder::new().metadata(metadata).spawn_unchecked(move |_| future, schedule) }
}

/// Scope manages a set of tasks that are enqueued and waited on together. See [`Runtime::scoped`].
pub struct Scope<'a> {
	handle: Handle,
	priority: TaskPriority,
	futures: Vec<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
	tx: Option<mpsc::Sender<()>>,
	rx: mpsc::Receiver<()>,
	lifetime: PhantomData<&'a ()>
}

impl<'a> Scope<'a> {
	fn new(handle: Handle, priority: TaskPriority) -> Self {
		let (tx, rx) = mpsc::channel(1);
		Self {
			handle,
			priority,
			tx: Some(tx),
			rx,
			futures: Default::default(),
			lifetime: PhantomData
		}
	}

	/// Spawn a future into this scope.
	#[track_caller]
	pub fn spawn<F>(&mut self, f: F)
	where
		F: Future<Output = ()> + Send + 'a
	{
		let tx = self.tx.clone().unwrap();

		// SAFETY: The 'a lifetime is guaranteed to outlive any of these futures because
		// dropping this `Scope` blocks until all of the futures have resolved.
		let f = unsafe {
			mem::transmute::<Pin<Box<dyn Future<Output = ()> + Send + 'a>>, Pin<Box<dyn Future<Output = ()> + Send + 'static>>>(Box::pin(async move {
				f.await;
				drop(tx);
			}))
		};
		self.futures.push(f);
	}
}

impl Drop for Scope<'_> {
	fn drop(&mut self) {
		self.tx.take().unwrap();

		// Wait until the channel is closed, which means that all of the spawned
		// futures have resolved.
		self.handle.block_on(self.rx.recv());
	}
}

#[cfg(test)]
mod test {
	use std::cell::RefCell;

	use fastrand::Rng;

	use super::*;
	use crate::{App, TestDispatcher, TestPlatform, http::FakeHttpClient};

	fn create_test_app() -> Rc<crate::AppCell> {
		let platform_dispatcher = TestDispatcher::new(Rng::with_seed(0));
		let arc_dispatcher = Arc::new(platform_dispatcher.clone());
		// Create liveness for task cancellation
		let liveness = std::sync::Arc::new(());
		let liveness_weak = std::sync::Arc::downgrade(&liveness);
		let runtime = Runtime::new(arc_dispatcher.clone());
		let dispatcher = Dispatcher::new(arc_dispatcher, liveness_weak);

		let platform = TestPlatform::new(runtime.clone(), dispatcher);
		let asset_source = Arc::new(());
		let http_client = FakeHttpClient::with_404_response();

		App::new_app(platform, liveness, asset_source, http_client)
	}

	#[test]
	fn sanity_test_tasks_run() {
		let app = create_test_app();
		let dispatcher = app.borrow().dispatcher.clone();

		let task_ran = Rc::new(RefCell::new(false));

		dispatcher
			.dispatch({
				let task_ran = Rc::clone(&task_ran);
				async move {
					*task_ran.borrow_mut() = true;
				}
			})
			.detach();

		// Run dispatcher while app is still alive
		dispatcher.run_until_parked();

		// Task should have run
		assert!(*task_ran.borrow(), "Task should run normally when app is alive");
	}

	#[test]
	fn test_task_cancelled_when_app_dropped() {
		let app = create_test_app();
		let dispatcher = app.borrow().dispatcher.clone();

		let app_weak = Rc::downgrade(&app);

		let task_ran = Rc::new(RefCell::new(false));
		let task_ran_clone = Rc::clone(&task_ran);

		dispatcher
			.dispatch(async move {
				*task_ran_clone.borrow_mut() = true;
			})
			.detach();

		drop(app);

		assert!(app_weak.upgrade().is_none(), "App should have been dropped");

		dispatcher.run_until_parked();

		// The task should have been cancelled, not run
		assert!(!*task_ran.borrow(), "Task should have been cancelled when app was dropped, but it ran!");
	}

	#[test]
	fn test_nested_tasks_both_cancel() {
		let app = create_test_app();
		let dispatcher = app.borrow().dispatcher.clone();

		let app_weak = Rc::downgrade(&app);

		let outer_completed = Rc::new(RefCell::new(false));
		let inner_completed = Rc::new(RefCell::new(false));
		let reached_await = Rc::new(RefCell::new(false));

		let outer_flag = Rc::clone(&outer_completed);
		let inner_flag = Rc::clone(&inner_completed);
		let await_flag = Rc::clone(&reached_await);

		// Channel to block the inner task until we're ready
		let (tx, rx) = tokio::sync::oneshot::channel::<()>();

		// We need clones of the dispatcher and liveness_token for the inner spawn
		let dispatcher2 = dispatcher.clone();

		dispatcher
			.dispatch(async move {
				let inner_task = dispatcher2.dispatch({
					let inner_flag = Rc::clone(&inner_flag);
					async move {
						rx.await.ok();
						*inner_flag.borrow_mut() = true;
					}
				});

				*await_flag.borrow_mut() = true;

				inner_task.await;

				*outer_flag.borrow_mut() = true;
			})
			.detach();

		// Run dispatcher until outer task reaches the await point
		// The inner task will be blocked on the channel
		dispatcher.run_until_parked();

		// Verify we actually reached the await point before dropping the app
		assert!(*reached_await.borrow(), "Outer task should have reached the await point");

		// Neither task should have completed yet
		assert!(!*outer_completed.borrow(), "Outer task should not have completed yet");
		assert!(!*inner_completed.borrow(), "Inner task should not have completed yet");

		// Drop the channel sender and app while outer is awaiting inner
		drop(tx);
		drop(app);
		assert!(app_weak.upgrade().is_none(), "App should have been dropped");

		// Run dispatcher - both tasks should be cancelled
		dispatcher.run_until_parked();

		// Neither task should have completed (both were cancelled)
		assert!(!*outer_completed.borrow(), "Outer task should have been cancelled, not completed");
		assert!(!*inner_completed.borrow(), "Inner task should have been cancelled, not completed");
	}

	#[test]
	#[should_panic]
	fn test_polling_cancelled_task_panics() {
		let app = create_test_app();
		let dispatcher = app.borrow().dispatcher.clone();
		let runtime = app.borrow().runtime.clone();

		let app_weak = Rc::downgrade(&app);

		let task = dispatcher.dispatch(async move { 42 });

		drop(app);

		assert!(app_weak.upgrade().is_none(), "App should have been dropped");

		dispatcher.run_until_parked();

		runtime.block(TaskPriority::Normal, task);
	}

	#[test]
	fn test_polling_cancelled_task_returns_none_with_fallible() {
		let app = create_test_app();
		let dispatcher = app.borrow().dispatcher.clone();
		let runtime = app.borrow().runtime.clone();

		let app_weak = Rc::downgrade(&app);

		let task = dispatcher.dispatch(async move { 42 }).fallible();

		drop(app);

		assert!(app_weak.upgrade().is_none(), "App should have been dropped");

		dispatcher.run_until_parked();

		let result = runtime.block(TaskPriority::Normal, task);
		assert_eq!(result, None, "Cancelled task should return None");
	}
}
