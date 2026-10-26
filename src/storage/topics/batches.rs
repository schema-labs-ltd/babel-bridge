use super::*;

impl MessageStore {
    pub fn topic_message_input(&self, batch: &TopicBatch) -> Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        // Include unclassified neighbours too, but never another chat or future
        // messages. Keep each target's context separate during history imports.
        let columns = r#"SELECT m.id,COALESCE(NULLIF(m.original_text,''),json_extract(m.content_json,'$.body'),json_extract(m.content_json,'$.caption'),''),t.title,m.timestamp,m.is_from_me
            FROM messages m LEFT JOIN topic_assignments a ON a.message_id=m.id
            LEFT JOIN chat_topics t ON t.id=a.topic_id AND t.contact_id=m.contact_id"#;
        let mut previous = conn.prepare(&format!(r#"{columns} WHERE m.contact_id=?1 AND (m.timestamp<?2 OR (m.timestamp=?2 AND m.id<?3)) AND m.timestamp>=?4
            AND lower(m.content_type)!='reaction' AND lower(COALESCE(json_extract(m.content_json,'$.type'),''))!='reaction'
            AND length(trim(COALESCE(NULLIF(m.original_text,''),json_extract(m.content_json,'$.body'),json_extract(m.content_json,'$.caption'),'')))>0
            ORDER BY m.timestamp DESC,m.id DESC LIMIT 6"#))?;
        let mut quoted = conn.prepare(&format!(
            "{columns} WHERE m.contact_id=?1 AND m.id=?2 AND m.id!=?3 AND m.timestamp<=?4"
        ))?;
        let context_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
            Ok(
                json!({"messageId":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?.chars().take(600).collect::<String>(),
                "topic":r.get::<_,Option<String>>(2)?,"timestamp":r.get::<_,i64>(3)?,"isFromMe":r.get::<_,bool>(4)?}),
            )
        };
        batch.messages.iter().map(|message| {
            anyhow::ensure!(message.contact_id == batch.contact_id, "Invalid topic context ownership");
            let since = message.timestamp.saturating_sub(crate::topics::IMPORT_DAYS * 24 * 60 * 60 * 1000);
            let mut context = previous.query_map(params![batch.contact_id,message.timestamp,message.id,since],context_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            context.reverse();
            let content: Value = serde_json::from_str(&message.content_json).unwrap_or(Value::Null);
            let quote = content.get("reply_context").filter(|v| !v.is_null()).or_else(|| content.get("reply_to")).or_else(|| content.get("replyTo"));
            let quote_id = quote.and_then(|q| q.as_str().or_else(|| q.get("messageId").or_else(|| q.get("message_id")).or_else(|| q.get("id")).and_then(Value::as_str)));
            let resolved = match quote_id {
                Some(id) => quoted.query_row(params![batch.contact_id,id,message.id,message.timestamp],context_row).optional()?,
                None => None,
            };
            let reply = resolved.or_else(|| quote.map(|q| json!({"messageId":quote_id.map(|id|id.chars().take(512).collect::<String>()),
                "text":q.get("text").or_else(||q.get("body")).and_then(Value::as_str).unwrap_or("").chars().take(600).collect::<String>()})));
            Ok(json!({"messageId":message.id,"text":message.original_text.as_deref().unwrap_or("").chars().take(2000).collect::<String>(),
                "timestamp":message.timestamp,"isFromMe":message.is_from_me,"replyTo":reply,"previousMessages":context}))
        }).collect()
    }

    pub fn next_topic_batch(&self) -> Result<Option<TopicBatch>> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let next:Option<(String,String,i64)>=conn.query_row(r#"SELECT m.contact_id,s.epoch,j.attempts FROM topic_jobs j JOIN messages m ON m.id=j.message_id
            JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1 WHERE j.attempts<3 AND j.retry_at<=? ORDER BY j.retry_at,m.timestamp,m.id LIMIT 1"#, [now], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((contact_id, epoch, attempts)) = next else {
            return Ok(None);
        };
        // A slow or invalid message must not repeatedly exhaust a whole batch.
        let limit = if attempts == 0 { BATCH_SIZE } else { 1 };
        let mut stmt=conn.prepare(r#"SELECT m.id,m.contact_id,m.timestamp,m.is_from_me,m.is_forwarded,m.sender_name,m.sender_phone,m.chat_type,m.content_type,
            m.content_json,m.original_text,m.translated_text,m.source_language,m.is_translated,m.delivery_status FROM messages m JOIN topic_jobs j ON j.message_id=m.id
            WHERE m.contact_id=?1 AND j.attempts=?3 AND j.retry_at<=?2 ORDER BY m.timestamp,m.id LIMIT ?4"#)?;
        let mut messages = stmt
            .query_map(params![contact_id, now, attempts, limit], |r| {
                Self::row_to_stored_message(r, None, None)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for m in &mut messages {
            let (raw, parsed) = Self::strip_media_from_content(&m.content_json);
            m.content_json = raw;
            m.content = parsed;
        }
        Ok(Some(TopicBatch {
            contact_id,
            epoch,
            messages,
        }))
    }

    pub fn finish_topic_batch(
        &self,
        batch: &TopicBatch,
        assignments: &[TopicAssignment],
    ) -> Result<usize> {
        // Validate ownership and completeness again at the persistence boundary.
        let mut seen = std::collections::HashSet::new();
        anyhow::ensure!(
            assignments.len() == batch.messages.len(),
            "Incomplete topic assignments"
        );
        for a in assignments {
            anyhow::ensure!(
                seen.insert(&a.message_id)
                    && batch
                        .messages
                        .iter()
                        .any(|m| m.id == a.message_id && m.contact_id == batch.contact_id),
                "Invalid topic ownership"
            );
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM topic_settings WHERE contact_id=? AND enabled=1 AND epoch=?)",params![batch.contact_id,batch.epoch],|r|r.get::<_,bool>(0))? {return Ok(0)}
        let mut assigned = 0;
        for a in assignments {
            let message = batch
                .messages
                .iter()
                .find(|m| m.id == a.message_id)
                .unwrap();
            let current:Option<(Option<String>,i64)>=tx.query_row("SELECT original_text,COALESCE(json_extract(content_json,'$.edited_at_ms'),0) FROM messages WHERE id=? AND contact_id=?",params![message.id,batch.contact_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if current != Some((message.original_text.clone(), revision(message))) {
                continue;
            }
            let title = a.topic.trim();
            anyhow::ensure!(valid_title(title), "Invalid topic title");
            let key = title.to_lowercase();
            tx.execute(
                "INSERT OR IGNORE INTO chat_topics(id,contact_id,title,name_key) VALUES(?,?,?,?)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    batch.contact_id,
                    title,
                    key
                ],
            )?;
            let topic_id: String = tx.query_row(
                "SELECT id FROM chat_topics WHERE contact_id=? AND name_key=?",
                params![batch.contact_id, key],
                |r| r.get(0),
            )?;
            tx.execute("INSERT INTO topic_assignments VALUES(?,?) ON CONFLICT(message_id) DO UPDATE SET topic_id=excluded.topic_id",params![message.id,topic_id])?;
            tx.execute("DELETE FROM topic_jobs WHERE message_id=?", [&message.id])?;
            assigned += 1;
        }
        tx.commit()?;
        Ok(assigned)
    }

    pub fn retry_topic_batch(&self, batch: &TopicBatch) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM topic_settings WHERE contact_id=? AND enabled=1 AND epoch=?)",params![batch.contact_id,batch.epoch],|r|r.get::<_,bool>(0))? {return Ok(())}
        for m in &batch.messages {
            tx.execute(r#"UPDATE topic_jobs SET attempts=attempts+1,retry_at=?1+(30*(attempts+1)*(attempts+1)) WHERE message_id=?2
                AND EXISTS(SELECT 1 FROM messages WHERE id=?2 AND contact_id=?3 AND original_text IS ?4 AND COALESCE(json_extract(content_json,'$.edited_at_ms'),0)=?5)"#,params![chrono::Utc::now().timestamp(),m.id,batch.contact_id,m.original_text,revision(m)])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn topic_messages(
        &self,
        topic_id: &str,
        limit: u32,
        before: Option<i64>,
        before_id: Option<&str>,
    ) -> Result<Option<Vec<StoredMessage>>> {
        let conn = self.conn.lock().unwrap();
        let category = crate::topics::category_key(topic_id);
        let is_category = category.is_some();
        let key = category.as_deref().unwrap_or(topic_id);
        if !conn.query_row("SELECT EXISTS(SELECT 1 FROM chat_topics t JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1 WHERE (?2=1 AND t.name_key=?1) OR (?2=0 AND t.id=?1))",params![key,is_category],|r|r.get::<_,bool>(0))? {return Ok(None)}
        let mut stmt=conn.prepare(r#"SELECT m.id,m.contact_id,m.timestamp,m.is_from_me,m.is_forwarded,m.sender_name,m.sender_phone,m.chat_type,m.content_type,
            m.content_json,m.original_text,m.translated_text,m.source_language,m.is_translated,m.delivery_status,c.name,c.phone
            FROM messages m JOIN topic_assignments a ON a.message_id=m.id JOIN chat_topics t ON t.id=a.topic_id AND t.contact_id=m.contact_id
            JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1
            LEFT JOIN contacts c ON c.id=m.contact_id WHERE ((?5=1 AND t.name_key=?1) OR (?5=0 AND t.id=?1)) AND (?2 IS NULL OR m.timestamp<?2 OR(m.timestamp=?2 AND ?3 IS NOT NULL AND m.id<?3))
            ORDER BY m.timestamp DESC,m.id DESC LIMIT ?4"#)?;
        let mut messages = stmt
            .query_map(params![key, before, before_id, limit, is_category], |r| {
                Self::row_to_stored_message(r, r.get(15)?, r.get(16)?)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for m in &mut messages {
            let (raw, parsed) = Self::strip_media_from_content(&m.content_json);
            m.content_json = raw;
            m.content = parsed;
        }
        messages.reverse();
        Ok(Some(messages))
    }
}
