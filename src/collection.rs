use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A collection (static album) — a manually curated list of asset IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub asset_ids: Vec<String>,
}

/// Summary of a collection for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub asset_count: u64,
    pub created_at: String,
}

/// Wrapper for the YAML file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionsFile {
    pub collections: Vec<Collection>,
}

const FILENAME: &str = "collections.yaml";

/// Load collections from the YAML file. Returns empty list if file doesn't exist.
pub fn load_yaml(catalog_root: &Path) -> Result<CollectionsFile> {
    let path = catalog_root.join(FILENAME);
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let file: CollectionsFile = serde_yaml::from_str(&contents)?;
        Ok(file)
    } else {
        Ok(CollectionsFile::default())
    }
}

/// Save collections to the YAML file.
pub fn save_yaml(catalog_root: &Path, file: &CollectionsFile) -> Result<()> {
    let path = catalog_root.join(FILENAME);
    let contents = serde_yaml::to_string(file)?;
    std::fs::write(path, contents)?;
    Ok(())
}

/// Collection operations backed by SQLite catalog.
pub struct CollectionStore<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> CollectionStore<'a> {
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Create collection tables (called from Catalog::initialize).
    pub fn initialize(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS collections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS collection_assets (
                collection_id TEXT NOT NULL REFERENCES collections(id),
                asset_id TEXT NOT NULL REFERENCES assets(id),
                added_at TEXT NOT NULL,
                PRIMARY KEY (collection_id, asset_id)
            );

            CREATE INDEX IF NOT EXISTS idx_collection_assets_asset
                ON collection_assets(asset_id);",
        )?;
        Ok(())
    }

    /// Create a new collection.
    pub fn create(&self, name: &str, description: Option<&str>) -> Result<Collection> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO collections (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id.to_string(), name, description, now.to_rfc3339()],
        )?;
        Ok(Collection {
            id,
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            created_at: now,
            asset_ids: Vec::new(),
        })
    }

    /// List all collections with summary info.
    pub fn list(&self) -> Result<Vec<CollectionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.description, c.created_at,
                    (SELECT COUNT(*) FROM collection_assets ca WHERE ca.collection_id = c.id)
             FROM collections c ORDER BY c.name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CollectionSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                asset_count: row.get::<_, i64>(4)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get a collection by name.
    pub fn get_by_name(&self, name: &str) -> Result<Option<Collection>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, created_at FROM collections WHERE name = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![name])?;
        if let Some(row) = rows.next()? {
            let id_str: String = row.get(0)?;
            let id: Uuid = id_str.parse().map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;
            let name: String = row.get(1)?;
            let description: Option<String> = row.get(2)?;
            let created_at_str: String = row.get(3)?;
            let created_at: DateTime<Utc> = created_at_str.parse().map_err(|e| anyhow::anyhow!("invalid date: {e}"))?;

            // Load asset IDs
            let asset_ids = self.get_asset_ids(&id_str)?;

            Ok(Some(Collection {
                id,
                name,
                description,
                created_at,
                asset_ids,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get asset IDs for a collection.
    pub fn get_asset_ids(&self, collection_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_id FROM collection_assets WHERE collection_id = ?1 ORDER BY added_at",
        )?;
        let rows = stmt.query_map(rusqlite::params![collection_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Add assets to a collection. Returns the number of new additions.
    pub fn add_assets(&self, collection_name: &str, asset_ids: &[String]) -> Result<u32> {
        let col = self.get_by_name(collection_name)?
            .ok_or_else(|| anyhow::anyhow!("no collection named '{collection_name}'"))?;
        let col_id = col.id.to_string();
        let now = Utc::now().to_rfc3339();
        let mut added = 0u32;
        for id in asset_ids {
            match self.conn.execute(
                "INSERT OR IGNORE INTO collection_assets (collection_id, asset_id, added_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![col_id, id, now],
            ) {
                Ok(n) if n > 0 => added += 1,
                Ok(_) => {} // already present
                Err(e) => eprintln!("Warning: could not add asset {id} to collection: {e}"),
            }
        }
        Ok(added)
    }

    /// Remove assets from a collection. Returns the number of removals.
    pub fn remove_assets(&self, collection_name: &str, asset_ids: &[String]) -> Result<u32> {
        let col = self.get_by_name(collection_name)?
            .ok_or_else(|| anyhow::anyhow!("no collection named '{collection_name}'"))?;
        let col_id = col.id.to_string();
        let mut removed = 0u32;
        for id in asset_ids {
            let n = self.conn.execute(
                "DELETE FROM collection_assets WHERE collection_id = ?1 AND asset_id = ?2",
                rusqlite::params![col_id, id],
            )?;
            if n > 0 {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Delete a collection and its membership records.
    pub fn delete(&self, name: &str) -> Result<()> {
        let col = self.get_by_name(name)?
            .ok_or_else(|| anyhow::anyhow!("no collection named '{name}'"))?;
        let col_id = col.id.to_string();
        self.conn.execute("DELETE FROM collection_assets WHERE collection_id = ?1", rusqlite::params![col_id])?;
        self.conn.execute("DELETE FROM collections WHERE id = ?1", rusqlite::params![col_id])?;
        Ok(())
    }

    /// Get all collection names an asset belongs to.
    pub fn collections_for_asset(&self, asset_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.name FROM collections c
             JOIN collection_assets ca ON ca.collection_id = c.id
             WHERE ca.asset_id = ?1
             ORDER BY c.name",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get all asset IDs in a collection by name (for use as search filter).
    pub fn asset_ids_for_collection(&self, name: &str) -> Result<Vec<String>> {
        let col = self.get_by_name(name)?
            .ok_or_else(|| anyhow::anyhow!("no collection named '{name}'"))?;
        self.get_asset_ids(&col.id.to_string())
    }

    /// Export all collections to a CollectionsFile for YAML persistence.
    pub fn export_all(&self) -> Result<CollectionsFile> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, created_at FROM collections ORDER BY name",
        )?;
        let mut collections = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id_str: String = row.get(0)?;
            let id: Uuid = id_str.parse().map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;
            let name: String = row.get(1)?;
            let description: Option<String> = row.get(2)?;
            let created_at_str: String = row.get(3)?;
            let created_at: DateTime<Utc> = created_at_str.parse().map_err(|e| anyhow::anyhow!("invalid date: {e}"))?;
            let asset_ids = self.get_asset_ids(&id_str)?;
            collections.push(Collection {
                id,
                name,
                description,
                created_at,
                asset_ids,
            });
        }
        Ok(CollectionsFile { collections })
    }

    /// Import collections from YAML into SQLite (used by rebuild-catalog).
    pub fn import_from_yaml(&self, file: &CollectionsFile) -> Result<u32> {
        let mut count = 0u32;
        for col in &file.collections {
            self.conn.execute(
                "INSERT OR REPLACE INTO collections (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![col.id.to_string(), col.name, col.description, col.created_at.to_rfc3339()],
            )?;
            for asset_id in &col.asset_ids {
                let _ = self.conn.execute(
                    "INSERT OR IGNORE INTO collection_assets (collection_id, asset_id, added_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![col.id.to_string(), asset_id, col.created_at.to_rfc3339()],
                );
            }
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Minimal schema for tests
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT
            );",
        )
        .unwrap();
        CollectionStore::initialize(&conn).unwrap();
        conn
    }

    fn insert_test_asset(conn: &rusqlite::Connection, id: &str) {
        conn.execute(
            "INSERT INTO assets (id, created_at, asset_type) VALUES (?1, '2026-01-01T00:00:00Z', 'image')",
            rusqlite::params![id],
        )
        .unwrap();
    }

    #[test]
    fn create_and_list() {
        let conn = setup_db();
        let store = CollectionStore::new(&conn);

        store.create("Portfolio", Some("Best shots")).unwrap();
        store.create("Favorites", None).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Favorites");
        assert_eq!(list[1].name, "Portfolio");
        assert_eq!(list[1].description.as_deref(), Some("Best shots"));
    }

    #[test]
    fn add_and_remove_assets() {
        let conn = setup_db();
        insert_test_asset(&conn, "asset-1");
        insert_test_asset(&conn, "asset-2");
        insert_test_asset(&conn, "asset-3");

        let store = CollectionStore::new(&conn);
        store.create("Test", None).unwrap();

        let added = store.add_assets("Test", &["asset-1".into(), "asset-2".into()]).unwrap();
        assert_eq!(added, 2);

        // Duplicate add
        let added = store.add_assets("Test", &["asset-1".into()]).unwrap();
        assert_eq!(added, 0);

        let list = store.list().unwrap();
        assert_eq!(list[0].asset_count, 2);

        let removed = store.remove_assets("Test", &["asset-1".into()]).unwrap();
        assert_eq!(removed, 1);

        let col = store.get_by_name("Test").unwrap().unwrap();
        assert_eq!(col.asset_ids.len(), 1);
        assert_eq!(col.asset_ids[0], "asset-2");
    }

    #[test]
    fn delete_collection() {
        let conn = setup_db();
        insert_test_asset(&conn, "asset-1");

        let store = CollectionStore::new(&conn);
        store.create("Temp", None).unwrap();
        store.add_assets("Temp", &["asset-1".into()]).unwrap();

        store.delete("Temp").unwrap();
        assert!(store.get_by_name("Temp").unwrap().is_none());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn collections_for_asset() {
        let conn = setup_db();
        insert_test_asset(&conn, "asset-1");

        let store = CollectionStore::new(&conn);
        store.create("A", None).unwrap();
        store.create("B", None).unwrap();
        store.add_assets("A", &["asset-1".into()]).unwrap();
        store.add_assets("B", &["asset-1".into()]).unwrap();

        let cols = store.collections_for_asset("asset-1").unwrap();
        assert_eq!(cols, vec!["A", "B"]);
    }

    #[test]
    fn export_and_import() {
        let conn = setup_db();
        insert_test_asset(&conn, "asset-1");

        let store = CollectionStore::new(&conn);
        store.create("Test", Some("desc")).unwrap();
        store.add_assets("Test", &["asset-1".into()]).unwrap();

        let exported = store.export_all().unwrap();
        assert_eq!(exported.collections.len(), 1);
        assert_eq!(exported.collections[0].asset_ids.len(), 1);

        // Wipe and reimport
        conn.execute_batch("DELETE FROM collection_assets; DELETE FROM collections;").unwrap();
        assert!(store.list().unwrap().is_empty());

        let imported = store.import_from_yaml(&exported).unwrap();
        assert_eq!(imported, 1);

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].asset_count, 1);
    }

    #[test]
    fn duplicate_name_errors() {
        let conn = setup_db();
        let store = CollectionStore::new(&conn);
        store.create("Dup", None).unwrap();
        assert!(store.create("Dup", None).is_err());
    }

    #[test]
    fn yaml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = CollectionsFile {
            collections: vec![Collection {
                id: Uuid::new_v4(),
                name: "Test".to_string(),
                description: Some("A test collection".to_string()),
                created_at: Utc::now(),
                asset_ids: vec!["abc".to_string(), "def".to_string()],
            }],
        };
        save_yaml(dir.path(), &file).unwrap();
        let loaded = load_yaml(dir.path()).unwrap();
        assert_eq!(loaded.collections.len(), 1);
        assert_eq!(loaded.collections[0].name, "Test");
        assert_eq!(loaded.collections[0].asset_ids.len(), 2);
    }
}
