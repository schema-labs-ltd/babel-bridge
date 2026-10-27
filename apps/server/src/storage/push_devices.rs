//! Registered push notification devices.

use super::*;

impl MessageStore {
    pub fn register_push_device(
        &self,
        installation_id: &str,
        token: &str,
        environment: &str,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM push_devices WHERE installation_id = ?1 OR token = ?2",
            params![installation_id, token],
        )?;
        tx.execute(
            r#"
            INSERT INTO push_devices (token, installation_id, environment, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                token,
                installation_id,
                environment,
                chrono::Utc::now().timestamp()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_push_devices(&self) -> Result<Vec<PushDevice>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT installation_id, token, environment FROM push_devices ORDER BY installation_id",
        )?;
        let devices = stmt
            .query_map([], |row| {
                Ok(PushDevice {
                    installation_id: row.get(0)?,
                    token: row.get(1)?,
                    environment: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(devices)
    }

    pub fn delete_push_device(&self, token: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM push_devices WHERE token = ?", params![token])? > 0)
    }

    pub fn delete_push_installation(&self, installation_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM push_devices WHERE installation_id = ?",
            params![installation_id],
        )? > 0)
    }
}
