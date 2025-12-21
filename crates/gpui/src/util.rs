#[cfg(any(test, feature = "test-support"))]
/// Uses smol executor to run a given future no longer than the timeout specified.
/// Note that this won't "rewind" on `cx.executor().advance_clock` call, truly waiting for the timeout to elapse.
pub async fn smol_timeout<F, T>(timeout: std::time::Duration, f: F) -> Result<T, ()>
where
	F: Future<Output = T>
{
	let timer = async {
		smol::Timer::after(timeout).await;
		Err(())
	};
	let future = async move { Ok(f.await) };
	smol::future::FutureExt::race(timer, future).await
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn file_url_to_path(url: &str) -> Option<std::path::PathBuf> {
	const FILE_SCHEME: &str = "file://";
	let url = percent_encoding::percent_decode_str(url).decode_utf8().ok()?;
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
