use std::{fmt, panic::Location};

pub use tracing::Level as LogLevel;

pub trait ResultExt<E> {
	type Ok;

	fn log_err(self) -> Option<Self::Ok>;
	fn log_with_level(self, level: LogLevel) -> Option<Self::Ok>;
}

impl<T, E> ResultExt<E> for Result<T, E>
where
	E: std::fmt::Debug
{
	type Ok = T;

	#[track_caller]
	fn log_err(self) -> Option<T> {
		self.log_with_level(LogLevel::ERROR)
	}

	#[track_caller]
	fn log_with_level(self, level: LogLevel) -> Option<T> {
		match self {
			Ok(value) => Some(value),
			Err(error) => {
				log_error_with_caller(*Location::caller(), error, level);
				None
			}
		}
	}
}

fn log_error_with_caller<E>(caller: Location<'static>, error: E, level: LogLevel)
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
		LogLevel::TRACE => {
			tracing::trace!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		LogLevel::DEBUG => {
			tracing::debug!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		LogLevel::INFO => {
			tracing::info!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		LogLevel::WARN => {
			tracing::warn!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
		LogLevel::ERROR => {
			tracing::error!(file = caller.file(), line = caller.line(), path = file.as_deref(), target = module_path.as_deref().unwrap_or(""), "{:?}", error)
		}
	}
}
