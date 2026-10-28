use super::*;

#[tokio::test(flavor = "current_thread")]
async fn verbose_mcp_read_logs_metadata_without_message_or_number() {
    const BODY: &str = "zz-unique-body-449d";
    const PHONE: &str = "15550001111";
    const NAME: &str = "ZzContactName449d";
    let jid = format!("{PHONE}@s.whatsapp.net");
    let (state, dir) = test_state();
    state
        .store
        .upsert_contact(&jid, Some(NAME), Some(PHONE), Some("private"), 10)
        .unwrap();
    let mut message = extension_message("privacy-1", &jid, 10, true);
    message.sender_name = Some(NAME.into());
    message.sender_phone = Some(PHONE.into());
    message.contact_name = Some(NAME.into());
    message.contact_phone = Some(PHONE.into());
    message.original_text = Some(BODY.into());
    message.translated_text = Some("zz-translated-body-449d".into());
    message.content_json = json!({"type":"text","body":BODY}).to_string();
    message.content = Some(json!({"type":"text","body":BODY}));
    state.store.add_message(&message).unwrap();
    feed_oauth_token(&state, "privacy-token", "whatsapp.read", i64::MAX);

    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(crate::log_privacy::logging_filter(true))
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer({
            let buf = buf.clone();
            move || LogCapture(buf.clone())
        })
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let read = feed_rpc(
        state.clone(),
        Some("Bearer privacy-token"),
        "tools/call",
        json!({"name":"read_messages","arguments":{"contact_id":jid,"limit":5}}),
    )
    .await;
    let read_body = feed_rpc_body(read).await;
    let read_text = read_body.to_string();
    assert!(
        read_text.contains(BODY),
        "api response must still return the message"
    );
    assert!(
        read_text.contains(PHONE),
        "api response must still return the number"
    );

    let failed = feed_rpc(
        state.clone(),
        Some("Bearer privacy-token"),
        "tools/call",
        json!({"name":"read_messages","arguments":{"contact_id":"missing-chat","note":BODY,"phone":PHONE}}),
    )
    .await;
    let failed_body = feed_rpc_body(failed).await;
    assert!(failed_body.get("error").is_some(), "{failed_body}");

    let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains(BODY), "{logs}");
    assert!(!logs.contains(PHONE), "{logs}");
    assert!(!logs.contains(NAME), "{logs}");
    assert!(!logs.contains("zz-translated-body-449d"), "{logs}");
    assert!(logs.contains("method=\"tools/call\""), "{logs}");
    assert!(logs.contains("tool=\"read_messages\""), "{logs}");
    assert!(logs.contains("status=\"ok\""), "{logs}");
    assert!(logs.contains("status=\"error\""), "{logs}");
    assert!(logs.contains("count=1"), "{logs}");
    assert!(logs.contains("duration_ms="), "{logs}");
    assert!(logs.contains("error_code="), "{logs}");
    drop(_guard);
    let _ = std::fs::remove_dir_all(dir);
}

struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
