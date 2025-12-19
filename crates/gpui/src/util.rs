use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

pub use util::*;

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
