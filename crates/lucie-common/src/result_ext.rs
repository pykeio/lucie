use std::{fmt, panic::Location};

use tracing::Level;

/// `Result::flatten` for Rust 1.88.
pub trait Flatten<T, E> {
	/// Converts from `Result<Result<T, E2>, E>` to `Result<T, E>`.
	fn flatten(self) -> Result<T, E>;
}

impl<T, E, E2> Flatten<T, E> for Result<Result<T, E>, E2>
where
	E: From<E2>
{
	fn flatten(self) -> Result<T, E> {
		self?
	}
}

impl<T, E> Flatten<T, E> for Result<T, E> {
	fn flatten(self) -> Result<T, E> {
		self
	}
}

pub trait ResultExt<E> {
	type Ok;

	fn log_err(self) -> Option<Self::Ok>;
	fn log_with_level(self, level: Level) -> Option<Self::Ok>;
}

impl<T, E> ResultExt<E> for Result<T, E>
where
	E: std::fmt::Debug
{
	type Ok = T;

	#[track_caller]
	fn log_err(self) -> Option<T> {
		self.log_with_level(Level::ERROR)
	}

	#[track_caller]
	fn log_with_level(self, level: Level) -> Option<T> {
		match self {
			Ok(value) => Some(value),
			Err(error) => {
				log_error_with_caller(*Location::caller(), error, level);
				None
			}
		}
	}
}

fn log_error_with_caller<E>(caller: Location<'static>, error: E, level: Level)
where
	E: fmt::Debug
{
	#[cfg(not(target_os = "windows"))]
	let file = caller.file();
	#[cfg(target_os = "windows")]
	let file = caller.file().replace('\\', "/");
	// In this codebase all crates reside in a `crates` directory,
	// so discard the prefix up to that segment to find the crate name
	let file = file.split_once("crates/");
	let target = file.as_ref().and_then(|(_, s)| s.split_once("/src/"));

	let module_path = target.map(|(krate, module)| {
		if module.starts_with(krate) {
			module.trim_end_matches(".rs").replace('/', "::")
		} else {
			krate.to_owned() + "::" + &module.trim_end_matches(".rs").replace('/', "::")
		}
	});
	let file = file.map(|(_, file)| format!("crates/{file}"));

	match level {
		Level::TRACE => {
			tracing::trace!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		Level::DEBUG => {
			tracing::debug!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		Level::INFO => {
			tracing::info!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		Level::WARN => {
			tracing::warn!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		Level::ERROR => {
			tracing::error!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
	}
}
