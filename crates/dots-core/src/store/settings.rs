use serde_json::Value;

use super::Store;
use crate::settings::Settings;
use crate::Result;

/// Sets the `settings` row of a key (`?` one) to a value (`?` two), replacing what was there.
pub(crate) const SET_SETTING_SQL: &str = "INSERT INTO settings (key, value) VALUES (?, ?) \
     ON CONFLICT(key) DO UPDATE SET value = excluded.value";

impl Store {
    /// This data folder's install id: a ULID made the first time it is asked for and kept in the
    /// `settings` table under `install_id`, outside `Settings` (env vault spec §5.2). It names
    /// the folder's secrets in Credential Manager.
    pub async fn install_id(&self) -> Result<String> {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('install_id', ?)")
            .bind(Value::String(crate::util::new_id()).to_string())
            .execute(&self.pool)
            .await?;
        let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'install_id'")
            .fetch_one(&self.pool)
            .await?;
        match serde_json::from_str::<Value>(&raw)? {
            Value::String(id) => Ok(id),
            _ => Err(crate::Error::Other(
                "the stored install id is not text".into(),
            )),
        }
    }

    /// A marker Dots keeps in the `settings` table outside `Settings` (`key` is none of its
    /// fields), as JSON.
    pub async fn marker(&self, key: &str) -> Result<Option<Value>> {
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(raw.map(|r| serde_json::from_str(&r)).transpose()?)
    }

    /// Sets a marker; with `keep` an existing one is left as it is.
    pub async fn set_marker(&self, key: &str, value: &Value, keep: bool) -> Result<()> {
        let sql = if keep {
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)"
        } else {
            SET_SETTING_SQL
        };
        sqlx::query(sql)
            .bind(key)
            .bind(value.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

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
