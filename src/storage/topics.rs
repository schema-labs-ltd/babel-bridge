use super::*;
use crate::topics::{
    revision, valid_title, ChatTopic, TopicAssignment, TopicBatch, TopicLabel, TopicSetting,
    BATCH_SIZE, INITIAL_MESSAGES, LABEL_BATCH_SIZE,
};
use serde_json::{json, Value};

mod batches;
mod labels;
mod settings;

impl MessageStore {
    /// The manual initial import uses only locally stored text and captions in the last seven days.
    /// Already assigned messages are excluded, and an import never expands to older history.
    pub fn topic_import_preview(&self, now_ms: i64) -> Result<crate::topics::TopicImportSummary> {
        let conn = self.conn.lock().unwrap();
        Self::topic_import_summary(&conn, now_ms)
    }

    fn topic_import_summary(
        conn: &rusqlite::Connection,
        now_ms: i64,
    ) -> Result<crate::topics::TopicImportSummary> {
        let since = now_ms - crate::topics::IMPORT_DAYS * 24 * 60 * 60 * 1000;
        Ok(conn.query_row(r#"SELECT count(DISTINCT m.contact_id),count(*) FROM messages m
            JOIN contacts c ON c.id=m.contact_id
            WHERE m.timestamp>=?1 AND m.timestamp<=?2 AND m.contact_id!='status@broadcast'
              AND length(trim(COALESCE(m.original_text,'')))>0
              AND lower(m.content_type)!='reaction' AND lower(COALESCE(json_extract(m.content_json,'$.type'),''))!='reaction'
              AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=m.id)"#,
            params![since,now_ms], |r|Ok(crate::topics::TopicImportSummary { days:crate::topics::IMPORT_DAYS,chat_count:r.get(0)?,message_count:r.get(1)? }))?)
    }

    pub fn import_recent_topics(&self, now_ms: i64) -> Result<crate::topics::TopicImportSummary> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let summary = Self::topic_import_summary(&tx, now_ms)?;
        let since = now_ms - crate::topics::IMPORT_DAYS * 24 * 60 * 60 * 1000;
        let candidates: Vec<(String, String)> = {
            let mut stmt=tx.prepare(r#"SELECT m.id,m.contact_id FROM messages m JOIN contacts c ON c.id=m.contact_id
                WHERE m.timestamp>=?1 AND m.timestamp<=?2 AND m.contact_id!='status@broadcast'
                  AND length(trim(COALESCE(m.original_text,'')))>0
                  AND lower(m.content_type)!='reaction' AND lower(COALESCE(json_extract(m.content_json,'$.type'),''))!='reaction'
                  AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=m.id)"#)?;
            let rows = stmt
                .query_map(params![since, now_ms], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let chats: std::collections::HashSet<_> =
            candidates.iter().map(|(_, id)| id.as_str()).collect();
        for id in chats {
            // Preserve an enabled chat's epoch so its current batch remains valid.
            tx.execute("INSERT INTO topic_settings VALUES (?1,1,?2) ON CONFLICT(contact_id) DO UPDATE SET enabled=1,epoch=CASE WHEN topic_settings.enabled=1 THEN topic_settings.epoch ELSE excluded.epoch END",params![id,uuid::Uuid::new_v4().to_string()])?;
        }
        for (id, _) in candidates {
            tx.execute("INSERT INTO topic_jobs(message_id) VALUES(?) ON CONFLICT(message_id) DO UPDATE SET attempts=0,retry_at=0",[id])?;
        }
        tx.commit()?;
        Ok(summary)
    }

    pub(super) fn init_topics(&self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch(r#"
            CREATE TABLE IF NOT EXISTS topic_settings(contact_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, epoch TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS chat_topics(id TEXT PRIMARY KEY, contact_id TEXT NOT NULL, title TEXT NOT NULL, name_key TEXT NOT NULL, UNIQUE(contact_id,name_key));
            CREATE TABLE IF NOT EXISTS topic_assignments(message_id TEXT PRIMARY KEY, topic_id TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS topic_assignments_topic ON topic_assignments(topic_id);
            CREATE TABLE IF NOT EXISTS topic_jobs(message_id TEXT PRIMARY KEY, attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS topic_label_jobs(topic_id TEXT PRIMARY KEY, attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER NOT NULL DEFAULT 0);
            CREATE TRIGGER IF NOT EXISTS topics_insert AFTER INSERT ON messages
            WHEN EXISTS(SELECT 1 FROM topic_settings WHERE contact_id=NEW.contact_id AND enabled=1)
                AND length(trim(COALESCE(NEW.original_text,'')))>0
                AND lower(NEW.content_type)!='reaction' AND lower(COALESCE(json_extract(NEW.content_json,'$.type'),''))!='reaction'
            BEGIN INSERT OR IGNORE INTO topic_jobs(message_id) VALUES(NEW.id); END;
            CREATE TRIGGER IF NOT EXISTS topics_edit AFTER UPDATE OF original_text,content_json ON messages
            WHEN OLD.original_text IS NOT NEW.original_text OR COALESCE(json_extract(OLD.content_json,'$.edited_at_ms'),0)!=COALESCE(json_extract(NEW.content_json,'$.edited_at_ms'),0)
            BEGIN
                DELETE FROM topic_assignments WHERE message_id=NEW.id;
                DELETE FROM topic_jobs WHERE message_id=NEW.id;
                INSERT OR IGNORE INTO topic_jobs(message_id) SELECT NEW.id
                  WHERE EXISTS(SELECT 1 FROM topic_settings WHERE contact_id=NEW.contact_id AND enabled=1)
                    AND length(trim(COALESCE(NEW.original_text,'')))>0
                    AND lower(NEW.content_type)!='reaction' AND lower(COALESCE(json_extract(NEW.content_json,'$.type'),''))!='reaction';
            END;
            CREATE TRIGGER IF NOT EXISTS topics_delete AFTER DELETE ON messages
            BEGIN DELETE FROM topic_assignments WHERE message_id=OLD.id; DELETE FROM topic_jobs WHERE message_id=OLD.id; END;
            CREATE TRIGGER IF NOT EXISTS topics_confirm_id AFTER UPDATE OF id ON messages WHEN OLD.id!=NEW.id
            BEGIN
                UPDATE OR IGNORE topic_jobs SET message_id=NEW.id WHERE message_id=OLD.id;
                DELETE FROM topic_jobs WHERE message_id=OLD.id;
                UPDATE OR IGNORE topic_assignments SET message_id=NEW.id WHERE message_id=OLD.id;
                DELETE FROM topic_assignments WHERE message_id=OLD.id;
            END;
        "#)?;
        self.recover_topic_jobs("topic_json_input_v1", "json_input")?;
        self.recover_topic_jobs("topic_background_timeout_v1", "background_timeout")?;
        self.recover_topic_jobs("topic_compact_batches_v1", "compact_batches")?;
        self.queue_legacy_topic_labels()?;
        self.recover_isolated_topic_retries()?;
        self.recover_outgoing_topics(chrono::Utc::now().timestamp_millis())?;
        Ok(())
    }

    /// Repair only recent, unassigned outgoing messages in opted-in chats.
    /// A restart cannot repeatedly import older history or replace saved categories.
    fn recover_outgoing_topics(&self, now_ms: i64) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if tx.execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('topic_outgoing_context_v1','1')",
            [],
        )? == 0
        {
            return Ok(());
        }
        let since = now_ms - crate::topics::IMPORT_DAYS * 24 * 60 * 60 * 1000;
        let restored = tx.execute(r#"UPDATE messages SET original_text=CASE lower(json_extract(content_json,'$.type'))
                WHEN 'text' THEN json_extract(content_json,'$.body') ELSE json_extract(content_json,'$.caption') END
            WHERE is_from_me=1 AND timestamp>=?1 AND timestamp<=?2 AND length(trim(COALESCE(original_text,'')))=0
                AND lower(json_extract(content_json,'$.type')) IN ('text','image','video','document')
                AND length(trim(COALESCE(CASE lower(json_extract(content_json,'$.type')) WHEN 'text' THEN json_extract(content_json,'$.body') ELSE json_extract(content_json,'$.caption') END,'')))>0
                AND EXISTS(SELECT 1 FROM topic_settings s WHERE s.contact_id=messages.contact_id AND s.enabled=1)
                AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=messages.id)"#, params![since,now_ms])?;
        let queued = tx.execute(r#"INSERT OR IGNORE INTO topic_jobs(message_id)
            SELECT m.id FROM messages m JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1
            WHERE m.is_from_me=1 AND m.timestamp>=?1 AND m.timestamp<=?2 AND length(trim(COALESCE(m.original_text,'')))>0
                AND lower(m.content_type)!='reaction' AND lower(COALESCE(json_extract(m.content_json,'$.type'),''))!='reaction'
                AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=m.id)"#,params![since,now_ms])?;
        tx.execute("DELETE FROM topic_jobs WHERE NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=topic_jobs.message_id)", [])?;
        tx.execute("DELETE FROM topic_assignments WHERE NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=topic_assignments.message_id)", [])?;
        tx.commit()?;
        tracing::info!(restored, queued, "Recovered recent outgoing topic messages");
        Ok(())
    }

    fn queue_legacy_topic_labels(&self) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if tx.execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('topic_broad_labels_v1','1')",
            [],
        )? > 0
        {
            tx.execute(
                "INSERT OR IGNORE INTO topic_label_jobs(topic_id) SELECT id FROM chat_topics",
                [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
