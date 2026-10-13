use serde_json::Value;

use super::Store;
use crate::settings::Settings;
use crate::Result;

impl Store {
    /// The stored settings, with defaults for missing keys. Unknown keys are ignored, and so is
    /// a stored value that does not fit its field (logged), so one bad row never hides the rest.
    pub async fn get_settings(&self) -> Result<Settings> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await?;
        let mut merged = serde_json::to_value(Settings::default())?;
        for (key, raw) in rows {
            if merged.get(&key).is_none() {
                continue;
            }
            let mut candidate = merged.clone();
            match serde_json::from_str::<Value>(&raw) {
                Ok(value) => candidate[&key] = value,
                Err(e) => {
                    tracing::warn!("ignoring setting {key}: {e}");
                    continue;
                }
            }
            let accepted = serde_json::from_value::<Settings>(candidate.clone())
                .map_err(crate::Error::from)
                .and_then(|s| s.validate());
            match accepted {
                Ok(()) => merged = candidate,
                Err(e) => tracing::warn!("ignoring setting {key}: {e}"),
            }
        }
        Ok(serde_json::from_value(merged)?)
    }

    /// Validates `settings` and stores every field in one transaction.
    pub async fn put_settings(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        let Value::Object(fields) = serde_json::to_value(settings)? else {
            unreachable!("Settings serializes to a JSON object");
        };
        let mut tx = self.pool.begin().await?;
        for (key, value) in fields {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(&key)
            .bind(value.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
