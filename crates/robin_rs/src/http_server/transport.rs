//! Application-owned transport lifecycle: native listener start/stop, the
//! browser bridge binding, and the listener-side relay to the game tick.

use super::*;

#[cfg(any(test, feature = "script-rpc", target_arch = "wasm32"))]
pub(super) async fn resolve_deferred_reply(reply: Reply) -> Reply {
    match reply? {
        ReplyBody::ReplayExport(result) => result
            .recv()
            .await
            .map_err(|error| {
                RpcError::internal(format!("replay export worker dropped its result: {error}"))
            })?
            .map(|content| ReplyBody::Json(serde_json::json!({ "content": content })))
            .map_err(|error| match error {
                crate::replay_service::ExportError::Capacity(message) => {
                    RpcError::capacity(message)
                }
                crate::replay_service::ExportError::Retired(message) => RpcError::retired(message),
                crate::replay_service::ExportError::Unavailable(message) => {
                    RpcError::unavailable_capability(message)
                }
                crate::replay_service::ExportError::Internal(message) => {
                    RpcError::internal(message)
                }
            }),
        body => Ok(body),
    }
}

pub(super) type Queue = Arc<Mutex<RequestRouter>>;

pub(super) struct HttpServer {
    pub(super) replay_exports: crate::replay_service::ReplayExports,
    pub(super) replay_launches: crate::replay_service::ReplayLaunches,
    pub(super) queue: Queue,
    #[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
    pub(super) bind_addr: std::net::SocketAddr,
    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    pub(super) listener: Option<thread::JoinHandle<()>>,
    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    pub(super) stop: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Application-owned transport. Diagnostics cannot recreate a listener.
/// Stopping retires the queue and cancels and joins every native connection,
/// including stalled bodies and responses. Dropping performs the same teardown.
#[derive(Default, serde::Serialize)]
pub struct HttpTransport {
    pub(super) port: Option<u16>,
    #[serde(skip)]
    pub(super) server: Option<HttpServer>,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("port", &self.port)
            .finish()
    }
}

robin_util::deny_deserialize!(HttpTransport, "live HTTP transport cannot be deserialized");

#[cfg(target_arch = "wasm32")]
thread_local! {
    // JS needs one current entry point, not ownership of an application service.
    pub(super) static BROWSER_QUEUE: std::cell::RefCell<std::sync::Weak<Mutex<RequestRouter>>> = const { std::cell::RefCell::new(std::sync::Weak::new()) };
}

impl HttpTransport {
    pub fn is_started(&self) -> bool {
        self.port.is_some()
    }

    pub fn matches_replay(
        &self,
        exports: &crate::replay_service::ReplayExports,
        launches: &crate::replay_service::ReplayLaunches,
    ) -> bool {
        self.server.as_ref().is_none_or(|server| {
            server.replay_exports.same_service(exports)
                && server.replay_launches.same_service(launches)
        })
    }

    pub fn attach(&self) -> SessionIngress {
        SessionIngress::attach(self.server.as_ref())
    }

    pub fn drain_pre_engine(&self) {
        if let Some(server) = &self.server {
            drain_pre_engine(server);
        }
    }

    pub fn stop(&mut self) {
        self.port = None;
        if let Some(server) = self.server.take() {
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            let mut server = server;
            #[cfg(target_arch = "wasm32")]
            BROWSER_QUEUE.with(|binding| {
                let mut binding = binding.borrow_mut();
                if binding.ptr_eq(&Arc::downgrade(&server.queue)) {
                    *binding = std::sync::Weak::new();
                }
            });
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if let Some(stop) = server.stop.take() {
                let _ = stop.send(());
            }
            server.queue.lock().expect("RPC router poisoned").retire();
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if let Some(listener) = server.listener.take() {
                if listener.join().is_err() {
                    tracing::error!("script HTTP listener panicked during shutdown");
                }
            }
        }
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start this application's script-RPC listener. Repeated starts must match
/// both the binding and replay capabilities; stop explicitly before rebinding.
///
/// Native: binds a loopback HTTP listener on `port` (0 disables).
/// Wasm: ignores `port`; just installs the empty queue so `rh_rpc`
/// has somewhere to push.
impl HttpTransport {
    pub fn start(
        &mut self,
        port: u16,
        replay_exports: crate::replay_service::ReplayExports,
        replay_launches: crate::replay_service::ReplayLaunches,
    ) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        let port = {
            let _ = port;
            0
        };
        if let Some(bound_port) = self.port {
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if self
                .server
                .as_ref()
                .and_then(|server| server.listener.as_ref())
                .is_some_and(thread::JoinHandle::is_finished)
            {
                return Err("HTTP listener has exited; stop it before restarting".into());
            }
            return if bound_port == port && self.matches_replay(&replay_exports, &replay_launches) {
                Ok(())
            } else {
                Err(
                    "HTTP transport already initialized with a different port or replay authority"
                        .into(),
                )
            };
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = port;
            let queue = Arc::new(Mutex::new(RequestRouter::default()));
            BROWSER_QUEUE.with(|binding| {
                let mut binding = binding.borrow_mut();
                if binding.upgrade().is_some() {
                    return Err("another application owns the browser RPC bridge".to_owned());
                }
                *binding = Arc::downgrade(&queue);
                Ok(())
            })?;
            self.server = Some(HttpServer {
                replay_exports,
                replay_launches,
                queue,
            });
            self.port = Some(port);
            tracing::info!("script RPC: wasm bridge ready (rh_rpc)");
            Ok(())
        }
        #[cfg(all(not(feature = "script-rpc"), not(target_arch = "wasm32")))]
        {
            let _ = (replay_exports, replay_launches);
            if port != 0 {
                return Err("native HTTP transport requires the script-rpc feature".into());
            }
            self.port = Some(0);
            Ok(())
        }
        #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
        {
            if port == 0 {
                self.port = Some(port);
                tracing::info!("script HTTP server: disabled (--http-server 0)");
                return Ok(());
            }
            let server = start(port, replay_exports, replay_launches)?;
            self.server = Some(server);
            self.port = Some(port);
            Ok(())
        }
    }
}

#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
pub(super) fn start(
    port: u16,
    replay_exports: crate::replay_service::ReplayExports,
    replay_launches: crate::replay_service::ReplayLaunches,
) -> Result<HttpServer, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("HTTP runtime: {e}"))?;
    let listener = {
        let _entered = runtime.enter();
        let socket = tokio::net::TcpSocket::new_v4().map_err(|e| format!("HTTP socket: {e}"))?;
        // Reuse a stopped listener's address after accepted connections enter
        // TIME_WAIT. Do not enable Windows SO_REUSEADDR's port-sharing semantics.
        #[cfg(unix)]
        socket
            .set_reuseaddr(true)
            .map_err(|e| format!("HTTP socket reuse: {e}"))?;
        socket.bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).map_err(|e| {
            format!("script HTTP server failed to bind 127.0.0.1:{port}: {e} (another robin instance? pass `--http-server 0` to disable, or `--http-server <port>` to pick a different port)")
        })?;
        socket
            .listen(128)
            .map_err(|e| format!("HTTP listen: {e}"))?
    };
    let bind_addr = listener
        .local_addr()
        .map_err(|e| format!("HTTP listener address: {e}"))?;
    tracing::info!("script HTTP server listening on http://{bind_addr}");

    let queue: Queue = Arc::new(Mutex::new(RequestRouter::default()));
    let queue_for_thread = queue.clone();
    let (stop, listener_stop) = tokio::sync::oneshot::channel();
    let listener = thread::Builder::new()
        .name("robin-http-server".into())
        .spawn(move || {
            runtime.block_on(native_transport::run(
                listener,
                queue_for_thread,
                listener_stop,
                bind_addr.port(),
            ))
        })
        .map_err(|e| format!("script HTTP server: failed to spawn listener thread: {e}"))?;
    Ok(HttpServer {
        queue,
        #[cfg(test)]
        bind_addr,
        replay_exports,
        replay_launches,
        listener: Some(listener),
        stop: Some(stop),
    })
}

/// Reject requests a browser could have issued cross-origin.
///
/// The POST endpoints are unauthenticated and reachable via
/// no-preflight `POST` from any web page, so a malicious page could
/// drive the running game (CSRF). Non-browser clients (curl, scripts)
/// never send `Origin` / `Sec-Fetch-Site` and send an exact local
/// `Host`, so they pass untouched. Returns a rejection reason, or
/// `None` when the request is acceptable.
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
pub(super) fn browser_rejection_reason(
    origin: Option<&str>,
    sec_fetch_site: Option<&str>,
    host: Option<&str>,
    port: u16,
) -> Option<&'static str> {
    // Browsers attach `Origin` to every cross-origin (and most
    // same-origin) fetches; no legitimate client of this API is a web
    // page, so any `Origin` at all is rejected.
    if origin.is_some() {
        return Some("cross-origin requests are not allowed (Origin header present)");
    }
    match sec_fetch_site {
        None | Some("none") | Some("same-origin") => {}
        Some(_) => return Some("cross-site request blocked (Sec-Fetch-Site)"),
    }
    // Anti DNS-rebinding: the Host header must name this loopback server.
    let host_ok = host.is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
    });
    if !host_ok {
        return Some("request rejected: Host header does not match the local server");
    }
    None
}

/// Send a payload to the game loop and wait for the reply.  Caps the
/// wait at 60 s so a wedged game doesn't hang the client forever.
#[cfg(all(any(feature = "script-rpc", test), not(target_arch = "wasm32")))]
pub(super) async fn relay(queue: &Queue, payload: HttpPayload) -> (u16, ReplyBody) {
    let (response_tx, rx) = Responder::channel();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let retirement = queue
        .lock()
        .expect("RPC router poisoned")
        .retirement_receiver();
    queue
        .lock()
        .expect("queue mutex poisoned")
        .push_back(HttpRequest {
            payload,
            response_tx: response_tx.with_router(queue).with_deadline(deadline),
        });
    let reply = tokio::select! {
        biased;
        _ = retirement.recv() => return (400, RpcError::retired("HTTP transport stopped").wire_body().into()),
        reply = async {
            match rx.recv().await {
                Ok(reply) => Ok(resolve_deferred_reply(reply).await),
                Err(error) => Err(error),
            }
        } => reply,
        _ = tokio::time::sleep_until(deadline.into()) => {
            rx.expire();
            return (504, RpcError::deadline("game loop did not process the request within 60s; already-admitted work may complete").wire_body().into());
        },
    };
    match reply {
        Ok(Ok(body)) => (200, body),
        Ok(Err(error)) => (400, error.wire_body().into()),
        Err(_) if rx.is_expired() => (
            504,
            RpcError::deadline("request expired before admission")
                .wire_body()
                .into(),
        ),
        Err(_) => (
            500,
            RpcError::internal("game loop dropped the response channel")
                .wire_body()
                .into(),
        ),
    }
}
