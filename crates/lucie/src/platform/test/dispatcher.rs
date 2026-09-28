use std::{collections::VecDeque, sync::Arc};

use fastrand::Rng;
use lucie_common::post_inc;
use parking_lot::Mutex;
use rapidhash::{HashMapExt, fast::RapidHashMap};

use crate::{PlatformDispatcher, Runnable};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct TestDispatcherId(usize);

#[doc(hidden)]
pub struct TestDispatcher {
	id: TestDispatcherId,
	state: Arc<Mutex<TestDispatcherState>>
}

struct TestDispatcherState {
	random: Rng,
	processes: RapidHashMap<TestDispatcherId, VecDeque<Runnable>>,
	next_id: TestDispatcherId
}

impl TestDispatcher {
	pub fn new(random: Rng) -> Self {
		let state = TestDispatcherState {
			random,
			processes: RapidHashMap::new(),
			next_id: TestDispatcherId(1)
		};
		TestDispatcher {
			id: TestDispatcherId(0),
			state: Arc::new(Mutex::new(state))
		}
	}

	pub fn tick(&self) -> bool {
		let mut state_lock = self.state.lock();

		if state_lock.processes.values().map(|runnables| runnables.len()).sum::<usize>() == 0 {
			return false;
		}

		let state = &mut *state_lock;
		let runnable = state
			.random
			.choice(state.processes.values_mut().filter(|runnables| !runnables.is_empty()).collect::<Vec<_>>())
			.unwrap()
			.pop_front()
			.unwrap();

		drop(state_lock);

		// todo(localcc): add timings to tests
		if !runnable.app_dropped() {
			runnable.run_unprofiled();
		}

		true
	}

	pub fn run_until_parked(&self) {
		while self.tick() {}
	}

	pub fn rng(&self) -> Rng {
		self.state.lock().random.clone()
	}
}

impl Clone for TestDispatcher {
	fn clone(&self) -> Self {
		let id = post_inc(&mut self.state.lock().next_id.0);
		Self {
			id: TestDispatcherId(id),
			state: self.state.clone()
		}
	}
}

impl PlatformDispatcher for TestDispatcher {
	fn get_all_timings(&self) -> Vec<crate::ThreadTaskTimings> {
		Vec::new()
	}

	fn get_current_thread_timings(&self) -> Vec<crate::TaskTiming> {
		Vec::new()
	}

	fn is_main_thread(&self) -> bool {
		true
	}

	fn dispatch_on_main_thread(&self, runnable: Runnable) {
		self.state.lock().processes.entry(self.id).or_default().push_back(runnable);
	}

	fn as_test(&self) -> Option<&TestDispatcher> {
		Some(self)
	}

	fn set_thread_priority(&self, _priority: crate::ThreadPriority) {}
}
