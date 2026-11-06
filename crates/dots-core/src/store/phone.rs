//! The phone view's records (phone view spec section 6): the paired phones, by the hash of
//! their credential only; what each phone did; and the listener's certificate.

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::{Error, Result};

/// A paired phone, as Settings lists it. The credential's hash never leaves the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneDevice {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub last_ip: Option<String>,
}

/// One thing a phone did (the audit trail). `action` is `approve`, `deny`, `answer`,
/// `run_anyway`, `paired` or `forgot`; `target` the id it was done to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneAction {
    pub device_id: String,
    pub device_name: String,
    pub action: String,
    pub target: String,
    pub summary: String,
    pub at: String,
}

/// The listener's certificate and its private key as the protector encrypted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneTls {
    pub cert_der: Vec<u8>,
    pub key_protected: Vec<u8>,
}

const DEVICE_COLUMNS: &str = "id, name, created_at, last_seen_at, last_ip";

fn device_from_row(row: &SqliteRow) -> Result<PhoneDevice> {
    Ok(PhoneDevice {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        last_ip: row.try_get("last_ip")?,
    })
}

impl Store {
    /// Adds a paired phone with the hash of its credential.
    pub async fn add_phone_device(&self, device: &PhoneDevice, token_hash: &str) -> Result<()> {
        let added = sqlx::query(
            "INSERT INTO phone_devices (id, name, token_hash, created_at, last_seen_at, last_ip) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&device.id)
        .bind(&device.name)
        .bind(token_hash)
        .bind(&device.created_at)
        .bind(&device.last_seen_at)
        .bind(&device.last_ip)
        .execute(&self.pool)
        .await;
        match added {
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Err(Error::Conflict("this phone is paired already".into()))
            }
            other => other.map(|_| ()).map_err(Into::into),
        }
    }

    /// The phone whose credential hashes to `token_hash`, with the stored hash, so the caller
    /// can compare the two in constant time.
    pub async fn phone_device_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<(PhoneDevice, String)>> {
        let row = sqlx::query(&format!(
            "SELECT {DEVICE_COLUMNS}, token_hash FROM phone_devices WHERE token_hash = ?"
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| Ok((device_from_row(&r)?, r.try_get("token_hash")?)))
            .transpose()
    }

    /// Every paired phone, newest first.
    pub async fn list_phone_devices(&self) -> Result<Vec<PhoneDevice>> {
        let rows = sqlx::query(&format!(
            "SELECT {DEVICE_COLUMNS} FROM phone_devices ORDER BY created_at DESC, id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(device_from_row).collect()
    }

    /// Removes a phone: its credential stops working at once. `NotFound` if it is gone.
    pub async fn remove_phone_device(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM phone_devices WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("phone {id}")));
        }
        Ok(())
    }

    /// Notes when and from where a phone was last seen.
    pub async fn touch_phone_device(&self, id: &str, at: &str, ip: &str) -> Result<()> {
        sqlx::query("UPDATE phone_devices SET last_seen_at = ?, last_ip = ? WHERE id = ?")
            .bind(at)
            .bind(ip)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_phone_action(&self, action: &PhoneAction) -> Result<()> {
        sqlx::query(
            "INSERT INTO phone_actions (device_id, device_name, action, target, summary, at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&action.device_id)
        .bind(&action.device_name)
        .bind(&action.action)
        .bind(&action.target)
        .bind(&action.summary)
        .bind(&action.at)
        .execute(&self.pool)
        .await?;
        // The trail keeps the newest 1000 actions.
        sqlx::query(
            "DELETE FROM phone_actions WHERE id <= (SELECT MAX(id) FROM phone_actions) - 1000",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The newest `limit` phone actions, newest first.
    pub async fn recent_phone_actions(&self, limit: u32) -> Result<Vec<PhoneAction>> {
        let rows = sqlx::query(
            "SELECT device_id, device_name, action, target, summary, at FROM phone_actions \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(PhoneAction {
                    device_id: r.try_get("device_id")?,
                    device_name: r.try_get("device_name")?,
                    action: r.try_get("action")?,
                    target: r.try_get("target")?,
                    summary: r.try_get("summary")?,
                    at: r.try_get("at")?,
                })
            })
            .collect()
    }

    pub async fn phone_tls(&self) -> Result<Option<PhoneTls>> {
        let row = sqlx::query("SELECT cert_der, key_protected FROM phone_tls WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            Ok(PhoneTls {
                cert_der: r.try_get("cert_der")?,
                key_protected: r.try_get("key_protected")?,
            })
        })
        .transpose()
    }

    /// Keeps the listener's certificate, replacing the one kept before.
    pub async fn put_phone_tls(&self, tls: &PhoneTls, at: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO phone_tls (id, cert_der, key_protected, created_at) VALUES (1, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET cert_der = excluded.cert_der, \
             key_protected = excluded.key_protected, created_at = excluded.created_at",
        )
        .bind(&tls.cert_der)
        .bind(&tls.key_protected)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
