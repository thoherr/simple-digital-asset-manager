use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stack (scene grouping) — a set of assets collapsed into one "pick" in the browse grid.
/// Anonymous (no name/description), position-based ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stack {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    /// Ordered list of asset IDs — index 0 is the pick.
    pub asset_ids: Vec<String>,
}

/// Summary of a stack for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackSummary {
    pub id: String,
    pub member_count: u64,
    pub created_at: String,
    pub pick_asset_id: Option<String>,
}

/// Wrapper for the YAML file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StacksFile {
    pub stacks: Vec<Stack>,
}

const FILENAME: &str = "stacks.yaml";

/// Load stacks from the YAML file. Returns empty list if file doesn't exist.
pub fn load_yaml(catalog_root: &Path) -> Result<StacksFile> {
    let path = catalog_root.join(FILENAME);
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let file: StacksFile = serde_yaml::from_str(&contents)?;
        Ok(file)
    } else {
        Ok(StacksFile::default())
    }
}

/// Save stacks to the YAML file.
pub fn save_yaml(catalog_root: &Path, file: &StacksFile) -> Result<()> {
    let path = catalog_root.join(FILENAME);
    let contents = serde_yaml::to_string(file)?;
    std::fs::write(path, contents)?;
    Ok(())
}

/// Stack operations backed by SQLite catalog.
pub struct StackStore<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> StackStore<'a> {
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Create stacks table (called from Catalog::initialize).
    pub fn initialize(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stacks (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                member_count INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        Ok(())
    }

    /// Create a new stack from the given asset IDs.
    /// The first asset becomes the pick (position 0).
    pub fn create(&self, asset_ids: &[String]) -> Result<Stack> {
        if asset_ids.len() < 2 {
            anyhow::bail!("a stack requires at least 2 assets");
        }

        // Check none are already stacked
        for id in asset_ids {
            let existing: Option<String> = self.conn.query_row(
                "SELECT stack_id FROM assets WHERE id = ?1 AND stack_id IS NOT NULL",
                rusqlite::params![id],
                |row| row.get(0),
            ).ok();
            if let Some(sid) = existing {
                anyhow::bail!("asset {id} is already in stack {sid}");
            }
        }

        let id = Uuid::new_v4();
        let now = Utc::now();
        let member_count = asset_ids.len() as i64;

        self.conn.execute(
            "INSERT INTO stacks (id, created_at, member_count) VALUES (?1, ?2, ?3)",
            rusqlite::params![id.to_string(), now.to_rfc3339(), member_count],
        )?;

        for (pos, asset_id) in asset_ids.iter().enumerate() {
            self.conn.execute(
                "UPDATE assets SET stack_id = ?1, stack_position = ?2 WHERE id = ?3",
                rusqlite::params![id.to_string(), pos as i64, asset_id],
            )?;
        }

        Ok(Stack {
            id,
            created_at: now,
            asset_ids: asset_ids.to_vec(),
        })
    }

    /// Add assets to an existing stack (identified by any current member).
    /// Returns the number of assets actually added.
    pub fn add(&self, reference_asset_id: &str, new_asset_ids: &[String]) -> Result<u32> {
        let (stack_id, mut members) = self.stack_for_asset(reference_asset_id)?
            .ok_or_else(|| anyhow::anyhow!("asset {reference_asset_id} is not in a stack"))?;

        let mut added = 0u32;
        for id in new_asset_ids {
            // Check not already stacked
            let existing: Option<String> = self.conn.query_row(
                "SELECT stack_id FROM assets WHERE id = ?1 AND stack_id IS NOT NULL",
                rusqlite::params![id],
                |row| row.get(0),
            ).ok();
            if existing.is_some() {
                eprintln!("Warning: asset {id} is already in a stack, skipping");
                continue;
            }

            let pos = members.len() as i64;
            self.conn.execute(
                "UPDATE assets SET stack_id = ?1, stack_position = ?2 WHERE id = ?3",
                rusqlite::params![stack_id, pos, id],
            )?;
            members.push(id.clone());
            added += 1;
        }

        if added > 0 {
            self.conn.execute(
                "UPDATE stacks SET member_count = ?1 WHERE id = ?2",
                rusqlite::params![members.len() as i64, stack_id],
            )?;
        }

        Ok(added)
    }

    /// Remove assets from their stack. Dissolves the stack if ≤1 member remains.
    /// Returns the number of assets actually removed.
    pub fn remove(&self, asset_ids: &[String]) -> Result<u32> {
        let mut removed = 0u32;
        // Group by stack_id to handle dissolves
        let mut affected_stacks: std::collections::HashSet<String> = std::collections::HashSet::new();

        for id in asset_ids {
            let stack_id: Option<String> = self.conn.query_row(
                "SELECT stack_id FROM assets WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            ).ok().flatten();

            if let Some(ref sid) = stack_id {
                self.conn.execute(
                    "UPDATE assets SET stack_id = NULL, stack_position = NULL WHERE id = ?1",
                    rusqlite::params![id],
                )?;
                affected_stacks.insert(sid.clone());
                removed += 1;
            }
        }

        // For each affected stack, renumber positions and possibly dissolve
        for sid in &affected_stacks {
            self.renumber_and_maybe_dissolve(sid)?;
        }

        Ok(removed)
    }

    /// Set the pick of a stack (move this asset to position 0).
    pub fn set_pick(&self, asset_id: &str) -> Result<()> {
        let (stack_id, members) = self.stack_for_asset(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("asset {asset_id} is not in a stack"))?;

        let current_pos = members.iter().position(|id| id == asset_id)
            .ok_or_else(|| anyhow::anyhow!("asset {asset_id} not found in stack members"))?;

        if current_pos == 0 {
            return Ok(()); // Already the pick
        }

        // Swap positions: current pick (pos 0) gets this asset's position
        let current_pick = &members[0];
        self.conn.execute(
            "UPDATE assets SET stack_position = ?1 WHERE id = ?2 AND stack_id = ?3",
            rusqlite::params![current_pos as i64, current_pick, stack_id],
        )?;
        self.conn.execute(
            "UPDATE assets SET stack_position = 0 WHERE id = ?1 AND stack_id = ?2",
            rusqlite::params![asset_id, stack_id],
        )?;

        Ok(())
    }

    /// Dissolve an entire stack (unstack all members).
    pub fn dissolve(&self, asset_id: &str) -> Result<()> {
        let (stack_id, _) = self.stack_for_asset(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("asset {asset_id} is not in a stack"))?;

        self.conn.execute(
            "UPDATE assets SET stack_id = NULL, stack_position = NULL WHERE stack_id = ?1",
            rusqlite::params![stack_id],
        )?;
        self.conn.execute(
            "DELETE FROM stacks WHERE id = ?1",
            rusqlite::params![stack_id],
        )?;

        Ok(())
    }

    /// List all stacks with summary info.
    pub fn list(&self) -> Result<Vec<StackSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.member_count, s.created_at,
                    (SELECT a.id FROM assets a WHERE a.stack_id = s.id AND a.stack_position = 0 LIMIT 1)
             FROM stacks s ORDER BY s.created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StackSummary {
                id: row.get(0)?,
                member_count: row.get::<_, i64>(1)? as u64,
                created_at: row.get(2)?,
                pick_asset_id: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get the stack and ordered members for an asset. Returns None if not stacked.
    pub fn stack_for_asset(&self, asset_id: &str) -> Result<Option<(String, Vec<String>)>> {
        let stack_id: Option<String> = self.conn.query_row(
            "SELECT stack_id FROM assets WHERE id = ?1",
            rusqlite::params![asset_id],
            |row| row.get(0),
        ).ok().flatten();

        match stack_id {
            Some(sid) => {
                let members = self.ordered_members(&sid)?;
                Ok(Some((sid, members)))
            }
            None => Ok(None),
        }
    }

    /// Get ordered member asset IDs for a stack.
    pub fn ordered_members(&self, stack_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM assets WHERE stack_id = ?1 ORDER BY stack_position",
        )?;
        let rows = stmt.query_map(rusqlite::params![stack_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Renumber positions after removal. Dissolves if ≤1 member remains.
    fn renumber_and_maybe_dissolve(&self, stack_id: &str) -> Result<()> {
        let members = self.ordered_members(stack_id)?;

        if members.len() <= 1 {
            // Dissolve
            self.conn.execute(
                "UPDATE assets SET stack_id = NULL, stack_position = NULL WHERE stack_id = ?1",
                rusqlite::params![stack_id],
            )?;
            self.conn.execute(
                "DELETE FROM stacks WHERE id = ?1",
                rusqlite::params![stack_id],
            )?;
            return Ok(());
        }

        // Renumber
        for (pos, id) in members.iter().enumerate() {
            self.conn.execute(
                "UPDATE assets SET stack_position = ?1 WHERE id = ?2 AND stack_id = ?3",
                rusqlite::params![pos as i64, id, stack_id],
            )?;
        }
        self.conn.execute(
            "UPDATE stacks SET member_count = ?1 WHERE id = ?2",
            rusqlite::params![members.len() as i64, stack_id],
        )?;

        Ok(())
    }

    /// Export all stacks to a StacksFile for YAML persistence.
    pub fn export_all(&self) -> Result<StacksFile> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at FROM stacks ORDER BY created_at",
        )?;
        let mut stacks = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id_str: String = row.get(0)?;
            let id: Uuid = id_str.parse().map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;
            let created_at_str: String = row.get(1)?;
            let created_at: DateTime<Utc> = created_at_str.parse().map_err(|e| anyhow::anyhow!("invalid date: {e}"))?;
            let asset_ids = self.ordered_members(&id_str)?;
            stacks.push(Stack {
                id,
                created_at,
                asset_ids,
            });
        }
        Ok(StacksFile { stacks })
    }

    /// Import stacks from YAML into SQLite (used by rebuild-catalog).
    pub fn import_from_yaml(&self, file: &StacksFile) -> Result<u32> {
        let mut count = 0u32;
        for stack in &file.stacks {
            let stack_id = stack.id.to_string();
            self.conn.execute(
                "INSERT OR REPLACE INTO stacks (id, created_at, member_count) VALUES (?1, ?2, ?3)",
                rusqlite::params![stack_id, stack.created_at.to_rfc3339(), stack.asset_ids.len() as i64],
            )?;
            for (pos, asset_id) in stack.asset_ids.iter().enumerate() {
                let _ = self.conn.execute(
                    "UPDATE assets SET stack_id = ?1, stack_position = ?2 WHERE id = ?3",
                    rusqlite::params![stack_id, pos as i64, asset_id],
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
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                stack_id TEXT,
                stack_position INTEGER
            );",
        )
        .unwrap();
        StackStore::initialize(&conn).unwrap();
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
    fn create_stack() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        let stack = store.create(&["a1".into(), "a2".into(), "a3".into()]).unwrap();

        assert_eq!(stack.asset_ids, vec!["a1", "a2", "a3"]);

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].member_count, 3);
        assert_eq!(list[0].pick_asset_id.as_deref(), Some("a1"));
    }

    #[test]
    fn create_stack_too_few() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");

        let store = StackStore::new(&conn);
        assert!(store.create(&["a1".into()]).is_err());
    }

    #[test]
    fn create_stack_already_stacked() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into()]).unwrap();

        let err = store.create(&["a2".into(), "a3".into()]).unwrap_err();
        assert!(err.to_string().contains("already in stack"));
    }

    #[test]
    fn set_pick() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into(), "a3".into()]).unwrap();

        store.set_pick("a2").unwrap();

        let (_, members) = store.stack_for_asset("a2").unwrap().unwrap();
        assert_eq!(members[0], "a2");
        assert_eq!(members[1], "a1");
        assert_eq!(members[2], "a3");
    }

    #[test]
    fn set_pick_already_pick_is_noop() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into()]).unwrap();

        store.set_pick("a1").unwrap(); // already pick
        let (_, members) = store.stack_for_asset("a1").unwrap().unwrap();
        assert_eq!(members[0], "a1");
    }

    #[test]
    fn remove_from_stack() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into(), "a3".into()]).unwrap();

        let removed = store.remove(&["a2".into()]).unwrap();
        assert_eq!(removed, 1);

        let (_, members) = store.stack_for_asset("a1").unwrap().unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members, vec!["a1", "a3"]);

        // a2 should not be in a stack
        assert!(store.stack_for_asset("a2").unwrap().is_none());
    }

    #[test]
    fn remove_dissolves_when_single() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into()]).unwrap();

        store.remove(&["a1".into()]).unwrap();

        // Stack should be dissolved — a2 not in a stack anymore
        assert!(store.stack_for_asset("a2").unwrap().is_none());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn dissolve_stack() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into(), "a3".into()]).unwrap();

        store.dissolve("a2").unwrap();

        assert!(store.stack_for_asset("a1").unwrap().is_none());
        assert!(store.stack_for_asset("a2").unwrap().is_none());
        assert!(store.stack_for_asset("a3").unwrap().is_none());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn add_to_stack() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");
        insert_test_asset(&conn, "a3");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into()]).unwrap();

        let added = store.add("a1", &["a3".into()]).unwrap();
        assert_eq!(added, 1);

        let (_, members) = store.stack_for_asset("a1").unwrap().unwrap();
        assert_eq!(members.len(), 3);
        assert_eq!(members, vec!["a1", "a2", "a3"]);

        let list = store.list().unwrap();
        assert_eq!(list[0].member_count, 3);
    }

    #[test]
    fn export_and_import() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");
        insert_test_asset(&conn, "a2");

        let store = StackStore::new(&conn);
        store.create(&["a1".into(), "a2".into()]).unwrap();

        let exported = store.export_all().unwrap();
        assert_eq!(exported.stacks.len(), 1);
        assert_eq!(exported.stacks[0].asset_ids, vec!["a1", "a2"]);

        // Wipe and reimport
        conn.execute_batch("UPDATE assets SET stack_id = NULL, stack_position = NULL; DELETE FROM stacks;").unwrap();
        assert!(store.list().unwrap().is_empty());

        let imported = store.import_from_yaml(&exported).unwrap();
        assert_eq!(imported, 1);

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].member_count, 2);

        let (_, members) = store.stack_for_asset("a1").unwrap().unwrap();
        assert_eq!(members, vec!["a1", "a2"]);
    }

    #[test]
    fn yaml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = StacksFile {
            stacks: vec![Stack {
                id: Uuid::new_v4(),
                created_at: Utc::now(),
                asset_ids: vec!["abc".to_string(), "def".to_string()],
            }],
        };
        save_yaml(dir.path(), &file).unwrap();
        let loaded = load_yaml(dir.path()).unwrap();
        assert_eq!(loaded.stacks.len(), 1);
        assert_eq!(loaded.stacks[0].asset_ids.len(), 2);
    }

    #[test]
    fn dissolve_not_stacked_errors() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");

        let store = StackStore::new(&conn);
        assert!(store.dissolve("a1").is_err());
    }

    #[test]
    fn stack_for_asset_not_stacked() {
        let conn = setup_db();
        insert_test_asset(&conn, "a1");

        let store = StackStore::new(&conn);
        assert!(store.stack_for_asset("a1").unwrap().is_none());
    }
}
