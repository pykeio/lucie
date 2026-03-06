use std::{
	env,
	hash::{Hash, Hasher},
	ops::AddAssign,
	sync::{
		OnceLock,
		atomic::{AtomicUsize, Ordering}
	},
	time::Instant
};

// needed to make #[derive(Refineable)] work
extern crate self as lucie_common;

mod arc_cow;
mod arena;
mod bounds_tree;
pub mod color;
mod defer;
pub mod geometry;
mod ref_scope;
pub mod refineable;
mod result_ext;
mod shared_string;
mod shared_uri;
pub mod sum_tree;
mod trys;

use rapidhash::fast::RapidHasher;

pub use self::{
	arc_cow::ArcCow,
	arena::{Arena, ArenaBox},
	bounds_tree::BoundsTree,
	defer::{Deferred, defer},
	ref_scope::RefScope,
	result_ext::{Flatten, LogLevel, ResultExt},
	shared_string::SharedString,
	shared_uri::SharedUri
};

pub mod __private {
	pub use std;
	pub extern crate tracing;
}

pub fn post_inc<T: From<u8> + AddAssign<T> + Copy>(value: &mut T) -> T {
	let prev = *value;
	*value += T::from(1);
	prev
}

#[inline]
pub const fn mix_hashes(a: u64, b: u64) -> u64 {
	let r = (a as u128).wrapping_mul(b as u128);
	(r as u64) ^ (r >> 64) as u64
}

#[inline]
pub fn hash<H: Hash>(x: &H) -> u64 {
	let mut hasher = RapidHasher::default_const();
	x.hash(&mut hasher);
	hasher.finish()
}

/// Increment the given atomic counter if it is not zero.
/// Return the new value of the counter.
pub fn atomic_incr_if_not_zero(counter: &AtomicUsize) -> usize {
	let mut loaded = counter.load(Ordering::SeqCst);
	loop {
		if loaded == 0 {
			return 0;
		}
		match counter.compare_exchange_weak(loaded, loaded + 1, Ordering::SeqCst, Ordering::SeqCst) {
			Ok(x) => return x + 1,
			Err(actual) => loaded = actual
		}
	}
}

#[macro_export]
macro_rules! debug_panic {
    ($($fmt_arg:tt)*) => {
        if cfg!(debug_assertions) {
            panic!($($fmt_arg)*);
        } else {
            let backtrace = $crate::__private::std::backtrace::Backtrace::capture();
            $crate::__private::tracing::error!("{}\n{}", format_args!($($fmt_arg)*), backtrace);
        }
    };
}

pub fn measure<R>(label: &str, f: impl FnOnce() -> R) -> R {
	static SHOULD_MEASURE: OnceLock<bool> = OnceLock::new();
	let should_measure = SHOULD_MEASURE.get_or_init(|| {
		env::var("LUCIE_MEASURE")
			.map(|measure| measure == "1" || measure == "true" || measure == "yes" || measure == "y")
			.unwrap_or(false)
	});

	if *should_measure {
		let start = Instant::now();
		let result = f();
		let elapsed = start.elapsed();
		tracing::info!(target: "lucie_measure", "{label} took {elapsed:?}");
		result
	} else {
		f()
	}
}
