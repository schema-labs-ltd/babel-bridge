//! Shared wire fixtures for the Rust <-> Go bridge protocol, in `fixtures/bridge`.
//!
//! The bridge owns events (Go writes, Rust reads) and the server owns commands
//! (Rust writes, Go reads). `wa-bridge/protocol_fixtures_test.go` checks the
//! other half. After an intended protocol change, regenerate the command
//! fixtures with `UPDATE_FIXTURES=1 cargo test bridge::protocol` and the event
//! fixtures with `UPDATE_FIXTURES=1 go test ./...` in `wa-bridge/`.

use super::protocol::*;
use std::path::PathBuf;

fn fixture_dir(kind: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/bridge")
        .join(kind)
}

fn fixture_names(kind: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixture_dir(kind))
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension()? == "json").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    names
}

fn event(name: &str) -> BridgeEvent {
    let path = fixture_dir("events").join(format!("{name}.json"));
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error} (the bridge writes these)", path.display()));
    serde_json::from_str(&json).unwrap_or_else(|error| {
        panic!("{name}: the server can no longer read this bridge event: {error}")
    })
}

fn message(name: &str) -> Message {
    match event(name) {
        BridgeEvent::Message(message) => message,
        other => panic!("{name}: expected a message event, got {other:?}"),
    }
}

#[test]
fn server_reads_every_bridge_event_fixture() {
    let checked = [
        "chat_presence",
        "connected",
        "connection_state",
        "error",
        "log",
        "logged_out",
        "mark_as_read",
        "message_audio",
        "message_edit_caption",
        "message_history",
        "message_image",
        "message_location",
        "message_reaction",
        "message_text",
        "message_voice_note",
        "profile_picture",
        "qr",
        "read_receipt_result",
        "receipt",
        "send_progress",
        "send_result_album",
        "send_result_failure",
        "send_result_success",
    ];
    assert_eq!(
        fixture_names("events"),
        checked,
        "add a check below for each new bridge event fixture"
    );

    assert!(matches!(event("qr"), BridgeEvent::Qr { data } if data == "2@ABC123"));
    assert!(matches!(
        event("connected"),
        BridgeEvent::Connected { phone, name, platform: Some(platform) }
            if phone == "447700900000" && name == "Me" && platform == "android"
    ));
    assert!(matches!(
        event("connection_state"),
        BridgeEvent::ConnectionState {
            state: ConnectionState::Reconnecting
        }
    ));

    let text = message("message_text");
    assert_eq!(text.id, "3EB0TEXT");
    assert_eq!(text.timestamp.timestamp(), 1767225600);
    assert_eq!(text.from.jid, "447700900001@s.whatsapp.net");
    assert_eq!(text.from.name.as_deref(), Some("Ana"));
    assert!(matches!(&text.chat, Chat::Private { name: Some(name), .. } if name == "Ana"));
    assert!(matches!(&text.content, MessageContent::Text { body } if body == "Olá @447700900002"));
    let reply = text.reply_context.expect("reply context");
    assert_eq!(
        (reply.message_id.as_str(), reply.sender_name.as_str()),
        ("3EB0QUOTED", "Ben")
    );
    assert!(!text.is_history);
    assert_eq!(text.push_name.as_deref(), Some("Ana"));

    let history = message("message_history");
    assert!(history.is_history);
    assert_eq!(history.unread_count, Some(2));

    let image = message("message_image");
    assert!(matches!(
        &image.chat,
        Chat::Group {
            participant_count: Some(3),
            ..
        }
    ));
    assert!(matches!(
        &image.content,
        MessageContent::Image {
            caption: Some(caption),
            mime_type,
            file_size: 2048,
            album_id: Some(album),
            album_index: Some(1),
            width: Some(1280),
            height: Some(960),
            ..
        } if caption == "Beach" && mime_type == "image/jpeg" && album == "3EB0ALBUM"
    ));

    assert!(matches!(
        message("message_voice_note").content,
        MessageContent::Audio {
            is_voice_note: true,
            duration_seconds: Some(7),
            file_size: 4096,
            ..
        }
    ));
    assert!(matches!(
        message("message_audio").content,
        MessageContent::Audio {
            is_voice_note: false,
            media_data: Some(data),
            mime_type,
            ..
        } if data == "YXVkaW8=" && mime_type == "audio/mpeg"
    ));
    assert!(matches!(
        message("message_location").content,
        MessageContent::Location { latitude, name: Some(name), .. }
            if (latitude - 51.5007).abs() < 1e-9 && name == "Big Ben"
    ));
    assert!(matches!(
        message("message_reaction").content,
        MessageContent::Reaction { emoji, target_message_id, sender_timestamp_ms: Some(1767225601000) }
            if emoji == "👍" && target_message_id == "3EB0IMAGE"
    ));

    match event("message_edit_caption") {
        BridgeEvent::MessageEdit {
            message,
            edited_at_ms,
        } => {
            assert_eq!(edited_at_ms, 1767225700000);
            assert_eq!(message.id, "3EB0IMAGE");
            assert!(matches!(
                message.content,
                MessageContent::Image { caption: Some(caption), .. } if caption == "Beach at sunset"
            ));
        }
        other => panic!("expected message_edit, got {other:?}"),
    }

    assert!(matches!(
        event("send_result_success"),
        BridgeEvent::SendResult { request_id: 4, success: true, message_id: Some(id), timestamp: Some(1767225600), error: None, .. }
            if id == "3EB0SENT"
    ));
    assert!(matches!(
        event("send_result_failure"),
        BridgeEvent::SendResult { request_id: 5, success: false, message_id: None, error: Some(error), .. }
            if error == "not connected"
    ));
    assert!(matches!(
        event("send_result_album"),
        BridgeEvent::SendResult { request_id: 6, message_ids, timestamps, .. }
            if message_ids == ["3EB0ALBUM1", "3EB0ALBUM2"] && timestamps == [1767225600, 1767225601]
    ));
    assert!(matches!(
        event("send_progress"),
        BridgeEvent::SendProgress { request_id: 6, stage, completed: 1, total: 2, .. } if stage == "uploading"
    ));
    assert!(matches!(
        event("profile_picture"),
        BridgeEvent::ProfilePicture { request_id: 7, url: Some(_), id: Some(id), error: None, .. } if id == "1700000000"
    ));
    assert!(matches!(
        event("chat_presence"),
        BridgeEvent::ChatPresence {
            state: ChatPresenceState::Typing,
            ..
        }
    ));
    assert!(matches!(
        event("receipt"),
        BridgeEvent::Receipt { message_ids, status } if message_ids == ["3EB0SENT"] && status == "read"
    ));
    assert!(matches!(
        event("read_receipt_result"),
        BridgeEvent::ReadReceiptResult {
            success: false,
            error: Some(_),
            ..
        }
    ));
    assert!(matches!(
        event("mark_as_read"),
        BridgeEvent::MarkAsRead { chat_id } if chat_id == "447700900001@s.whatsapp.net"
    ));
    assert!(matches!(
        event("error"),
        BridgeEvent::Error { code, .. } if code == "connection_failed"
    ));
    assert!(matches!(event("log"), BridgeEvent::Log { level, .. } if level == "info"));
    assert!(matches!(
        event("logged_out"),
        BridgeEvent::LoggedOut { reason } if reason == "device removed"
    ));
}

fn fixture_commands() -> Vec<(&'static str, BridgeCommand)> {
    let chat = "447700900001@s.whatsapp.net".to_string();
    let group = "120363000000000001@g.us".to_string();
    vec![
        (
            "send",
            BridgeCommand::Send {
                request_id: Some(1),
                to: chat.clone(),
                text: "Olá".into(),
                reply_to: Some("3EB0QUOTED".into()),
                reply_to_sender: Some("447700900002@s.whatsapp.net".into()),
                reply_to_text: Some("Are you coming?".into()),
            },
        ),
        (
            "send_audio",
            BridgeCommand::SendAudio {
                request_id: Some(2),
                to: chat.clone(),
                media_data: "T2dnUw==".into(),
                duration_seconds: 7,
                reply_to: None,
                reply_to_sender: None,
                reply_to_text: None,
            },
        ),
        (
            "send_media",
            BridgeCommand::SendMedia {
                request_id: Some(3),
                to: chat.clone(),
                media_data: "JVBERi0=".into(),
                mime_type: "application/pdf".into(),
                media_kind: "document".into(),
                file_name: Some("menu.pdf".into()),
                caption: Some("Menu".into()),
                reply_to: None,
                reply_to_sender: None,
                reply_to_text: None,
            },
        ),
        (
            "send_image",
            BridgeCommand::SendImage {
                request_id: Some(4),
                to: group.clone(),
                media_data: "/9j/4AAQ".into(),
                mime_type: "image/jpeg".into(),
                caption: Some("Beach".into()),
                reply_to: None,
                reply_to_sender: None,
                reply_to_text: None,
            },
        ),
        (
            "send_images",
            BridgeCommand::SendImages {
                request_id: Some(6),
                progress_id: Some("progress-1".into()),
                to: group.clone(),
                images: vec![
                    BridgeImage {
                        media_data: "/9j/4AAQ".into(),
                        mime_type: "image/jpeg".into(),
                    },
                    BridgeImage {
                        media_data: "iVBORw0K".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                caption: Some("Holiday".into()),
                reply_to: None,
                reply_to_sender: None,
                reply_to_text: None,
            },
        ),
        (
            "send_reaction",
            BridgeCommand::SendReaction {
                request_id: Some(8),
                to: group.clone(),
                message_id: "3EB0IMAGE".into(),
                sender_jid: Some(chat.clone()),
                emoji: "👍".into(),
            },
        ),
        (
            "mark_read",
            BridgeCommand::MarkRead {
                to: group,
                message_id: "3EB0IMAGE".into(),
                timestamp: 1767225700,
                sender_jid: Some(chat.clone()),
            },
        ),
        (
            "get_profile_picture",
            BridgeCommand::GetProfilePicture {
                request_id: 7,
                to: chat,
            },
        ),
        ("disconnect", BridgeCommand::Disconnect),
        ("logout", BridgeCommand::Logout),
    ]
}

#[test]
fn server_commands_match_shared_fixtures() {
    let commands = fixture_commands();
    let update = std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1");
    let dir = fixture_dir("commands");
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, command) in &commands {
        let json = serde_json::to_string_pretty(command).unwrap() + "\n";
        let path = dir.join(format!("{name}.json"));
        if update {
            std::fs::write(&path, json).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error} (regenerate with UPDATE_FIXTURES=1 cargo test bridge::protocol)",
                path.display()
            )
        });
        assert_eq!(
            want, json,
            "{name}: server command changed; if intended, regenerate with \
             UPDATE_FIXTURES=1 cargo test bridge::protocol and check the Go reader"
        );
    }
    if !update {
        let mut names: Vec<&str> = commands.iter().map(|(name, _)| *name).collect();
        names.sort();
        assert_eq!(fixture_names("commands"), names);
    }
}
