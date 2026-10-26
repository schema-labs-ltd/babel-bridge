//! Connection status tests.

use super::*;

#[tokio::test]
async fn connection_status_recovers_missed_events_and_expires_old_linking_codes() {
    let (state, data_dir) = test_state(None);
    let mut events = state.broadcast_tx.subscribe();
    state.require_whatsapp_link().await;
    state.set_qr_code("test-code".into()).await;
    let status = get_status(State(state.clone())).await.0;
    assert!(!status.connected);
    assert_eq!(
        status.connection_state,
        WhatsAppConnectionState::LinkingRequired
    );
    assert_eq!(status.qr.as_deref(), Some("test-code"));
    assert!(matches!(
        events.recv().await.unwrap(),
        WebSocketEvent::Status {
            connection_state: WhatsAppConnectionState::LinkingRequired,
            ..
        }
    ));
    // QR timeout/bridge exit keeps the linking flow open while removing the
    // expired code. The next bridge instance supplies a fresh one.
    state.set_connected(false, None, None).await;
    let status = get_status(State(state.clone())).await.0;
    assert_eq!(
        status.connection_state,
        WhatsAppConnectionState::LinkingRequired
    );
    assert!(status.qr.is_none());
    state.set_qr_code("replacement-code".into()).await;
    state
        .set_connected(true, Some("447700900123".into()), Some("Preview".into()))
        .await;
    let status = get_status(State(state)).await.0;
    assert!(status.connected);
    assert_eq!(status.connection_state, WhatsAppConnectionState::Connected);
    assert!(status.qr.is_none());
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn get_status_reports_reconnecting_until_retries_stop() {
    let (state, data_dir) = test_state(None);
    state.set_connected(false, None, None).await;
    let retrying = get_status(State(state.clone())).await.0;
    assert!(!retrying.connected);
    assert_eq!(
        retrying.connection_state,
        WhatsAppConnectionState::Reconnecting
    );
    state.mark_whatsapp_disconnected().await;
    let stopped = get_status(State(state)).await.0;
    assert!(!stopped.connected);
    assert_eq!(
        stopped.connection_state,
        WhatsAppConnectionState::Disconnected
    );
    let _ = std::fs::remove_dir_all(data_dir);
}
