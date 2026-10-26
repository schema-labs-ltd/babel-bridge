//! Cached link previews.

use super::*;

impl MessageStore {
    /// Get a cached link preview by URL
    /// Returns None if not cached or if cache is older than max_age_secs
    pub fn get_link_preview(&self, url: &str, max_age_secs: i64) -> Result<Option<LinkPreview>> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let min_time = now - max_age_secs;

        let result = conn.query_row(
            r#"
            SELECT url, title, description, image_url, site_name, error, fetched_at
            FROM link_previews
            WHERE url = ? AND fetched_at > ?
            "#,
            params![url, min_time],
            |row| {
                Ok(LinkPreview {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    image_url: row.get(3)?,
                    site_name: row.get(4)?,
                    error: row.get(5)?,
                })
            },
        );

        match result {
            Ok(preview) => Ok(Some(preview)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Save a link preview to cache
    pub fn save_link_preview(&self, preview: &LinkPreview) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        conn.execute(
            r#"
            INSERT OR REPLACE INTO link_previews 
            (url, title, description, image_url, site_name, error, fetched_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                preview.url,
                preview.title,
                preview.description,
                preview.image_url,
                preview.site_name,
                preview.error,
                now,
            ],
        )?;

        Ok(())
    }
}
