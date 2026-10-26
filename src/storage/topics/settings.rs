use super::*;

impl MessageStore {
    pub fn set_topics_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        anyhow::ensure!(
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM contacts WHERE id=?)",
                [id],
                |r| r.get::<_, bool>(0)
            )?,
            "Conversation not found"
        );
        tx.execute("INSERT INTO topic_settings VALUES (?1,?2,?3) ON CONFLICT(contact_id) DO UPDATE SET enabled=excluded.enabled,epoch=excluded.epoch", params![id,enabled,uuid::Uuid::new_v4().to_string()])?;
        if enabled {
            tx.execute("UPDATE topic_label_jobs SET attempts=0,retry_at=0 WHERE topic_id IN (SELECT id FROM chat_topics WHERE contact_id=?)", [id])?;
            tx.execute("UPDATE topic_jobs SET attempts=0,retry_at=0 WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", [id])?;
            tx.execute(r#"INSERT OR IGNORE INTO topic_jobs(message_id)
                SELECT id FROM (SELECT id, original_text,content_type,content_json FROM messages WHERE contact_id=?1
                    AND lower(content_type)!='reaction' AND lower(COALESCE(json_extract(content_json,'$.type'),''))!='reaction'
                    AND length(trim(COALESCE(original_text,'')))>0 ORDER BY timestamp DESC,id DESC LIMIT ?2)
                WHERE id NOT IN (SELECT message_id FROM topic_assignments)"#, params![id,INITIAL_MESSAGES])?;
        } else {
            tx.execute("DELETE FROM topic_jobs WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn topic_settings(&self) -> Result<Vec<TopicSetting>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt=conn.prepare(r#"SELECT s.contact_id,s.enabled,
            CASE WHEN s.enabled=1 THEN
            (SELECT count(*) FROM topic_jobs j JOIN messages m ON m.id=j.message_id WHERE m.contact_id=s.contact_id AND j.attempts<3) +
            (SELECT count(*) FROM topic_assignments a JOIN messages m ON m.id=a.message_id JOIN chat_topics t ON t.id=a.topic_id AND t.contact_id=m.contact_id JOIN topic_label_jobs j ON j.topic_id=t.id WHERE t.contact_id=s.contact_id AND j.attempts<3) ELSE 0 END,
            CASE WHEN s.enabled=1 THEN
            (SELECT count(*) FROM topic_jobs j JOIN messages m ON m.id=j.message_id WHERE m.contact_id=s.contact_id AND j.attempts>=3) +
            (SELECT count(*) FROM topic_assignments a JOIN messages m ON m.id=a.message_id JOIN chat_topics t ON t.id=a.topic_id AND t.contact_id=m.contact_id JOIN topic_label_jobs j ON j.topic_id=t.id WHERE t.contact_id=s.contact_id AND j.attempts>=3) ELSE 0 END
            FROM topic_settings s JOIN contacts c ON c.id=s.contact_id ORDER BY s.contact_id"#)?;
        let result = stmt
            .query_map([], |r| {
                Ok(TopicSetting {
                    contact_id: r.get(0)?,
                    enabled: r.get(1)?,
                    pending_count: r.get(2)?,
                    failed_count: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }

    /// Counts only: no chat identifiers, text, or topic titles in operational logs.
    pub fn log_topic_queue_health(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let (ready, delayed, failed): (i64,i64,i64) = conn.query_row(r#"SELECT
            COALESCE(sum(j.attempts<3 AND j.retry_at<=?1),0), COALESCE(sum(j.attempts<3 AND j.retry_at>?1),0), COALESCE(sum(j.attempts>=3),0)
            FROM topic_jobs j JOIN messages m ON m.id=j.message_id JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1"#, [chrono::Utc::now().timestamp()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let (label_ready, label_delayed, label_failed, disabled_label_messages, invalid_label_assignments): (i64,i64,i64,i64,i64) = conn.query_row(r#"SELECT
            (SELECT count(*) FROM topic_label_jobs j JOIN chat_topics t ON t.id=j.topic_id JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1 WHERE j.attempts<3 AND j.retry_at<=?1 AND EXISTS(SELECT 1 FROM topic_assignments a JOIN messages m ON m.id=a.message_id AND m.contact_id=t.contact_id WHERE a.topic_id=t.id)),
            (SELECT count(*) FROM topic_label_jobs j JOIN chat_topics t ON t.id=j.topic_id JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1 WHERE j.attempts<3 AND j.retry_at>?1 AND EXISTS(SELECT 1 FROM topic_assignments a JOIN messages m ON m.id=a.message_id AND m.contact_id=t.contact_id WHERE a.topic_id=t.id)),
            (SELECT count(*) FROM topic_label_jobs j JOIN chat_topics t ON t.id=j.topic_id JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1 WHERE j.attempts>=3 AND EXISTS(SELECT 1 FROM topic_assignments a JOIN messages m ON m.id=a.message_id AND m.contact_id=t.contact_id WHERE a.topic_id=t.id)),
            (SELECT count(*) FROM topic_assignments a JOIN chat_topics t ON t.id=a.topic_id JOIN topic_label_jobs j ON j.topic_id=t.id JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=0 WHERE j.attempts<3),
            (SELECT count(*) FROM topic_assignments a JOIN chat_topics t ON t.id=a.topic_id JOIN topic_label_jobs j ON j.topic_id=t.id WHERE j.attempts<3 AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=a.message_id AND m.contact_id=t.contact_id))"#, [chrono::Utc::now().timestamp()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        tracing::info!(
            ready,
            delayed,
            failed,
            label_ready,
            label_delayed,
            label_failed,
            disabled_label_messages,
            invalid_label_assignments,
            "Topic queue health"
        );
        Ok(())
    }

    pub fn message_topics(&self, ids: &[String]) -> Result<Vec<crate::topics::MessageTopic>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(r#"SELECT m.id,m.contact_id,COALESCE(json_extract(m.content_json,'$.edited_at_ms'),0),t.title,
            CASE WHEN t.id IS NOT NULL THEN 'assigned' WHEN COALESCE(s.enabled,0)=0 THEN 'off'
                 WHEN j.attempts>=3 THEN 'failed' WHEN j.message_id IS NOT NULL THEN 'pending' ELSE 'unassigned' END
            FROM messages m LEFT JOIN topic_settings s ON s.contact_id=m.contact_id
            LEFT JOIN topic_assignments a ON a.message_id=m.id
            LEFT JOIN chat_topics t ON t.id=a.topic_id AND t.contact_id=m.contact_id
            LEFT JOIN topic_jobs j ON j.message_id=m.id
            WHERE m.id IN (SELECT value FROM json_each(?1)) ORDER BY m.id"#)?;
        let result = stmt
            .query_map([serde_json::to_string(ids)?], |r| {
                Ok(crate::topics::MessageTopic {
                    message_id: r.get(0)?,
                    contact_id: r.get(1)?,
                    revision: r.get(2)?,
                    title: r.get(3)?,
                    state: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }

    pub fn list_topics(&self) -> Result<Vec<ChatTopic>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt=conn.prepare(r#"SELECT t.id,t.contact_id,COALESCE(c.name,c.phone,t.contact_id),t.title,count(*),max(m.timestamp),t.name_key
            FROM chat_topics t JOIN topic_assignments a ON a.topic_id=t.id JOIN messages m ON m.id=a.message_id AND m.contact_id=t.contact_id
            JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1 JOIN contacts c ON c.id=t.contact_id
            GROUP BY t.id ORDER BY max(m.timestamp) DESC,t.id"#)?;
        let result = stmt
            .query_map([], |r| {
                Ok(ChatTopic {
                    id: r.get(0)?,
                    category_id: crate::topics::category_id(&r.get::<_, String>(6)?),
                    contact_id: r.get(1)?,
                    contact_name: r.get(2)?,
                    title: r.get(3)?,
                    message_count: r.get(4)?,
                    last_message_time: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }

    pub fn topic_names(&self, contact_id: &str) -> Result<Vec<String>> {
        let mut topics = self.list_topics()?;
        // Reuse labels across chats while keeping this chat's useful labels first.
        topics.sort_by_key(|t| t.contact_id != contact_id);
        let mut seen = std::collections::HashSet::new();
        Ok(topics
            .into_iter()
            .filter(|t| seen.insert(t.category_id.clone()))
            .take(60)
            .map(|t| t.title)
            .collect())
    }
}
