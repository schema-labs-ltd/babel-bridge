//! One-time repair for conversations previously keyed by a WhatsApp device JID.
use super::*;
use std::collections::BTreeSet;

const REPAIR_KEY: &str = "device_jid_repair_v1";

impl MessageStore {
    pub(super) fn migrate_device_jids(&self, conn: &Connection) -> Result<()> {
        if conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key=?)",
            [REPAIR_KEY],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        // Include orphan references left by older imports, plus per-chat settings.
        let ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM contacts UNION SELECT contact_id FROM messages UNION SELECT contact_id FROM translation_usage WHERE contact_id IS NOT NULL UNION SELECT contact_id FROM style_profiles UNION SELECT substr(key,7) FROM app_settings WHERE key LIKE 'voice:%' UNION SELECT substr(key,27) FROM app_settings WHERE key LIKE 'message-tone:conversation:%'")?;
            let rows = stmt
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let aliases: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let canonical = canonical_chat_id(&id).into_owned();
                (canonical != id).then_some((id, canonical))
            })
            .collect();

        // A consistent, private SQLite snapshot must succeed before changing data.
        // Copy SQLite pages, including committed WAL contents, without rebuilding
        // the database in temp_store=MEMORY. Memory use must not scale with its size.
        if !aliases.is_empty() {
            let path = conn
                .path()
                .filter(|p| !p.is_empty())
                .context("Identity repair requires a file-backed database snapshot")?;
            let backup = Path::new(path).with_file_name(format!(
                "messages-before-device-jid-repair-{}.db",
                uuid::Uuid::new_v4()
            ));
            // Reuse only this incomplete destination after an interrupted start.
            // Completed snapshots have unique names and are never overwritten.
            let incomplete =
                Path::new(path).with_file_name("messages-device-jid-repair.incomplete");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let snapshot_file = options
                .open(&incomplete)
                .context("Create identity repair snapshot")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                snapshot_file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            drop(snapshot_file);
            info!(aliases = aliases.len(), "Identity repair snapshot starting");
            let snapshot = (|| -> Result<()> {
                let mut destination = Connection::open(&incomplete)?;
                destination.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-4096; PRAGMA temp_store=FILE;")?;
                {
                    use rusqlite::backup::{Backup, StepResult};
                    use std::time::{Duration, Instant};
                    let copy = Backup::new(conn, &mut destination)?;
                    let started = Instant::now();
                    let mut last_log = started;
                    loop {
                        let result = copy.step(1024)?;
                        if result == StepResult::Done {
                            break;
                        }
                        anyhow::ensure!(
                            started.elapsed() < Duration::from_secs(480),
                            "Identity repair snapshot exceeded eight minutes"
                        );
                        if last_log.elapsed() >= Duration::from_secs(5) {
                            let progress = copy.progress();
                            info!(
                                remaining_pages = progress.remaining,
                                total_pages = progress.pagecount,
                                "Identity repair snapshot progress"
                            );
                            last_log = Instant::now();
                        }
                        if matches!(result, StepResult::Busy | StepResult::Locked) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
                destination.close().map_err(|(_, error)| error)?;
                std::fs::rename(&incomplete, &backup)?;
                Ok(())
            })();
            if let Err(error) = snapshot {
                // Only remove this attempt's incomplete output. A restart must
                // not consume the volume with a new partial snapshot each time.
                if let Err(cleanup) = std::fs::remove_file(&incomplete) {
                    tracing::warn!(
                        "Could not remove incomplete identity repair snapshot: {cleanup}"
                    );
                }
                return Err(error).context("Snapshot database before identity repair");
            }
            info!("Identity repair snapshot saved at {}", backup.display());
        }

        let tx = conn.unchecked_transaction()?;
        let messages_before: i64 =
            tx.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
        let mut moved_messages = 0;
        let mut repaired = BTreeSet::new();
        for (alias, canonical) in &aliases {
            let existed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM contacts WHERE id=?)",
                [canonical],
                |r| r.get(0),
            )?;
            // If the account has no row, inherit the first device row's preferences.
            // Otherwise retain the account's explicit settings, including opt-out.
            tx.execute("INSERT OR IGNORE INTO contacts(id,name,phone,type,last_message_time,unread_count,last_message_preview,pinned_at,participant_count,translation_enabled,language_override,translation_style,send_original_follow_up,reply_only) SELECT ?1,name,phone,type,last_message_time,unread_count,last_message_preview,pinned_at,participant_count,translation_enabled,language_override,translation_style,send_original_follow_up,reply_only FROM contacts WHERE id=?2", params![canonical, alias])?;
            tx.execute("INSERT OR IGNORE INTO contacts(id,type,last_message_time,unread_count) VALUES (?,'private',0,0)", [canonical])?;
            if existed {
                tx.execute("UPDATE contacts SET unread_count=COALESCE(unread_count,0)+COALESCE((SELECT unread_count FROM contacts WHERE id=?2),0), name=COALESCE(NULLIF(name,''),(SELECT NULLIF(name,'') FROM contacts WHERE id=?2)), last_message_time=MAX(COALESCE(last_message_time,0),COALESCE((SELECT last_message_time FROM contacts WHERE id=?2),0)) WHERE id=?1", params![canonical, alias])?;
            }
            if let Some(phone) = crate::identity::phone_number(canonical) {
                tx.execute(
                    "UPDATE contacts SET phone=?2 WHERE id=?1",
                    params![canonical, phone],
                )?;
            }
            moved_messages += tx.execute(
                "UPDATE messages SET contact_id=?1 WHERE contact_id=?2",
                params![canonical, alias],
            )?;
            tx.execute(
                "UPDATE translation_usage SET contact_id=?1 WHERE contact_id=?2",
                params![canonical, alias],
            )?;
            tx.execute("INSERT OR IGNORE INTO style_profiles(contact_id,profile_text,sample_messages,message_count,updated_at) SELECT ?1,profile_text,sample_messages,message_count,updated_at FROM style_profiles WHERE contact_id=?2", params![canonical, alias])?;
            tx.execute("DELETE FROM style_profiles WHERE contact_id=?", [alias])?;
            for prefix in ["voice:", "message-tone:conversation:"] {
                let old_key = format!("{prefix}{alias}");
                let new_key = format!("{prefix}{canonical}");
                tx.execute("INSERT OR IGNORE INTO app_settings(key,value) SELECT ?1,value FROM app_settings WHERE key=?2", params![new_key, old_key])?;
                tx.execute("DELETE FROM app_settings WHERE key=?", [old_key])?;
            }
            tx.execute("DELETE FROM contacts WHERE id=?", [alias])?;
            repaired.insert(canonical);
        }
        for canonical in &repaired {
            Self::refresh_contact_last_message(&tx, canonical)?;
            // An alias's previous opt-in must not override the account's opt-out.
            tx.execute("DELETE FROM translation_jobs WHERE message_id IN (SELECT m.id FROM messages m JOIN contacts c ON c.id=m.contact_id WHERE c.id=? AND c.translation_enabled=0)", [canonical])?;
            tx.execute("UPDATE pending_notifications SET requires_translation=0 WHERE message_id IN (SELECT m.id FROM messages m JOIN contacts c ON c.id=m.contact_id WHERE c.id=? AND c.translation_enabled=0)", [canonical])?;
        }
        let messages_after: i64 =
            tx.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
        anyhow::ensure!(
            messages_before == messages_after,
            "Identity repair changed the message count"
        );
        tx.execute(
            "INSERT INTO app_settings(key,value) VALUES (?,?)",
            params![REPAIR_KEY, "1"],
        )?;
        tx.commit()?;
        info!(
            aliases = aliases.len(),
            conversations = repaired.len(),
            moved_messages,
            messages_before,
            messages_after,
            "Identity repair complete"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
