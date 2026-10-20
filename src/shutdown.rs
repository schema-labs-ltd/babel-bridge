//! Signal handling and the ordered, bounded process shutdown.
//!
//! Railway delivers SIGTERM and then kills the container. The first signal is
//! logged by name. Open HTTP and MCP requests get a short chance to finish,
//! then the WhatsApp bridge and SQLite are closed. A second signal or the hard
//! backstop exits even if a step is stuck.

use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Budgets for one shutdown. The step limits add up to less than `hard`, and
/// the signal listener's backstop is longer again so a stuck step reports its
/// own reason before the process is forced out.
pub const REQUEST_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
pub const BRIDGE_STOP_TIMEOUT: Duration = Duration::from_secs(3);
/// Outer bound around the bridge stop. It is longer than [`BRIDGE_STOP_TIMEOUT`]
/// so the child is killed and reaped before this step is cancelled.
pub const BRIDGE_STEP_TIMEOUT: Duration = Duration::from_secs(4);
pub const SQLITE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
pub const HARD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(12);
pub const HARD_SHUTDOWN_BACKSTOP: Duration = Duration::from_secs(14);

#[derive(Debug, Clone, Copy)]
pub struct ShutdownBudgets {
    pub request_drain: Duration,
    pub bridge: Duration,
    pub sqlite: Duration,
    pub hard: Duration,
}

impl ShutdownBudgets {
    pub const fn production() -> Self {
        Self {
            request_drain: REQUEST_DRAIN_TIMEOUT,
            bridge: BRIDGE_STEP_TIMEOUT,
            sqlite: SQLITE_CLOSE_TIMEOUT,
            hard: HARD_SHUTDOWN_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownSignal {
    Sigterm,
    Sigint,
}

impl ShutdownSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sigterm => "SIGTERM",
            Self::Sigint => "SIGINT",
        }
    }

    pub fn received_log_line(self) -> String {
        format!("Shutdown signal received: {}", self.as_str())
    }

    pub fn second_signal_reason(self) -> &'static str {
        match self {
            Self::Sigterm => "second signal SIGTERM",
            Self::Sigint => "second signal SIGINT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    Begin(ShutdownSignal),
    Force(ShutdownSignal),
}

pub fn classify_signal(shutting_down: bool, signal: ShutdownSignal) -> SignalAction {
    if shutting_down {
        SignalAction::Force(signal)
    } else {
        SignalAction::Begin(signal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownStatus {
    Complete {
        elapsed_ms: u128,
    },
    Forced {
        elapsed_ms: u128,
        reason: &'static str,
    },
}

impl ShutdownStatus {
    pub fn log_line(&self) -> String {
        match self {
            Self::Complete { elapsed_ms } => format!("Shutdown complete in {elapsed_ms}ms"),
            Self::Forced { elapsed_ms, reason } => {
                format!("Shutdown forced after {elapsed_ms}ms: {reason}")
            }
        }
    }
}

pub struct SignalListener {
    signals: watch::Receiver<Option<ShutdownSignal>>,
    finished: Option<oneshot::Sender<()>>,
}

impl SignalListener {
    pub fn signals(&mut self) -> &mut watch::Receiver<Option<ShutdownSignal>> {
        &mut self.signals
    }
}

impl Drop for SignalListener {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send(());
        }
    }
}

pub fn spawn_signal_listener() -> SignalListener {
    let (tx, rx) = watch::channel(None::<ShutdownSignal>);
    let (finished_tx, finished_rx) = oneshot::channel();
    tokio::spawn(async move {
        if let Err(error) = listen_for_signals(tx, finished_rx).await {
            tracing::error!("Shutdown signal listener failed: {error}");
        }
    });
    SignalListener {
        signals: rx,
        finished: Some(finished_tx),
    }
}

async fn listen_for_signals(
    tx: watch::Sender<Option<ShutdownSignal>>,
    mut finished: oneshot::Receiver<()>,
) -> Result<()> {
    listen_unix(tx, &mut finished).await
}

async fn listen_unix(
    tx: watch::Sender<Option<ShutdownSignal>>,
    finished: &mut oneshot::Receiver<()>,
) -> Result<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    let received = tokio::select! {
        _ = &mut *finished => return Ok(()),
        _ = sigterm.recv() => ShutdownSignal::Sigterm,
        _ = sigint.recv() => ShutdownSignal::Sigint,
    };
    let SignalAction::Begin(signal) = classify_signal(false, received) else {
        unreachable!("the first signal starts shutdown");
    };
    info!("{}", signal.received_log_line());
    let _ = tx.send(Some(signal));
    let started = Instant::now();
    tokio::select! {
        _ = &mut *finished => Ok(()),
        _ = tokio::time::sleep(HARD_SHUTDOWN_BACKSTOP) => {
            force_exit(ShutdownStatus::Forced {
                elapsed_ms: started.elapsed().as_millis(),
                reason: "hard timeout",
            });
        }
        _ = sigterm.recv() => force_exit(forced_signal(started, ShutdownSignal::Sigterm)),
        _ = sigint.recv() => force_exit(forced_signal(started, ShutdownSignal::Sigint)),
    }
}

fn forced_signal(started: Instant, signal: ShutdownSignal) -> ShutdownStatus {
    let SignalAction::Force(signal) = classify_signal(true, signal) else {
        unreachable!("a signal during shutdown is a force exit");
    };
    ShutdownStatus::Forced {
        elapsed_ms: started.elapsed().as_millis(),
        reason: signal.second_signal_reason(),
    }
}

fn force_exit(status: ShutdownStatus) -> ! {
    warn!("{}", status.log_line());
    kill_tracked_bridge();
    flush_logs();
    std::process::exit(0);
}

pub fn finish_process(status: ShutdownStatus) -> ! {
    match &status {
        ShutdownStatus::Complete { .. } => info!("{}", status.log_line()),
        ShutdownStatus::Forced { .. } => warn!("{}", status.log_line()),
    }
    kill_tracked_bridge();
    flush_logs();
    std::process::exit(0);
}

static BRIDGE_PID: AtomicI32 = AtomicI32::new(0);

pub fn set_bridge_pid(pid: u32) {
    BRIDGE_PID.store(i32::try_from(pid).unwrap_or(0), Ordering::SeqCst);
}

pub fn clear_bridge_pid() {
    BRIDGE_PID.store(0, Ordering::SeqCst);
}

fn kill_tracked_bridge() {
    let pid = BRIDGE_PID.swap(0, Ordering::SeqCst);
    if pid <= 0 {
        return;
    }
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn flush_logs() {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
}

/// Stop accepting on `listener` and wait until current connections finish.
pub async fn serve_until_shutdown(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
}

/// Owns the axum task. Dropping it, including when the drain budget fires,
/// stops the server task so a stuck connection cannot hold shutdown open.
pub struct ServerDrain {
    trigger: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ServerDrain {
    pub fn new(trigger: oneshot::Sender<()>, task: JoinHandle<()>) -> Self {
        Self {
            trigger: Some(trigger),
            task,
        }
    }
}

impl Future for ServerDrain {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if let Some(trigger) = this.trigger.take() {
            let _ = trigger.send(());
        }
        match Pin::new(&mut this.task).poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for ServerDrain {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn run_shutdown_steps<D, B, S>(
    budgets: ShutdownBudgets,
    drain: D,
    stop_bridge: B,
    close_sqlite: S,
) -> ShutdownStatus
where
    D: Future<Output = ()>,
    B: Future<Output = bool>,
    S: Future<Output = bool>,
{
    let started = Instant::now();
    let steps = async {
        let mut reason = None;
        if tokio::time::timeout(budgets.request_drain, drain)
            .await
            .is_err()
        {
            reason = Some("request drain timed out");
        }
        if tokio::time::timeout(budgets.bridge, stop_bridge)
            .await
            .unwrap_or(false)
        {
            // Clean bridge exit.
        } else if reason.is_none() {
            reason = Some("bridge stop timed out");
        }
        if !tokio::time::timeout(budgets.sqlite, close_sqlite)
            .await
            .unwrap_or(false)
            && reason.is_none()
        {
            reason = Some("database close timed out");
        }
        match reason {
            None => ShutdownStatus::Complete {
                elapsed_ms: started.elapsed().as_millis(),
            },
            Some(reason) => ShutdownStatus::Forced {
                elapsed_ms: started.elapsed().as_millis(),
                reason,
            },
        }
    };

    match tokio::time::timeout(budgets.hard, steps).await {
        Ok(status) => status,
        Err(_) => ShutdownStatus::Forced {
            elapsed_ms: started.elapsed().as_millis(),
            reason: "hard timeout",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn force_exit_kills_the_tracked_bridge_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        set_bridge_pid(child.id());
        kill_tracked_bridge();
        let status = child.wait().expect("wait");
        assert!(!status.success());
        assert_eq!(BRIDGE_PID.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shutdown_log_lines_name_the_signal_and_the_outcome() {
        assert_eq!(
            ShutdownSignal::Sigterm.received_log_line(),
            "Shutdown signal received: SIGTERM"
        );
        assert_eq!(
            ShutdownSignal::Sigint.received_log_line(),
            "Shutdown signal received: SIGINT"
        );
        assert_eq!(
            ShutdownStatus::Complete { elapsed_ms: 12 }.log_line(),
            "Shutdown complete in 12ms"
        );
        assert_eq!(
            ShutdownStatus::Forced {
                elapsed_ms: 8000,
                reason: "hard timeout",
            }
            .log_line(),
            "Shutdown forced after 8000ms: hard timeout"
        );
        assert_eq!(
            ShutdownSignal::Sigint.second_signal_reason(),
            "second signal SIGINT"
        );
        assert_eq!(
            classify_signal(false, ShutdownSignal::Sigterm),
            SignalAction::Begin(ShutdownSignal::Sigterm)
        );
        assert_eq!(
            classify_signal(true, ShutdownSignal::Sigint),
            SignalAction::Force(ShutdownSignal::Sigint)
        );
    }

    #[tokio::test]
    async fn shutdown_steps_run_drain_then_bridge_then_database() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let status = run_shutdown_steps(
            ShutdownBudgets {
                request_drain: Duration::from_secs(1),
                bridge: Duration::from_secs(1),
                sqlite: Duration::from_secs(1),
                hard: Duration::from_secs(2),
            },
            {
                let order = order.clone();
                async move {
                    order.lock().expect("order").push("drain");
                }
            },
            {
                let order = order.clone();
                async move {
                    order.lock().expect("order").push("bridge");
                    true
                }
            },
            {
                let order = order.clone();
                async move {
                    order.lock().expect("order").push("database");
                    true
                }
            },
        )
        .await;

        assert!(matches!(status, ShutdownStatus::Complete { .. }));
        assert_eq!(
            order.lock().expect("order").as_slice(),
            ["drain", "bridge", "database"]
        );
    }

    #[tokio::test]
    async fn hard_timeout_stops_a_stuck_shutdown_step() {
        let started = Instant::now();
        let status = run_shutdown_steps(
            ShutdownBudgets {
                request_drain: Duration::from_secs(30),
                bridge: Duration::from_secs(30),
                sqlite: Duration::from_secs(30),
                hard: Duration::from_millis(100),
            },
            std::future::pending::<()>(),
            std::future::pending::<bool>(),
            std::future::pending::<bool>(),
        )
        .await;

        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            status,
            ShutdownStatus::Forced {
                elapsed_ms: status_elapsed(&status),
                reason: "hard timeout",
            }
        );
    }

    #[tokio::test]
    async fn bridge_timeout_still_closes_the_database() {
        let closed = Arc::new(Mutex::new(false));
        let status = run_shutdown_steps(
            ShutdownBudgets {
                request_drain: Duration::from_millis(50),
                bridge: Duration::from_millis(50),
                sqlite: Duration::from_secs(1),
                hard: Duration::from_secs(2),
            },
            std::future::ready(()),
            std::future::pending::<bool>(),
            {
                let closed = closed.clone();
                async move {
                    *closed.lock().expect("closed") = true;
                    true
                }
            },
        )
        .await;

        assert!(*closed.lock().expect("closed"));
        match status {
            ShutdownStatus::Forced { reason, .. } => {
                assert_eq!(reason, "bridge stop timed out");
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[tokio::test]
    async fn inflight_request_finishes_and_new_connections_stop() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));
        let app = axum::Router::new().route(
            "/hold",
            axum::routing::get({
                let started_tx = Arc::clone(&started_tx);
                let release_rx = Arc::clone(&release_rx);
                move || {
                    let started_tx = Arc::clone(&started_tx);
                    let release_rx = Arc::clone(&release_rx);
                    async move {
                        let started = started_tx.lock().expect("started").take();
                        let release = release_rx.lock().expect("release").take();
                        if let Some(tx) = started {
                            let _ = tx.send(());
                        }
                        if let Some(rx) = release {
                            let _ = rx.await;
                        }
                        "done"
                    }
                }
            }),
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = serve_until_shutdown(listener, app, async move {
                let _ = shutdown_rx.await;
            })
            .await;
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .get(format!("http://{addr}/hold"))
                    .send()
                    .await
                    .expect("inflight response")
                    .text()
                    .await
                    .expect("inflight body")
            }
        });
        started_rx.await.expect("request started");

        let shutdown = tokio::spawn(async move {
            run_shutdown_steps(
                ShutdownBudgets {
                    request_drain: Duration::from_secs(2),
                    bridge: Duration::from_secs(1),
                    sqlite: Duration::from_secs(1),
                    hard: Duration::from_secs(3),
                },
                ServerDrain::new(shutdown_tx, task),
                std::future::ready(true),
                std::future::ready(true),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !shutdown.is_finished(),
            "shutdown waited for the inflight request"
        );
        release_tx.send(()).expect("release");
        let body = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect("request join")
            .expect("request task");
        assert_eq!(body, "done");
        let status = tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .expect("shutdown join")
            .expect("shutdown task");
        assert!(matches!(status, ShutdownStatus::Complete { .. }));

        let refused = client.get(format!("http://{addr}/hold")).send().await;
        assert!(refused.is_err(), "listener stopped accepting");
    }

    #[tokio::test]
    async fn request_drain_timeout_is_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/hold",
            axum::routing::get(|| async { std::future::pending::<&'static str>().await }),
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = serve_until_shutdown(listener, app, async move {
                let _ = shutdown_rx.await;
            })
            .await;
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let pending = tokio::spawn(async move {
            let _ = client.get(format!("http://{addr}/hold")).send().await;
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let started = Instant::now();
        let status = run_shutdown_steps(
            ShutdownBudgets {
                request_drain: Duration::from_millis(150),
                bridge: Duration::from_millis(50),
                sqlite: Duration::from_millis(50),
                hard: Duration::from_secs(2),
            },
            ServerDrain::new(shutdown_tx, task),
            std::future::ready(true),
            std::future::ready(true),
        )
        .await;

        assert!(started.elapsed() < Duration::from_secs(2));
        match status {
            ShutdownStatus::Forced { reason, .. } => {
                assert_eq!(reason, "request drain timed out");
            }
            other => panic!("unexpected status: {other:?}"),
        }
        pending.abort();
    }

    fn status_elapsed(status: &ShutdownStatus) -> u128 {
        match status {
            ShutdownStatus::Complete { elapsed_ms } | ShutdownStatus::Forced { elapsed_ms, .. } => {
                *elapsed_ms
            }
        }
    }
}
