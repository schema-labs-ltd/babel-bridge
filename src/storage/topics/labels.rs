use super::*;

impl MessageStore {
    pub fn next_topic_labels(&self) -> Result<Vec<TopicLabel>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(r#"SELECT t.id,t.contact_id,s.epoch,t.title FROM topic_label_jobs j
            JOIN chat_topics t ON t.id=j.topic_id JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1
            WHERE j.attempts<3 AND j.retry_at<=?1 AND EXISTS(SELECT 1 FROM topic_assignments a JOIN messages m ON m.id=a.message_id AND m.contact_id=t.contact_id WHERE a.topic_id=t.id)
            ORDER BY j.retry_at,t.name_key,t.id LIMIT ?2"#)?;
        let labels = stmt
            .query_map(
                params![chrono::Utc::now().timestamp(), LABEL_BATCH_SIZE],
                |r| {
                    Ok(TopicLabel {
                        id: r.get(0)?,
                        contact_id: r.get(1)?,
                        epoch: r.get(2)?,
                        title: r.get(3)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(labels)
    }

    pub fn finish_topic_labels(
        &self,
        labels: &[TopicLabel],
        assignments: &[TopicAssignment],
    ) -> Result<usize> {
        let mut seen = std::collections::HashSet::new();
        anyhow::ensure!(
            assignments.len() == labels.len(),
            "Incomplete topic label assignments"
        );
        for assignment in assignments {
            anyhow::ensure!(
                seen.insert(&assignment.message_id)
                    && labels.iter().any(|label| label.id == assignment.message_id)
                    && valid_title(&assignment.topic),
                "Invalid topic label assignment"
            );
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut moves = Vec::new();
        for assignment in assignments {
            let label = labels
                .iter()
                .find(|label| label.id == assignment.message_id)
                .unwrap();
            // Opt-out, reset, and stale responses cannot change a conversation's assignments.
            if !tx.query_row(r#"SELECT EXISTS(SELECT 1 FROM chat_topics t JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1
                JOIN topic_label_jobs j ON j.topic_id=t.id WHERE t.id=?1 AND t.contact_id=?2 AND t.title=?3 AND s.epoch=?4)"#,
                params![label.id,label.contact_id,label.title,label.epoch], |r| r.get::<_,bool>(0))? { continue; }
            let title = assignment.topic.trim();
            let key = title.to_lowercase();
            tx.execute("INSERT OR IGNORE INTO chat_topics(id,contact_id,title,name_key) VALUES(?1,?2,?3,?4)",params![uuid::Uuid::new_v4().to_string(),label.contact_id,title,key])?;
            let target: String = tx.query_row(
                "SELECT id FROM chat_topics WHERE contact_id=?1 AND name_key=?2",
                params![label.contact_id, key],
                |r| r.get(0),
            )?;
            moves.push(json!({"source":label.id,"target":target,"chat":label.contact_id}));
            tx.execute("DELETE FROM topic_label_jobs WHERE topic_id=?", [&label.id])?;
        }
        // Apply all mappings in one statement so an A -> B mapping cannot be moved again by B -> C.
        tx.execute(r#"UPDATE topic_assignments AS a SET topic_id=(SELECT json_extract(value,'$.target') FROM json_each(?1) WHERE json_extract(value,'$.source')=a.topic_id)
            WHERE EXISTS(SELECT 1 FROM json_each(?1) j JOIN messages m ON m.id=a.message_id
                WHERE json_extract(j.value,'$.source')=a.topic_id AND json_extract(j.value,'$.chat')=m.contact_id)"#, [serde_json::to_string(&moves)?])?;
        tx.commit()?;
        Ok(moves.len())
    }

    pub fn retry_topic_labels(&self, labels: &[TopicLabel]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for label in labels {
            tx.execute(r#"UPDATE topic_label_jobs SET attempts=attempts+1,retry_at=?1+(30*(attempts+1)*(attempts+1)) WHERE topic_id=?2
                AND EXISTS(SELECT 1 FROM chat_topics t JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1
                    WHERE t.id=?2 AND t.contact_id=?3 AND t.title=?4 AND s.epoch=?5)"#,
                params![chrono::Utc::now().timestamp(),label.id,label.contact_id,label.title,label.epoch])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Retry existing opted-in jobs once after a known request failure is fixed.
    pub(super) fn recover_topic_jobs(&self, marker: &str, reason: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let first_run = tx.execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES(?1,'1')",
            [marker],
        )? > 0;
        let recovered = if first_run {
            Some((tx.execute(r#"UPDATE topic_jobs SET attempts=0,retry_at=0 WHERE attempts>0
                AND message_id IN (SELECT m.id FROM messages m JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1)
                AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=topic_jobs.message_id)"#, [])?,
                tx.execute(r#"UPDATE topic_label_jobs SET attempts=0,retry_at=0 WHERE attempts>0 AND topic_id IN
                    (SELECT t.id FROM chat_topics t JOIN topic_settings s ON s.contact_id=t.contact_id AND s.enabled=1)"#,[])?))
        } else {
            None
        };
        tx.commit()?;
        if let Some((messages, topics)) = recovered {
            tracing::info!(
                messages,
                topics,
                reason,
                "Recovered topic jobs after request fix"
            );
        }
        Ok(())
    }

    /// Older batches could consume every retry together. Give only exhausted,
    /// unassigned, opted-in messages a bounded retry through the isolated path.
    pub(super) fn recover_isolated_topic_retries(&self) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let first_run = tx.execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('topic_isolated_retries_v1','1')",
            [],
        )? > 0;
        let recovered = if first_run {
            tx.execute(r#"UPDATE topic_jobs SET attempts=1,retry_at=0 WHERE attempts>=3
                AND message_id IN (SELECT m.id FROM messages m JOIN topic_settings s ON s.contact_id=m.contact_id AND s.enabled=1)
                AND NOT EXISTS(SELECT 1 FROM topic_assignments a WHERE a.message_id=topic_jobs.message_id)"#, [])?
        } else {
            0
        };
        tx.commit()?;
        if recovered > 0 {
            tracing::info!(
                messages = recovered,
                "Topic recovery queued individual retries"
            );
        }
        Ok(())
    }
}
