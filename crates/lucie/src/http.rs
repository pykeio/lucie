//! Types for HTTP requests, used for remote images

#[cfg(any(test, feature = "test-support"))]
use std::{any::type_name, fmt};
use std::{io::Cursor, pin::Pin, sync::Arc, task::Poll};

use bytes::Bytes;
use derive_more::Deref;
use futures_util::future::BoxFuture;
use http::HeaderValue;
pub use http::{self, Method, Request, Response, StatusCode, Uri, request::Builder};
use http_body::{Body, Frame};
use parking_lot::Mutex;
use tokio::io::{AsyncRead, ReadBuf};

/// Based on the implementation of AsyncBody in
/// <https://github.com/sagebind/isahc/blob/5c533f1ef4d6bdf1fd291b5103c22110f41d0bf0/src/body/mod.rs>.
pub struct AsyncBody(pub AsyncBodyInner);

/// Inner data contained in [`AsyncBody`]
pub enum AsyncBodyInner {
	/// An empty body.
	Empty,

	/// A body stored in memory.
	Bytes(std::io::Cursor<Bytes>),

	/// An asynchronous reader.
	AsyncReader(Pin<Box<dyn AsyncRead + Send + Sync>>)
}

impl AsyncBody {
	/// Create a new empty body.
	///
	/// An empty body represents the *absence* of a body, which is semantically
	/// different than the presence of a body of zero length.
	pub fn empty() -> Self {
		Self(AsyncBodyInner::Empty)
	}
	/// Create a streaming body that reads from the given reader.
	pub fn from_reader<R>(read: R) -> Self
	where
		R: AsyncRead + Send + Sync + 'static
	{
		Self(AsyncBodyInner::AsyncReader(Box::pin(read)))
	}

	/// Create a body from an array of bytes.
	pub fn from_bytes(bytes: Bytes) -> Self {
		Self(AsyncBodyInner::Bytes(Cursor::new(bytes)))
	}
}

impl Default for AsyncBody {
	fn default() -> Self {
		Self(AsyncBodyInner::Empty)
	}
}

impl From<()> for AsyncBody {
	fn from(_: ()) -> Self {
		Self(AsyncBodyInner::Empty)
	}
}

impl From<Bytes> for AsyncBody {
	fn from(bytes: Bytes) -> Self {
		Self::from_bytes(bytes)
	}
}

impl From<Vec<u8>> for AsyncBody {
	fn from(body: Vec<u8>) -> Self {
		Self::from_bytes(body.into())
	}
}

impl From<String> for AsyncBody {
	fn from(body: String) -> Self {
		Self::from_bytes(body.into())
	}
}

impl From<&'static [u8]> for AsyncBody {
	#[inline]
	fn from(s: &'static [u8]) -> Self {
		Self::from_bytes(Bytes::from_static(s))
	}
}

impl From<&'static str> for AsyncBody {
	#[inline]
	fn from(s: &'static str) -> Self {
		Self::from_bytes(Bytes::from_static(s.as_bytes()))
	}
}

impl<T: Into<Self>> From<Option<T>> for AsyncBody {
	fn from(body: Option<T>) -> Self {
		match body {
			Some(body) => body.into(),
			None => Self::empty()
		}
	}
}

impl AsyncRead for AsyncBody {
	fn poll_read(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
		// SAFETY: Standard Enum pin projection
		let inner = unsafe { &mut self.get_unchecked_mut().0 };
		match inner {
			AsyncBodyInner::Empty => Poll::Ready(Ok(())),
			// Blocking call is over an in-memory buffer
			AsyncBodyInner::Bytes(cursor) => {
				let pos = cursor.position();
				buf.put_slice(&cursor.get_ref()[pos as usize..pos as usize + buf.remaining()]);
				Poll::Ready(Ok(()))
			}
			AsyncBodyInner::AsyncReader(async_reader) => AsyncRead::poll_read(async_reader.as_mut(), cx, buf)
		}
	}
}

impl Body for AsyncBody {
	type Data = Bytes;
	type Error = std::io::Error;

	fn poll_frame(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let mut buffer = vec![0; 8192];
		let mut buffer = ReadBuf::new(&mut buffer);
		match AsyncRead::poll_read(self.as_mut(), cx, &mut buffer) {
			Poll::Ready(Ok(())) => {
				let filled = buffer.filled();
				if !filled.is_empty() {
					Poll::Ready(Some(Ok(Frame::data(Bytes::copy_from_slice(filled)))))
				} else {
					Poll::Ready(None)
				}
			}
			Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
			Poll::Pending => Poll::Pending
		}
	}
}

/// Policy for following redirects when making HTTP requests.
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedirectPolicy {
	/// Do not follow any redirects.
	#[default]
	NoFollow,
	/// Follow up to this amount of redirects before erroring.
	FollowLimit(u32),
	/// Follow unlimited redirects.
	FollowAll
}

/// Extensions for [`http::Request`].
pub trait HttpRequestExt {
	/// Conditionally modify self with the given closure.
	fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self
	where
		Self: Sized
	{
		if condition { then(self) } else { self }
	}

	/// Conditionally unwrap and modify self with the given closure, if the given option is Some.
	fn when_some<T>(self, option: Option<T>, then: impl FnOnce(Self, T) -> Self) -> Self
	where
		Self: Sized
	{
		match option {
			Some(value) => then(self, value),
			None => self
		}
	}

	/// Whether or not to follow redirects
	fn follow_redirects(self, follow: RedirectPolicy) -> Self;
}

impl HttpRequestExt for http::request::Builder {
	fn follow_redirects(self, follow: RedirectPolicy) -> Self {
		self.extension(follow)
	}
}

/// HTTP client trait, used for fetching remote images.
pub trait HttpClient: 'static + Send + Sync {
	/// Returns the user agent used by this client, if any.
	fn user_agent(&self) -> Option<&HeaderValue>;

	/// Returns the URI of the proxy used by this client, if any.
	fn proxy(&self) -> Option<&Uri>;

	/// Send a generic [`http::Request`].
	fn send(&self, req: http::Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>>;

	/// Send a `GET` request to the given URI.
	fn get(&self, uri: &str, body: AsyncBody, follow_redirects: bool) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		let request = Builder::new()
			.uri(uri)
			.follow_redirects(if follow_redirects { RedirectPolicy::FollowAll } else { RedirectPolicy::NoFollow })
			.body(body);

		match request {
			Ok(request) => self.send(request),
			Err(e) => Box::pin(async move { Err(e.into()) })
		}
	}

	/// Send a POST request to `uri` with a given `body`.
	fn post_json(&self, uri: &str, body: AsyncBody) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		let request = Builder::new()
			.uri(uri)
			.method(Method::POST)
			.header("Content-Type", "application/json")
			.body(body);

		match request {
			Ok(request) => self.send(request),
			Err(e) => Box::pin(async move { Err(e.into()) })
		}
	}

	/// Internal use
	#[cfg(any(test, feature = "test-support"))]
	#[allow(private_interfaces)]
	fn as_fake(&self) -> &FakeHttpClient {
		panic!("called as_fake on {}", type_name::<Self>())
	}
}

/// An [`HttpClient`] that may have a proxy.
#[derive(Deref)]
pub struct HttpClientWithProxy {
	#[deref]
	client: Arc<dyn HttpClient>,
	proxy: Option<Uri>
}

impl HttpClientWithProxy {
	/// Returns a new [`HttpClientWithProxy`] with the given proxy URL.
	pub fn new(client: Arc<dyn HttpClient>, proxy_url: Option<String>) -> Self {
		let proxy_url = proxy_url.and_then(|proxy| proxy.parse().ok()).or_else(read_proxy_from_env);

		Self::new_url(client, proxy_url)
	}

	/// Returns a new [`HttpClientWithProxy`] with the given proxy URL as a `Uri`.
	pub fn new_url(client: Arc<dyn HttpClient>, proxy_url: Option<Uri>) -> Self {
		Self { client, proxy: proxy_url }
	}
}

impl HttpClient for HttpClientWithProxy {
	fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		self.client.send(req)
	}

	fn user_agent(&self) -> Option<&HeaderValue> {
		self.client.user_agent()
	}

	fn proxy(&self) -> Option<&Uri> {
		self.proxy.as_ref()
	}

	#[cfg(any(test, feature = "test-support"))]
	#[allow(private_interfaces)]
	fn as_fake(&self) -> &FakeHttpClient {
		self.client.as_fake()
	}
}

/// An [`HttpClient`] that has a base URL.
#[derive(Deref)]
pub struct HttpClientWithUrl {
	base_url: Mutex<String>,
	#[deref]
	client: HttpClientWithProxy
}

impl HttpClientWithUrl {
	/// Returns a new [`HttpClientWithUrl`] with the given base URL.
	pub fn new(client: Arc<dyn HttpClient>, base_url: impl Into<String>, proxy_url: Option<String>) -> Self {
		let client = HttpClientWithProxy::new(client, proxy_url);

		Self {
			base_url: Mutex::new(base_url.into()),
			client
		}
	}

	/// Returns a new [`HttpClientWithUrl`] with the given base URL as a `Uri`.
	pub fn new_url(client: Arc<dyn HttpClient>, base_url: impl Into<String>, proxy_url: Option<Uri>) -> Self {
		let client = HttpClientWithProxy::new_url(client, proxy_url);

		Self {
			base_url: Mutex::new(base_url.into()),
			client
		}
	}

	/// Returns the base URL.
	pub fn base_url(&self) -> String {
		self.base_url.lock().clone()
	}

	/// Sets the base URL.
	pub fn set_base_url(&self, base_url: impl Into<String>) {
		let base_url = base_url.into();
		*self.base_url.lock() = base_url;
	}

	/// Builds a URL using the given path.
	pub fn build_url(&self, path: &str) -> String {
		format!("{}{}", self.base_url(), path)
	}
}

impl HttpClient for HttpClientWithUrl {
	fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		self.client.send(req)
	}

	fn user_agent(&self) -> Option<&HeaderValue> {
		self.client.user_agent()
	}

	fn proxy(&self) -> Option<&Uri> {
		self.client.proxy.as_ref()
	}

	#[cfg(any(test, feature = "test-support"))]
	#[allow(private_interfaces)]
	fn as_fake(&self) -> &FakeHttpClient {
		self.client.as_fake()
	}
}

fn read_proxy_from_env() -> Option<Uri> {
	const ENV_VARS: &[&str] = &["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"];

	ENV_VARS.iter().find_map(|var| std::env::var(var).ok()).and_then(|env| env.parse().ok())
}

/// An [`HttpClient`] that blocks all requests.
pub struct BlockedHttpClient;

impl BlockedHttpClient {
	/// Creates a new [`BlockedHttpClient`].
	pub fn new() -> Self {
		BlockedHttpClient
	}
}

impl HttpClient for BlockedHttpClient {
	fn send(&self, _req: Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		Box::pin(async { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "BlockedHttpClient disallowed request").into()) })
	}

	fn user_agent(&self) -> Option<&HeaderValue> {
		None
	}

	fn proxy(&self) -> Option<&Uri> {
		None
	}

	#[cfg(any(test, feature = "test-support"))]
	#[allow(private_interfaces)]
	fn as_fake(&self) -> &FakeHttpClient {
		panic!("called as_fake on {}", type_name::<Self>())
	}
}

#[cfg(any(test, feature = "test-support"))]
type FakeHttpHandler = Arc<dyn Fn(Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> + Send + Sync + 'static>;

#[cfg(any(test, feature = "test-support"))]
pub(crate) struct FakeHttpClient {
	handler: Mutex<Option<FakeHttpHandler>>,
	user_agent: HeaderValue
}

#[cfg(any(test, feature = "test-support"))]
impl FakeHttpClient {
	pub(crate) fn create<Fut, F>(handler: F) -> Arc<HttpClientWithUrl>
	where
		Fut: Future<Output = anyhow::Result<Response<AsyncBody>>> + Send + 'static,
		F: Fn(Request<AsyncBody>) -> Fut + Send + Sync + 'static
	{
		Arc::new(HttpClientWithUrl {
			base_url: Mutex::new("http://test.example".into()),
			client: HttpClientWithProxy {
				client: Arc::new(Self {
					handler: Mutex::new(Some(Arc::new(move |req| Box::pin(handler(req))))),
					user_agent: HeaderValue::from_static(type_name::<Self>())
				}),
				proxy: None
			}
		})
	}

	pub(crate) fn with_404_response() -> Arc<HttpClientWithUrl> {
		tracing::warn!("Using fake HTTP client with 404 response");
		Self::create(|_| async move { Ok(Response::builder().status(404).body(Default::default()).unwrap()) })
	}
}

#[cfg(any(test, feature = "test-support"))]
impl fmt::Debug for FakeHttpClient {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("FakeHttpClient").finish()
	}
}

#[cfg(any(test, feature = "test-support"))]
impl HttpClient for FakeHttpClient {
	fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
		((self.handler.lock().as_ref().unwrap())(req)) as _
	}

	fn user_agent(&self) -> Option<&HeaderValue> {
		Some(&self.user_agent)
	}

	fn proxy(&self) -> Option<&Uri> {
		None
	}

	#[allow(private_interfaces)]
	fn as_fake(&self) -> &FakeHttpClient {
		self
	}
}
