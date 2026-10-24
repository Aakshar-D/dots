//! The Library's items and MCP catalogs (MCP/skills spec sections 2 and 3).

use serde_json::{json, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use super::dots::{dot_from_row, DOT_COLUMNS};
use super::{opt_json, Store};
use crate::library::{
    check_name, CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses,
    PERSONAL_SCOPE,
};
use crate::model::Dot;
use crate::util::{new_id, now};
use crate::vault::org::check_org_id;
use crate::vault::redact::global;
use crate::{Error, Result};

const ITEM_COLUMNS: &str = "id, kind, name, config, owner_id, created_at, updated_at";
/// Servers, then skills, then plugins, then context collections; by name within a kind.
const ITEM_ORDER: &str = "CASE kind WHEN 'mcp_server' THEN 0 WHEN 'skill' THEN 1 \
    WHEN 'plugin' THEN 2 ELSE 3 END, name";
const CATALOG_COLUMNS: &str = "server_id, status, accepted_fingerprint, accepted_tools, \
    seen_fingerprint, seen_tools, checked_at, error";

/// The `config` column: the config of the item's kind, without the kind.
fn config_json(config: &LibraryConfig) -> Result<String> {
    Ok(match config {
        LibraryConfig::McpServer(c) => serde_json::to_string(c)?,
        LibraryConfig::Skill(c) => serde_json::to_string(c)?,
        LibraryConfig::Plugin(c) => serde_json::to_string(c)?,
        LibraryConfig::Context(c) => serde_json::to_string(c)?,
    })
}

fn item_from_row(row: &SqliteRow) -> Result<LibraryItem> {
    let kind = LibraryKind::parse(&row.try_get::<String, _>("kind")?)?;
    let config: Value = serde_json::from_str(&row.try_get::<String, _>("config")?)?;
    Ok(LibraryItem {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        config: serde_json::from_value(json!({ "kind": kind.as_str(), "config": config }))?,
        owner_id: row.try_get("owner_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn catalog_from_row(row: &SqliteRow) -> Result<McpCatalog> {
    Ok(McpCatalog {
        server_id: row.try_get("server_id")?,
        status: CatalogStatus::parse(&row.try_get::<String, _>("status")?)?,
        accepted_fingerprint: row.try_get("accepted_fingerprint")?,
        accepted_tools: opt_json(row.try_get("accepted_tools")?)?,
        seen_fingerprint: row.try_get("seen_fingerprint")?,
        seen_tools: opt_json(row.try_get("seen_tools")?)?,
        checked_at: row.try_get("checked_at")?,
        error: row.try_get("error")?,
    })
}

/// Refuses a catalog scope that is neither `personal` nor an org id.
fn check_scope(scope: &str) -> Result<()> {
    if scope == PERSONAL_SCOPE || check_org_id(scope).is_ok() {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "{scope:?} is not a scope: it is \"personal\" or an org id"
    )))
}

fn no_catalog(server_id: &str, scope: &str) -> Error {
    Error::NotFound(format!("catalog of MCP server {server_id} in {scope}"))
}

/// Refuses a name of `uses` that the Library does not have: an MCP server, skill or plugin of
/// that name. A plugin's MCP server is not one: it comes with the plugin. Run inside the
/// transaction that writes the dot, after its write, so that no removal of an item slips in
/// between.
pub(super) async fn check_uses(conn: &mut SqliteConnection, uses: &Uses) -> Result<()> {
    for kind in LibraryKind::ALL {
        let missing: Option<String> = sqlx::query_scalar(
            "SELECT value FROM json_each(?) WHERE value NOT IN \
             (SELECT name FROM library_items WHERE kind = ? AND owner_id IS NULL) LIMIT 1",
        )
        .bind(serde_json::to_string(uses.names(*kind))?)
        .bind(kind.as_str())
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(name) = missing {
            return Err(Error::Invalid(format!(
                "the Library has no {} named {name:?}",
                kind.label()
            )));
        }
    }
    Ok(())
}

impl Store {
    /// Adds an item. `owner_id` names the plugin an MCP server comes from. A new MCP server
    /// gets an `unchecked` Personal catalog; one in an org is made when it is first needed. A
    /// name its kind already has is a `Conflict`.
    pub async fn add_library_item(
        &self,
        name: &str,
        config: &LibraryConfig,
        owner_id: Option<&str>,
    ) -> Result<LibraryItem> {
        let kind = config.kind();
        check_name(kind, name)?;
        config.validate()?;
        if let Some(owner) = owner_id {
            let owner = self.get_library_item(owner).await?;
            if kind != LibraryKind::McpServer || owner.kind() != LibraryKind::Plugin {
                return Err(Error::Invalid(
                    "only an MCP server belongs to another item, and only to a plugin".into(),
                ));
            }
        }
        let id = new_id();
        let ts = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kind.as_str())
        .bind(name)
        .bind(config_json(config)?)
        .bind(owner_id)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => Error::Conflict(format!(
                "the Library already has a {} named {name:?}",
                kind.label()
            )),
            _ => e.into(),
        })?;
        if kind == LibraryKind::McpServer {
            sqlx::query("INSERT INTO mcp_catalogs (server_id, scope, status) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(PERSONAL_SCOPE)
                .bind(CatalogStatus::Unchecked.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_library_item(&id).await
    }

    pub async fn get_library_item(&self, id: &str) -> Result<LibraryItem> {
        let row = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("library item {id}")))?;
        item_from_row(&row)
    }

    /// The context collections a run of a dot with `uses` reads: those it names and every one
    /// whose `every_dot` is on, by name.
    pub async fn used_collections(&self, uses: &Uses) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = 'context' AND (\
             name IN (SELECT value FROM json_each(?)) \
             OR json_extract(config, '$.every_dot') = 1) ORDER BY name"
        ))
        .bind(serde_json::to_string(&uses.context)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    pub async fn find_library_item(
        &self,
        kind: LibraryKind,
        name: &str,
    ) -> Result<Option<LibraryItem>> {
        sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = ? AND name = ?"
        ))
        .bind(kind.as_str())
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(item_from_row)
        .transpose()
    }

    /// Every item: MCP servers, then skills, then plugins, each by name.
    pub async fn list_library_items(&self) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items ORDER BY {ITEM_ORDER}"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// Changes an item's config; its kind and name stay. A changed MCP server config resets
    /// the server's catalog in every scope to `unchecked` (spec section 6): what it lists must
    /// be checked and accepted again. The accepted list stays for that review, except when an
    /// http server moves to another origin: then nothing accepted for the old host carries
    /// over.
    pub async fn update_library_item(
        &self,
        id: &str,
        config: &LibraryConfig,
    ) -> Result<LibraryItem> {
        config.validate()?;
        let old = self.get_library_item(id).await?;
        if old.kind() != config.kind() {
            return Err(Error::Invalid(format!(
                "the {} {:?} cannot become a {}",
                old.kind().label(),
                old.name,
                config.kind().label()
            )));
        }
        if old.config == *config {
            return Ok(old);
        }
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("UPDATE library_items SET config = ?, updated_at = ? WHERE id = ?")
            .bind(config_json(config)?)
            .bind(now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("library item {id}")));
        }
        if config.kind() == LibraryKind::McpServer {
            // Without values: a URL whose text changed counts as moved (`origin_changed`). Both
            // statements change the server's catalog in every scope.
            if moved_origin(&old.config, config, &|_| None) {
                sqlx::query(
                    "UPDATE mcp_catalogs SET accepted_fingerprint = NULL, accepted_tools = NULL \
                     WHERE server_id = ?",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "UPDATE mcp_catalogs SET status = ?, seen_fingerprint = NULL, seen_tools = NULL, \
                 checked_at = NULL, error = NULL WHERE server_id = ?",
            )
            .bind(CatalogStatus::Unchecked.as_str())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.get_library_item(id).await
    }

    /// Deletes an item, a plugin's MCP servers and their catalogs in every scope, and removes its
    /// name from the `uses` of the dots that name it, in one transaction. Asking first is the
    /// caller's (`dots_using`).
    pub async fn remove_library_item(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        // The delete goes first: it takes the write lock before the dots are read.
        let removed: Option<(String, String)> =
            sqlx::query_as("DELETE FROM library_items WHERE id = ? RETURNING kind, name")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((kind, name)) = removed else {
            return Err(Error::NotFound(format!("library item {id}")));
        };
        let kind = LibraryKind::parse(&kind)?;
        let dots: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, uses FROM dots WHERE EXISTS \
             (SELECT 1 FROM json_each(dots.uses, ?) WHERE value = ?)",
        )
        .bind(format!("$.{}", kind.uses_key()))
        .bind(&name)
        .fetch_all(&mut *tx)
        .await?;
        let ts = now();
        for (dot_id, stored) in dots {
            let mut uses: Uses = serde_json::from_str(&stored)?;
            let list = match kind {
                LibraryKind::McpServer => &mut uses.mcp_servers,
                LibraryKind::Skill => &mut uses.skills,
                LibraryKind::Plugin => &mut uses.plugins,
                LibraryKind::Context => &mut uses.context,
            };
            list.retain(|n| *n != name);
            sqlx::query("UPDATE dots SET uses = ?, updated_at = ? WHERE id = ?")
                .bind(serde_json::to_string(&uses)?)
                .bind(&ts)
                .bind(&dot_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The dots that use `item`, by name. A plugin's MCP server is used by the dots that use
    /// the plugin.
    pub async fn dots_using(&self, item: &LibraryItem) -> Result<Vec<Dot>> {
        let (kind, name) = match &item.owner_id {
            Some(owner) => {
                let owner = self.get_library_item(owner).await?;
                (owner.kind(), owner.name)
            }
            None => (item.kind(), item.name.clone()),
        };
        let rows = sqlx::query(&format!(
            "SELECT {DOT_COLUMNS} FROM dots WHERE EXISTS \
             (SELECT 1 FROM json_each(dots.uses, ?) WHERE value = ?) ORDER BY name"
        ))
        .bind(format!("$.{}", kind.uses_key()))
        .bind(name)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(dot_from_row).collect()
    }

    /// The MCP servers a run of a dot with `uses` opens: those it names and those of the
    /// plugins it names, by name.
    pub async fn used_mcp_servers(&self, uses: &Uses) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = 'mcp_server' AND (\
             (owner_id IS NULL AND name IN (SELECT value FROM json_each(?))) \
             OR owner_id IN (SELECT id FROM library_items WHERE kind = 'plugin' \
             AND name IN (SELECT value FROM json_each(?)))) ORDER BY name"
        ))
        .bind(serde_json::to_string(&uses.mcp_servers)?)
        .bind(serde_json::to_string(&uses.plugins)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// The server's catalog in `scope` (`personal`, or an org id). A server never checked or
    /// run in an org has none stored there yet: it reads as `unchecked`. `NotFound` when there
    /// is no such MCP server.
    pub async fn get_catalog(&self, server_id: &str, scope: &str) -> Result<McpCatalog> {
        check_scope(scope)?;
        let row = sqlx::query(&format!(
            "SELECT {CATALOG_COLUMNS} FROM mcp_catalogs WHERE server_id = ? AND scope = ?"
        ))
        .bind(server_id)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            return catalog_from_row(&row);
        }
        let server: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM library_items WHERE id = ? AND kind = 'mcp_server'")
                .bind(server_id)
                .fetch_optional(&self.pool)
                .await?;
        match server {
            Some(_) => Ok(McpCatalog::unchecked(server_id)),
            None => Err(no_catalog(server_id, scope)),
        }
    }

    /// Every stored catalog with its scope, in every scope: by server, then scope.
    pub async fn list_catalogs(&self) -> Result<Vec<(String, McpCatalog)>> {
        let rows = sqlx::query(&format!(
            "SELECT scope, {CATALOG_COLUMNS} FROM mcp_catalogs ORDER BY server_id, scope"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("scope")?, catalog_from_row(row)?)))
            .collect()
    }

    /// Writes a server's catalog in `scope` as it is given.
    pub async fn put_catalog(&self, scope: &str, catalog: &McpCatalog) -> Result<()> {
        check_scope(scope)?;
        let tools = |t: &Option<Value>| t.as_ref().map(Value::to_string);
        sqlx::query(&format!(
            "INSERT INTO mcp_catalogs (scope, {CATALOG_COLUMNS}) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (server_id, scope) DO UPDATE SET status = excluded.status, \
             accepted_fingerprint = excluded.accepted_fingerprint, \
             accepted_tools = excluded.accepted_tools, \
             seen_fingerprint = excluded.seen_fingerprint, seen_tools = excluded.seen_tools, \
             checked_at = excluded.checked_at, error = excluded.error"
        ))
        .bind(scope)
        .bind(&catalog.server_id)
        .bind(catalog.status.as_str())
        .bind(&catalog.accepted_fingerprint)
        .bind(tools(&catalog.accepted_tools))
        .bind(&catalog.seen_fingerprint)
        .bind(tools(&catalog.seen_tools))
        .bind(&catalog.checked_at)
        .bind(&catalog.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Makes the server's `unchecked` catalog in `scope` if it has none there yet: at the first
    /// check or run of the server in an org. Nothing for a server that is not in the Library.
    async fn ensure_catalog(&self, server_id: &str, scope: &str) -> Result<()> {
        check_scope(scope)?;
        sqlx::query(
            "INSERT INTO mcp_catalogs (server_id, scope, status) SELECT id, ?, ? \
             FROM library_items WHERE id = ? AND kind = 'mcp_server' \
             ON CONFLICT (server_id, scope) DO NOTHING",
        )
        .bind(scope)
        .bind(CatalogStatus::Unchecked.as_str())
        .bind(server_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records a check in `scope` that listed the server's tools (spec section 3): they become
    /// the seen list there, and the status is `accepted` when their fingerprint is the one
    /// accepted in that scope, else `changed`. The update is one statement, so a check never
    /// undoes an accept that lands meanwhile.
    pub async fn record_catalog_seen(
        &self,
        server_id: &str,
        scope: &str,
        fingerprint: &str,
        tools: &Value,
    ) -> Result<McpCatalog> {
        self.ensure_catalog(server_id, scope).await?;
        let row = sqlx::query(&format!(
            "UPDATE mcp_catalogs SET seen_fingerprint = ?, seen_tools = ?, checked_at = ?, \
             error = NULL, status = CASE WHEN accepted_fingerprint = ? THEN ? ELSE ? END \
             WHERE server_id = ? AND scope = ? RETURNING {CATALOG_COLUMNS}"
        ))
        .bind(fingerprint)
        .bind(tools.to_string())
        .bind(now())
        .bind(fingerprint)
        .bind(CatalogStatus::Accepted.as_str())
        .bind(CatalogStatus::Changed.as_str())
        .bind(server_id)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| no_catalog(server_id, scope))?;
        catalog_from_row(&row)
    }

    /// Records a check in `scope` that failed: status `unreachable` with the error. The
    /// accepted and the seen lists stay.
    pub async fn record_catalog_failure(
        &self,
        server_id: &str,
        scope: &str,
        error: &str,
    ) -> Result<McpCatalog> {
        self.ensure_catalog(server_id, scope).await?;
        let row = sqlx::query(&format!(
            "UPDATE mcp_catalogs SET status = ?, error = ?, checked_at = ? \
             WHERE server_id = ? AND scope = ? RETURNING {CATALOG_COLUMNS}"
        ))
        .bind(CatalogStatus::Unreachable.as_str())
        .bind(global().scrub(error).as_ref())
        .bind(now())
        .bind(server_id)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| no_catalog(server_id, scope))?;
        catalog_from_row(&row)
    }

    /// The MCP servers that belong to the plugin `owner_id`, by name.
    pub async fn owned_library_items(&self, owner_id: &str) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE owner_id = ? ORDER BY name"
        ))
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// The MCP servers whose catalog in `scope` is `changed`: their tool list waits to be
    /// reviewed.
    pub async fn changed_mcp_servers(&self, scope: &str) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE id IN \
             (SELECT server_id FROM mcp_catalogs WHERE status = ? AND scope = ?) ORDER BY name"
        ))
        .bind(CatalogStatus::Changed.as_str())
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// Accepts the tool list last seen in `scope`, if its fingerprint is still `fingerprint`:
    /// the seen list becomes the accepted one there and the status `accepted`. A list seen
    /// since, or none seen in that scope, is a `Conflict`: the person accepts only what was
    /// shown.
    pub async fn accept_catalog(
        &self,
        server_id: &str,
        scope: &str,
        fingerprint: &str,
    ) -> Result<McpCatalog> {
        check_scope(scope)?;
        let res = sqlx::query(
            "UPDATE mcp_catalogs SET accepted_fingerprint = seen_fingerprint, \
             accepted_tools = seen_tools, status = ? \
             WHERE server_id = ? AND scope = ? AND seen_fingerprint = ?",
        )
        .bind(CatalogStatus::Accepted.as_str())
        .bind(server_id)
        .bind(scope)
        .bind(fingerprint)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            self.get_catalog(server_id, scope).await?;
            return Err(Error::Conflict(
                "the server's tool list changed since it was shown; review it again".into(),
            ));
        }
        self.get_catalog(server_id, scope).await
    }
}

/// Whether the origin of an http MCP server differs between two configs, as the client will
/// compute it with `env` (a stdio server has none, so a move between stdio and http counts as
/// moved).
fn moved_origin(
    old: &LibraryConfig,
    new: &LibraryConfig,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    use crate::library::{origin_changed, McpServerConfig};
    match (old, new) {
        (
            LibraryConfig::McpServer(McpServerConfig::Http { url: a, .. }),
            LibraryConfig::McpServer(McpServerConfig::Http { url: b, .. }),
        ) => origin_changed(a, b, env),
        (LibraryConfig::McpServer(McpServerConfig::Http { .. }), LibraryConfig::McpServer(_))
        | (
            LibraryConfig::McpServer(McpServerConfig::Stdio { .. }),
            LibraryConfig::McpServer(McpServerConfig::Http { .. }),
        ) => true,
        _ => false,
    }
}
