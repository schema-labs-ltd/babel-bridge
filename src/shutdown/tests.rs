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
