//! How requests reach GitHub, and how background work reports back to the UI.
//!
//! - Desktop: blocking HTTPS with `ureq` straight to the API (token from `.env`), run on a worker
//!   thread and driven by `pollster`.
//! - Web: `fetch` to the same-origin `/gh` proxy of `repohub-server`, which adds the token, so the
//!   token never reaches the page. Driven by `wasm_bindgen_futures`.
use std::future::Future;

use rune::AppHandle;

use crate::App;
use crate::config::Config;

/// One HTTP response, with the rate-limit headers the error messages use.
pub struct Resp {
    pub status: u16,
    pub body: String,
    pub remaining: Option<String>,
    pub reset: Option<i64>,
    pub retry_after: Option<u64>,
    pub link: Option<String>,
    /// The answer's `ETag`, to ask again with `If-None-Match` (see `etag`).
    pub etag: Option<String>,
}

/// Runs `work` in the background and hands its result to `done` on the UI thread.
pub fn spawn<R, Fut>(h: AppHandle<App>, work: impl FnOnce() -> Fut + Send + 'static, done: impl FnOnce(&mut App, R) + Send + 'static)
where
    R: Send + 'static,
    Fut: Future<Output = R> + 'static,
{
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let r = pollster::block_on(work());
        h.update(move |s: &mut App, _| done(s, r));
    });
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        let r = work().await;
        h.update(move |s: &mut App, _| done(s, r));
    });
}

/// Where background work reports: the app's handle, or, for the first load started before the
/// window exists (and so before there is a handle), a [`Mailbox`] that keeps what arrives until
/// the handle does.
#[derive(Clone)]
pub enum To {
    Now(AppHandle<App>),
    Later(Mailbox),
}

impl To {
    pub fn post(&self, f: impl FnOnce(&mut App) + Send + 'static) {
        match self {
            To::Now(h) => h.update(move |s: &mut App, _| f(s)),
            To::Later(m) => m.post(f),
        }
    }
}

type Posted = Box<dyn FnOnce(&mut App) + Send>;

/// The handle once it exists, and what arrived before it.
type Waiting = (Option<AppHandle<App>>, Vec<Posted>);

/// Keeps what background work sends until the app's handle exists, then hands it over in order.
#[derive(Clone, Default)]
pub struct Mailbox(std::sync::Arc<std::sync::Mutex<Waiting>>);

impl Mailbox {
    pub fn post(&self, f: impl FnOnce(&mut App) + Send + 'static) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &g.0 {
            Some(h) => h.update(move |s: &mut App, _| f(s)),
            None => g.1.push(Box::new(f)),
        }
    }

    /// From now on, and for what's already here, through `h`.
    pub fn attach(&self, h: AppHandle<App>) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for f in g.1.drain(..) {
            h.update(move |s: &mut App, _| f(s));
        }
        g.0 = Some(h);
    }
}

/// Runs `work` in the background and hands its result to `done` through `to`.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_to<R, Fut>(to: To, work: impl FnOnce() -> Fut + Send + 'static, done: impl FnOnce(&mut App, R) + Send + 'static)
where
    R: Send + 'static,
    Fut: Future<Output = R> + 'static,
{
    match to {
        To::Now(h) => spawn(h, work, done),
        To::Later(m) => {
            std::thread::spawn(move || {
                let r = pollster::block_on(work());
                m.post(move |s: &mut App| done(s, r));
            });
        }
    }
}

/// Waits `ms` milliseconds without blocking the UI (call it inside `spawn`).
pub async fn sleep(ms: u32) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Ok(f) = js_sys::Reflect::get(&js_sys::global(), &"setTimeout".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) {
                let _ = f.call2(&js_sys::global(), &resolve, &wasm_bindgen::JsValue::from(ms));
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// `Send` on the desktop, where the parts of a load run on threads; nothing on the web, where
/// they run on the page's one thread.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// `Sync` on the desktop, nothing on the web (see [`MaybeSend`]).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}

/// Runs `a` and `b` at the same time and returns both answers. Desktop: `a` on a scoped thread
/// over the blocking client, `b` on this one. Web: both futures polled together.
pub async fn join<A, B>(a: A, b: B) -> (A::Output, B::Output)
where
    A: Future + MaybeSend,
    B: Future,
    A::Output: MaybeSend,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::scope(|sc| {
            let other = sc.spawn(move || pollster::block_on(a));
            let rb = pollster::block_on(b);
            (other.join().unwrap_or_else(|p| std::panic::resume_unwind(p)), rb)
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::task::Poll;
        let (mut a, mut b) = (Box::pin(a), Box::pin(b));
        let (mut ra, mut rb) = (None, None);
        std::future::poll_fn(move |cx| {
            if ra.is_none() && let Poll::Ready(v) = a.as_mut().poll(cx) {
                ra = Some(v);
            }
            if rb.is_none() && let Poll::Ready(v) = b.as_mut().poll(cx) {
                rb = Some(v);
            }
            match (ra.take(), rb.take()) {
                (Some(x), Some(y)) => Poll::Ready((x, y)),
                (x, y) => {
                    (ra, rb) = (x, y);
                    Poll::Pending
                }
            }
        }).await
    }
}

/// Runs every future of `all` at the same time; the answers come back in `all`'s order.
pub async fn join_all<F>(all: Vec<F>) -> Vec<F::Output>
where
    F: Future + MaybeSend,
    F::Output: MaybeSend,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::scope(|sc| {
            let running: Vec<_> = all.into_iter().map(|f| sc.spawn(move || pollster::block_on(f))).collect();
            running.into_iter().map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::task::Poll;
        let mut futs: Vec<_> = all.into_iter().map(Box::pin).collect();
        let mut out: Vec<Option<F::Output>> = futs.iter().map(|_| None).collect();
        std::future::poll_fn(move |cx| {
            for (f, o) in futs.iter_mut().zip(out.iter_mut()) {
                if o.is_none() && let Poll::Ready(v) = f.as_mut().poll(cx) {
                    *o = Some(v);
                }
            }
            if out.iter().all(Option::is_some) { Poll::Ready(out.iter_mut().filter_map(Option::take).collect()) } else { Poll::Pending }
        }).await
    }
}

/// At most `n` requests at once (GitHub asks clients not to send many at the same time). A
/// request takes a place with [`Gate::enter`] and gives it back when the returned guard drops.
pub struct Gate {
    #[cfg(not(target_arch = "wasm32"))]
    free: std::sync::Mutex<usize>,
    #[cfg(not(target_arch = "wasm32"))]
    freed: std::sync::Condvar,
    #[cfg(target_arch = "wasm32")]
    free: std::cell::Cell<usize>,
    #[cfg(target_arch = "wasm32")]
    waiting: std::cell::RefCell<std::collections::VecDeque<std::task::Waker>>,
}

/// A place taken in a [`Gate`]; dropping it lets the next request in.
pub struct Entered<'a>(&'a Gate);

impl Gate {
    pub fn new(n: usize) -> Gate {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Gate { free: std::sync::Mutex::new(n.max(1)), freed: std::sync::Condvar::new() }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Gate { free: std::cell::Cell::new(n.max(1)), waiting: Default::default() }
        }
    }

    /// Waits for a free place (desktop: blocks this worker thread; web: yields).
    pub async fn enter(&self) -> Entered<'_> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
            while *free == 0 {
                free = self.freed.wait(free).unwrap_or_else(|e| e.into_inner());
            }
            *free -= 1;
        }
        #[cfg(target_arch = "wasm32")]
        std::future::poll_fn(|cx| {
            if self.free.get() > 0 {
                self.free.set(self.free.get() - 1);
                std::task::Poll::Ready(())
            } else {
                self.waiting.borrow_mut().push_back(cx.waker().clone());
                std::task::Poll::Pending
            }
        }).await;
        Entered(self)
    }
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            *self.0.free.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            self.0.freed.notify_one();
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.0.free.set(self.0.free.get() + 1);
            if let Some(w) = self.0.waiting.borrow_mut().pop_front() {
                w.wake();
            }
        }
    }
}

// ------------------------------------------------------------------ desktop

#[cfg(not(target_arch = "wasm32"))]
pub struct Transport {
    agent: ureq::Agent,
    api: String,
    token: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn shared_agent(use_proxy: bool) -> ureq::Agent {
    use std::sync::OnceLock;
    static DIRECT: OnceLock<ureq::Agent> = OnceLock::new();
    static PROXIED: OnceLock<ureq::Agent> = OnceLock::new();
    let slot = if use_proxy { &PROXIED } else { &DIRECT };
    slot.get_or_init(|| {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(use_proxy)
            .max_idle_connections_per_host(8)
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("RepoHub (Rune)")
            .build()
    })
    .clone()
}

#[cfg(not(target_arch = "wasm32"))]
impl Transport {
    pub fn new(cfg: &Config) -> Transport {
        let agent = shared_agent(!bypass_proxy(&cfg.api));
        Transport { agent, api: cfg.api.clone(), token: cfg.token.clone() }
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub async fn send(&self, method: &str, path: &str, accept: &str) -> Result<Resp, String> {
        self.send_body(method, path, accept, "").await
    }

    /// A GET that GitHub may answer with 304 when `etag` still names its answer.
    pub async fn send_if(&self, path: &str, accept: &str, etag: Option<&str>) -> Result<Resp, String> {
        self.request("GET", path, accept, "", etag)
    }

    /// `send` with a JSON body (empty: none).
    pub async fn send_body(&self, method: &str, path: &str, accept: &str, body: &str) -> Result<Resp, String> {
        self.request(method, path, accept, body, None)
    }

    fn request(&self, method: &str, path: &str, accept: &str, body: &str, etag: Option<&str>) -> Result<Resp, String> {
        let mut r = self.agent.request(method, &format!("{}{}", self.api, path))
            .set("Accept", accept)
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &self.token {
            r = r.set("Authorization", &format!("Bearer {t}"));
        }
        if let Some(tag) = etag {
            r = r.set("If-None-Match", tag);
        }
        let res = if !body.is_empty() {
            r.set("Content-Type", "application/json").send_string(body)
        } else if method == "PUT" {
            r.set("Content-Length", "0").send_string("")
        } else {
            r.call()
        };
        let resp = match res {
            Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
            Err(e) => return Err(format!("Can't reach {}: {e}", self.api)),
        };
        let status = resp.status();
        let remaining = resp.header("x-ratelimit-remaining").map(str::to_owned);
        let reset = resp.header("x-ratelimit-reset").and_then(|v| v.parse().ok());
        let retry_after = resp.header("retry-after").and_then(|v| v.trim().parse().ok());
        let link = resp.header("link").map(str::to_owned);
        let etag = resp.header("etag").map(str::to_owned);
        // The client reads at most 10 MB of an answer (a very long job log can be more).
        let body = resp.into_string().map_err(|e| match e.kind() {
            std::io::ErrorKind::Other if e.to_string().contains("too big") => "GitHub's answer is larger than RepoHub reads (10 MB): open it on GitHub instead.".to_owned(),
            _ => format!("GitHub's answer couldn't be read: {e}"),
        })?;
        Ok(Resp { status, body, remaining, reset, retry_after, link, etag })
    }
}

/// True when NO_PROXY (or a loopback host) says to reach `api` directly. The HTTP client reads
/// HTTPS_PROXY by itself but not NO_PROXY.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn bypass_proxy(api: &str) -> bool {
    let host = api.split("://").nth(1).unwrap_or(api).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    if host == "localhost" || host.starts_with("127.") || host == "[::1]" {
        return true;
    }
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty())
        .any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ------------------------------------------------------------------ web

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "
// The Android app has no server: its Kotlin side answers the same requests (android/).
let nativeSeq = 0;
const nativeWaiting = new Map();
globalThis.__repohubNative = (id, reply) => {
    const done = nativeWaiting.get(id);
    nativeWaiting.delete(id);
    if (done) done(reply);
};
export function native_exit() { if (globalThis.RepoHubNative) globalThis.RepoHubNative.exit(); }
export async function gh_fetch(method, url, accept, body, etag) {
    if (globalThis.RepoHubNative) {
        return await new Promise((done) => {
            const id = ++nativeSeq;
            nativeWaiting.set(id, done);
            globalThis.RepoHubNative.request(id, method, url, accept, body || '', etag || '');
        });
    }
    try {
        const init = { method, headers: { Accept: accept, 'X-RepoHub': '1' }, credentials: 'same-origin', cache: 'no-store' };
        if (method !== 'GET' && method !== 'HEAD') {
            init.headers['Content-Type'] = 'application/json';
        }
        // Set by hand, so a 304 comes back to the app (which keeps the answer) instead of
        // being filled in from the browser's cache.
        if (etag) { init.headers['If-None-Match'] = etag; }
        if (body) { init.body = body; }
        const r = await fetch(url, init);
        const text = await r.text();
        return [r.status, text, r.headers.get('x-ratelimit-remaining') || '', r.headers.get('x-ratelimit-reset') || '', r.headers.get('retry-after') || '', r.headers.get('link') || '', r.headers.get('etag') || ''];
    } catch (e) {
        return [0, String(e), '', '', '', '', ''];
    }
}
export function now_secs() { return Math.floor(Date.now() / 1000); }
export function absolute_url(rel) { return new URL(rel, location.href).href; }
")]
    extern "C" {
        pub async fn gh_fetch(method: &str, url: &str, accept: &str, body: &str, etag: &str) -> JsValue;
        pub fn now_secs() -> f64;
        pub fn absolute_url(rel: &str) -> String;
        pub fn native_exit();
    }
}

#[cfg(target_arch = "wasm32")]
pub struct Transport {
    has_token: bool,
}

#[cfg(target_arch = "wasm32")]
impl Transport {
    pub fn new(cfg: &Config) -> Transport {
        Transport { has_token: cfg.token.is_some() }
    }

    pub fn has_token(&self) -> bool {
        self.has_token
    }

    pub async fn send(&self, method: &str, path: &str, accept: &str) -> Result<Resp, String> {
        request(method, &format!("gh{path}"), accept, "").await
    }

    /// A GET that GitHub may answer with 304 when `etag` still names its answer.
    pub async fn send_if(&self, path: &str, accept: &str, etag: Option<&str>) -> Result<Resp, String> {
        request_if("GET", &format!("gh{path}"), accept, "", etag.unwrap_or("")).await
    }

    pub async fn send_body(&self, method: &str, path: &str, accept: &str, body: &str) -> Result<Resp, String> {
        request(method, &format!("gh{path}"), accept, body).await
    }
}

/// Web only: a request to `url` relative to the page (the RepoHub server).
#[cfg(target_arch = "wasm32")]
pub async fn request(method: &str, url: &str, accept: &str, body: &str) -> Result<Resp, String> {
    request_if(method, url, accept, body, "").await
}

#[cfg(target_arch = "wasm32")]
async fn request_if(method: &str, url: &str, accept: &str, body: &str, etag: &str) -> Result<Resp, String> {
    let v = js::gh_fetch(method, url, accept, body, etag).await;
    let a = js_sys::Array::from(&v);
    let status = a.get(0).as_f64().unwrap_or(0.0) as u16;
    let body = a.get(1).as_string().unwrap_or_default();
    if status == 0 {
        return Err(format!("Can't reach the RepoHub server: {body}"));
    }
    let remaining = a.get(2).as_string().filter(|s| !s.is_empty());
    let reset = a.get(3).as_string().and_then(|s| s.parse().ok());
    let retry_after = a.get(4).as_string().and_then(|s| s.trim().parse().ok());
    let link = a.get(5).as_string().filter(|s| !s.is_empty());
    let etag = a.get(6).as_string().filter(|s| !s.is_empty());
    Ok(Resp { status, body, remaining, reset, retry_after, link, etag })
}

/// The Android app: leaves the app (the back button, with nowhere left to go back to).
#[cfg(target_arch = "wasm32")]
pub fn leave_app() {
    js::native_exit();
}

/// Web only: `rel` resolved against the page's address.
#[cfg(target_arch = "wasm32")]
pub fn absolute(rel: &str) -> String {
    js::absolute_url(rel)
}

#[cfg(target_arch = "wasm32")]
pub fn now() -> i64 {
    js::now_secs() as i64
}

/// Web only: reads `config.json` from `repohub-server`. Absent (a static host such as GitHub
/// Pages) means the sample data.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_config() -> Option<Config> {
    let v = js::gh_fetch("GET", "config.json", "application/json", "", "").await;
    let a = js_sys::Array::from(&v);
    if a.get(0).as_f64() != Some(200.0) {
        return None;
    }
    let body = a.get(1).as_string()?;
    Config::from_json(&body)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// An answer larger than the client reads (a long job log) is said in words, not as the
    /// HTTP client's own error.
    #[test]
    fn an_answer_too_large_to_read_is_said_plainly() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                let _ = stream.read(&mut [0u8; 2048]);
                let body = vec![b'x'; 11 << 20];
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(&body);
            }
        });
        let mut cfg = crate::config::Config::from_json("{}").unwrap();
        cfg.api = api;
        cfg.token = Some("t".into());
        let e = pollster::block_on(crate::github::load_job_log(&cfg, "octo/app", 7)).unwrap_err();
        assert!(e.contains("larger than RepoHub reads") && !e.contains("into_string"), "{e}");
    }
}
