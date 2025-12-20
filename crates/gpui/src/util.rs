use anyhow::Result;

use std::{
    env,
    future::Future,
    ops::AddAssign,
    panic::Location,
    pin::Pin,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering::SeqCst},
    },
    task::{Context, Poll},
    time::Instant,
};

mod arc_cow;
pub mod sum_tree;

pub use self::arc_cow::ArcCow;

/// A helper trait for building complex objects with imperative conditionals in a fluent style.
pub trait FluentBuilder {
    /// Imperatively modify self with the given closure.
    fn map<U>(self, f: impl FnOnce(Self) -> U) -> U
    where
        Self: Sized,
    {
        f(self)
    }

    /// Conditionally modify self with the given closure.
    fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if condition { then(this) } else { this })
    }

    /// Conditionally modify self with the given closure.
    fn when_else(
        self,
        condition: bool,
        then: impl FnOnce(Self) -> Self,
        else_fn: impl FnOnce(Self) -> Self,
    ) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if condition { then(this) } else { else_fn(this) })
    }

    /// Conditionally unwrap and modify self with the given closure, if the given option is Some.
    fn when_some<T>(self, option: Option<T>, then: impl FnOnce(Self, T) -> Self) -> Self
    where
        Self: Sized,
    {
        self.map(|this| {
            if let Some(value) = option {
                then(this, value)
            } else {
                this
            }
        })
    }
    /// Conditionally unwrap and modify self with the given closure, if the given option is None.
    fn when_none<T>(self, option: &Option<T>, then: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if option.is_some() { this } else { then(this) })
    }
}

#[cfg(any(test, feature = "test-support"))]
/// Uses smol executor to run a given future no longer than the timeout specified.
/// Note that this won't "rewind" on `cx.executor().advance_clock` call, truly waiting for the timeout to elapse.
pub async fn smol_timeout<F, T>(timeout: std::time::Duration, f: F) -> Result<T, ()>
where
    F: Future<Output = T>,
{
    let timer = async {
        smol::Timer::after(timeout).await;
        Err(())
    };
    let future = async move { Ok(f.await) };
    smol::future::FutureExt::race(timer, future).await
}

/// Increment the given atomic counter if it is not zero.
/// Return the new value of the counter.
pub(crate) fn atomic_incr_if_not_zero(counter: &AtomicUsize) -> usize {
    let mut loaded = counter.load(SeqCst);
    loop {
        if loaded == 0 {
            return 0;
        }
        match counter.compare_exchange_weak(loaded, loaded + 1, SeqCst, SeqCst) {
            Ok(x) => return x + 1,
            Err(actual) => loaded = actual,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn file_url_to_path(url: &str) -> Option<std::path::PathBuf> {
    const FILE_SCHEME: &str = "file://";
    let url = percent_encoding::percent_decode_str(url)
        .decode_utf8()
        .ok()?;
    if !url.starts_with(FILE_SCHEME) {
        return None;
    }

    let path_str = &url[FILE_SCHEME.len()..];
    if !path_str.starts_with("/") {
        // has hostname, we're not doing all that
        return None;
    }

    std::path::Path::new(path_str).canonicalize().ok()
}

#[macro_export]
macro_rules! debug_panic {
    ( $($fmt_arg:tt)* ) => {
        if cfg!(debug_assertions) {
            panic!( $($fmt_arg)* );
        } else {
            let backtrace = std::backtrace::Backtrace::capture();
            log::error!("{}\n{:?}", format_args!($($fmt_arg)*), backtrace);
        }
    };
}

pub fn post_inc<T: From<u8> + AddAssign<T> + Copy>(value: &mut T) -> T {
    let prev = *value;
    *value += T::from(1);
    prev
}

pub fn measure<R>(label: &str, f: impl FnOnce() -> R) -> R {
    static ZED_MEASUREMENTS: OnceLock<bool> = OnceLock::new();
    let zed_measurements = ZED_MEASUREMENTS.get_or_init(|| {
        env::var("ZED_MEASUREMENTS")
            .map(|measurements| measurements == "1" || measurements == "true")
            .unwrap_or(false)
    });

    if *zed_measurements {
        let start = Instant::now();
        let result = f();
        let elapsed = start.elapsed();
        eprintln!("{}: {:?}", label, elapsed);
        result
    } else {
        f()
    }
}

pub trait ResultExt<E> {
    type Ok;

    fn log_err(self) -> Option<Self::Ok>;
    fn log_with_level(self, level: log::Level) -> Option<Self::Ok>;
}

impl<T, E> ResultExt<E> for Result<T, E>
where
    E: std::fmt::Debug,
{
    type Ok = T;

    #[track_caller]
    fn log_err(self) -> Option<T> {
        self.log_with_level(log::Level::Error)
    }

    #[track_caller]
    fn log_with_level(self, level: log::Level) -> Option<T> {
        match self {
            Ok(value) => Some(value),
            Err(error) => {
                log_error_with_caller(*Location::caller(), error, level);
                None
            }
        }
    }
}

fn log_error_with_caller<E>(caller: core::panic::Location<'_>, error: E, level: log::Level)
where
    E: std::fmt::Debug,
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
    log::logger().log(
        &log::Record::builder()
            .target(module_path.as_deref().unwrap_or(""))
            .module_path(file.as_deref())
            .args(format_args!("{:?}", error))
            .file(Some(caller.file()))
            .line(Some(caller.line()))
            .level(level)
            .build(),
    );
}

pub trait TryFutureExt {
    fn log_tracked_err(self, location: core::panic::Location<'static>) -> LogErrorFuture<Self>
    where
        Self: Sized;
}

impl<F, T, E> TryFutureExt for F
where
    F: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    fn log_tracked_err(self, location: core::panic::Location<'static>) -> LogErrorFuture<Self>
    where
        Self: Sized,
    {
        LogErrorFuture(self, log::Level::Error, location)
    }
}

#[must_use]
pub struct LogErrorFuture<F>(F, log::Level, core::panic::Location<'static>);

impl<F, T, E> Future for LogErrorFuture<F>
where
    F: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let level = self.1;
        let location = self.2;
        let inner = unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().0) };
        match inner.poll(cx) {
            Poll::Ready(output) => Poll::Ready(match output {
                Ok(output) => Some(output),
                Err(error) => {
                    log_error_with_caller(location, error, level);
                    None
                }
            }),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct Deferred<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Deferred<F> {
    /// Drop without running the deferred function.
    pub fn abort(mut self) {
        self.0.take();
    }
}

impl<F: FnOnce()> Drop for Deferred<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}

/// Run the given function when the returned value is dropped (unless it's cancelled).
#[must_use]
pub fn defer<F: FnOnce()>(f: F) -> Deferred<F> {
    Deferred(Some(f))
}

/// Expands to an immediately-invoked function expression. Good for using the ? operator
/// in functions which do not return an Option or Result.
///
/// Accepts a normal block, an async block, or an async move block.
#[macro_export]
macro_rules! maybe {
    ($block:block) => {
        (|| $block)()
    };
    (async $block:block) => {
        (async || $block)()
    };
    (async move $block:block) => {
        (async move || $block)()
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_iife() {
        fn option_returning_function() -> Option<()> {
            None
        }

        let foo = maybe!({
            option_returning_function()?;
            Some(())
        });

        assert_eq!(foo, None);
    }
}
