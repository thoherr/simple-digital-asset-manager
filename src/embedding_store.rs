//! SQLite-backed storage for image embeddings.
//!
//! Stores float vectors as BLOBs for visual similarity search.
//! Composite primary key `(asset_id, model)` allows storing embeddings from
//! different models without collision.
//! Only compiled when the `ai` feature is enabled.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Store and query image embeddings in SQLite.
pub struct EmbeddingStore<'a> {
    conn: &'a Connection,
}

impl<'a> EmbeddingStore<'a> {
    /// Create a new EmbeddingStore backed by the given connection.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Initialize the embeddings table (idempotent).
    /// Migrates old single-PK schema to composite PK if needed.
    pub fn initialize(conn: &Connection) -> Result<()> {
        // Check if table exists and has old schema (asset_id TEXT PRIMARY KEY).
        let needs_migration = Self::has_old_schema(conn);

        if needs_migration {
            // Migrate: rename old table, create new, copy data, drop old.
            conn.execute_batch(
                "ALTER TABLE embeddings RENAME TO embeddings_old;
                 CREATE TABLE embeddings (
                     asset_id TEXT NOT NULL,
                     model TEXT NOT NULL DEFAULT 'siglip-vit-b16-256',
                     embedding BLOB NOT NULL,
                     PRIMARY KEY (asset_id, model)
                 );
                 INSERT INTO embeddings (asset_id, model, embedding)
                     SELECT asset_id, COALESCE(model, 'siglip-vit-b16-256'), embedding
                     FROM embeddings_old;
                 DROP TABLE embeddings_old;"
            )
            .context("Failed to migrate embeddings table to composite PK")?;
        } else {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS embeddings (
                    asset_id TEXT NOT NULL,
                    model TEXT NOT NULL DEFAULT 'siglip-vit-b16-256',
                    embedding BLOB NOT NULL,
                    PRIMARY KEY (asset_id, model)
                )",
            )
            .context("Failed to create embeddings table")?;
        }
        Ok(())
    }

    /// Check if the old schema (asset_id TEXT PRIMARY KEY, no composite) exists.
    fn has_old_schema(conn: &Connection) -> bool {
        // Table must exist
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='embeddings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !table_exists {
            return false;
        }

        // Check if PK is single-column (old schema) by looking at table_info.
        // Old schema: asset_id is pk=1, model is pk=0.
        // New schema: asset_id is pk=1, model is pk=2.
        let mut stmt = match conn.prepare("PRAGMA table_info(embeddings)") {
            Ok(s) => s,
            Err(_) => return false,
        };
        let pk_count: i32 = stmt
            .query_map([], |row| row.get::<_, i32>(5)) // column 5 = pk
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).filter(|pk| *pk > 0).count() as i32)
            .unwrap_or(0);

        pk_count == 1
    }

    /// Store an embedding for an asset with a specific model (insert or replace).
    pub fn store(&self, asset_id: &str, embedding: &[f32], model: &str) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings (asset_id, model, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![asset_id, model, blob],
        ).context("Failed to store embedding")?;
        Ok(())
    }

    /// Retrieve an embedding for an asset and model.
    pub fn get(&self, asset_id: &str, model: &str) -> Result<Option<Vec<f32>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT embedding FROM embeddings WHERE asset_id = ?1 AND model = ?2")?;
        let mut rows = stmt.query(rusqlite::params![asset_id, model])?;
        match rows.next()? {
            Some(row) => {
                let blob: Vec<u8> = row.get(0)?;
                Ok(Some(blob_to_embedding(&blob)))
            }
            None => Ok(None),
        }
    }

    /// Check if an asset has a stored embedding for a specific model.
    pub fn has_embedding(&self, asset_id: &str, model: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM embeddings WHERE asset_id = ?1 AND model = ?2",
                rusqlite::params![asset_id, model],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Count total stored embeddings.
    pub fn count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// Remove an embedding for an asset and model.
    pub fn remove(&self, asset_id: &str, model: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM embeddings WHERE asset_id = ?1 AND model = ?2",
            rusqlite::params![asset_id, model],
        )?;
        Ok(())
    }

    /// Iterate all embeddings for a given model, returning (asset_id, embedding) pairs.
    pub fn all_embeddings_for_model(&self, model: &str) -> Result<Vec<(String, Vec<f32>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT asset_id, embedding FROM embeddings WHERE model = ?1")?;
        let rows = stmt.query_map(rusqlite::params![model], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            results.push((id, blob_to_embedding(&blob)));
        }
        Ok(results)
    }

    /// List all distinct model IDs stored in the embeddings table.
    pub fn list_models(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT model FROM embeddings ORDER BY model")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut models = Vec::new();
        for row in rows {
            models.push(row?);
        }
        Ok(models)
    }

    /// Find the most similar assets by cosine similarity (brute-force scan).
    /// Only compares embeddings from the same model.
    /// Returns `(asset_id, similarity)` pairs sorted by similarity descending.
    pub fn find_similar(
        &self,
        query_emb: &[f32],
        limit: usize,
        exclude_id: Option<&str>,
        model: &str,
    ) -> Result<Vec<(String, f32)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT asset_id, embedding FROM embeddings WHERE model = ?1")?;
        let rows = stmt.query_map(rusqlite::params![model], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;

        let mut results: Vec<(String, f32)> = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            if exclude_id == Some(id.as_str()) {
                continue;
            }
            let emb = blob_to_embedding(&blob);
            let sim = crate::ai::cosine_similarity(query_emb, &emb);
            results.push((id, sim));
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        results.truncate(limit);
        Ok(results)
    }
}

/// In-memory embedding index for fast similarity search.
///
/// Stores all embeddings for a single model in a contiguous f32 buffer.
/// Since SigLIP embeddings are already L2-normalized, similarity = dot product.
/// For 100k assets × 768 dims ≈ 300MB RAM, search takes <10ms.
pub struct EmbeddingIndex {
    ids: Vec<String>,
    data: Vec<f32>, // contiguous [N × dim] row-major
    dim: usize,
}

impl EmbeddingIndex {
    /// Load all embeddings for a model from SQLite into a contiguous buffer.
    pub fn load(conn: &Connection, model: &str, dim: usize) -> Result<Self> {
        let mut stmt = conn
            .prepare("SELECT asset_id, embedding FROM embeddings WHERE model = ?1")?;
        let rows = stmt.query_map(rusqlite::params![model], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;

        let mut ids = Vec::new();
        let mut data = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let emb = blob_to_embedding(&blob);
            if emb.len() == dim {
                ids.push(id);
                data.extend_from_slice(&emb);
            }
        }
        Ok(Self { ids, data, dim })
    }

    /// Find top-K most similar by dot product (embeddings are L2-normalized).
    pub fn search(
        &self,
        query: &[f32],
        limit: usize,
        exclude_id: Option<&str>,
    ) -> Vec<(String, f32)> {
        use std::collections::BinaryHeap;
        use std::cmp::Reverse;

        let n = self.ids.len();
        if n == 0 || query.len() != self.dim {
            return Vec::new();
        }

        // Min-heap of (similarity, index) — keeps top-K
        let mut heap: BinaryHeap<Reverse<(OrderedF32, usize)>> = BinaryHeap::with_capacity(limit + 1);

        for i in 0..n {
            if exclude_id == Some(self.ids[i].as_str()) {
                continue;
            }
            let offset = i * self.dim;
            let row = &self.data[offset..offset + self.dim];
            let dot = dot_product(query, row);

            if heap.len() < limit {
                heap.push(Reverse((OrderedF32(dot), i)));
            } else if let Some(&Reverse((OrderedF32(min_sim), _))) = heap.peek() {
                if dot > min_sim {
                    heap.pop();
                    heap.push(Reverse((OrderedF32(dot), i)));
                }
            }
        }

        let mut results: Vec<(String, f32)> = heap
            .into_iter()
            .map(|Reverse((OrderedF32(sim), idx))| (self.ids[idx].clone(), sim))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Add or update an embedding in the index.
    pub fn upsert(&mut self, asset_id: &str, embedding: &[f32]) {
        if embedding.len() != self.dim {
            return;
        }
        if let Some(pos) = self.ids.iter().position(|id| id == asset_id) {
            // Update in place
            let offset = pos * self.dim;
            self.data[offset..offset + self.dim].copy_from_slice(embedding);
        } else {
            // Append
            self.ids.push(asset_id.to_string());
            self.data.extend_from_slice(embedding);
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Wrapper for f32 that implements Ord (for BinaryHeap).
#[derive(PartialEq, PartialOrd)]
struct OrderedF32(f32);

impl Eq for OrderedF32 {}

impl Ord for OrderedF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Dot product of two equal-length slices.
#[inline]
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Convert a float embedding to a byte blob (little-endian).
pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

/// Convert a byte blob back to a float embedding.
pub fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// ── SigLIP binary embedding I/O ──────────────────────────────────────

/// Compute the path for a SigLIP embedding binary file.
/// Layout: `embeddings/<model>/<2-char prefix>/<asset_id>.bin`
pub fn embedding_binary_path(
    catalog_root: &std::path::Path,
    model: &str,
    asset_id: &str,
) -> std::path::PathBuf {
    let prefix = &asset_id[..2.min(asset_id.len())];
    catalog_root
        .join("embeddings")
        .join(model)
        .join(prefix)
        .join(format!("{asset_id}.bin"))
}

/// Write an embedding as raw little-endian f32 bytes.
pub fn write_embedding_binary(
    catalog_root: &std::path::Path,
    model: &str,
    asset_id: &str,
    embedding: &[f32],
) -> Result<()> {
    let path = embedding_binary_path(catalog_root, model, asset_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = embedding_to_blob(embedding);
    std::fs::write(&path, bytes)?;
    Ok(())
}

/// Delete an embedding binary file (if it exists).
pub fn delete_embedding_binary(
    catalog_root: &std::path::Path,
    model: &str,
    asset_id: &str,
) {
    let path = embedding_binary_path(catalog_root, model, asset_id);
    let _ = std::fs::remove_file(path);
}

/// Read an embedding from a binary file.
pub fn read_embedding_binary(
    catalog_root: &std::path::Path,
    model: &str,
    asset_id: &str,
) -> Result<Option<Vec<f32>>> {
    let path = embedding_binary_path(catalog_root, model, asset_id);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    Ok(Some(blob_to_embedding(&bytes)))
}

/// Scan all embedding binaries for a given model.
/// Returns `(asset_id, embedding)` pairs.
pub fn scan_embedding_binaries(
    catalog_root: &std::path::Path,
    model: &str,
) -> Result<Vec<(String, Vec<f32>)>> {
    let base = catalog_root.join("embeddings").join(model);
    let mut results = Vec::new();
    if !base.exists() {
        return Ok(results);
    }
    for prefix_entry in std::fs::read_dir(&base)? {
        let prefix_entry = prefix_entry?;
        if !prefix_entry.file_type()?.is_dir() {
            continue;
        }
        for file_entry in std::fs::read_dir(prefix_entry.path())? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let asset_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if asset_id.is_empty() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let embedding = blob_to_embedding(&bytes);
            results.push((asset_id, embedding));
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MODEL: &str = "siglip-vit-b16-256";

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        EmbeddingStore::initialize(&conn).unwrap();
        conn
    }

    #[test]
    fn store_and_retrieve() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);

        let emb = vec![0.1f32; 768];
        store.store("asset-1", &emb, TEST_MODEL).unwrap();

        let retrieved = store.get("asset-1", TEST_MODEL).unwrap().unwrap();
        assert_eq!(retrieved.len(), 768);
        assert!((retrieved[0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn get_nonexistent() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        assert!(store.get("nonexistent", TEST_MODEL).unwrap().is_none());
    }

    #[test]
    fn has_embedding_true() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("asset-1", &vec![0.0; 768], TEST_MODEL).unwrap();
        assert!(store.has_embedding("asset-1", TEST_MODEL));
    }

    #[test]
    fn has_embedding_false() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        assert!(!store.has_embedding("asset-1", TEST_MODEL));
    }

    #[test]
    fn count_empty() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn count_after_insert() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("a", &vec![0.0; 768], TEST_MODEL).unwrap();
        store.store("b", &vec![0.0; 768], TEST_MODEL).unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn remove_embedding() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("asset-1", &vec![0.0; 768], TEST_MODEL).unwrap();
        store.remove("asset-1", TEST_MODEL).unwrap();
        assert!(!store.has_embedding("asset-1", TEST_MODEL));
    }

    #[test]
    fn store_replaces_existing() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("asset-1", &vec![1.0; 768], TEST_MODEL).unwrap();
        store.store("asset-1", &vec![2.0; 768], TEST_MODEL).unwrap();

        let retrieved = store.get("asset-1", TEST_MODEL).unwrap().unwrap();
        assert!((retrieved[0] - 2.0).abs() < 1e-6);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn find_similar_basic() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);

        let base = vec![1.0f32; 768];
        let similar = vec![0.9f32; 768];
        let mut different = vec![0.0f32; 768];
        different[0] = 1.0;

        store.store("base", &base, TEST_MODEL).unwrap();
        store.store("similar", &similar, TEST_MODEL).unwrap();
        store.store("different", &different, TEST_MODEL).unwrap();

        let results = store.find_similar(&base, 2, Some("base"), TEST_MODEL).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "similar");
        assert!(results[0].1 > results[1].1);
    }

    #[test]
    fn find_similar_respects_limit() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);

        for i in 0..10 {
            store
                .store(&format!("asset-{i}"), &vec![i as f32; 768], TEST_MODEL)
                .unwrap();
        }

        let query = vec![5.0f32; 768];
        let results = store.find_similar(&query, 3, None, TEST_MODEL).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn multi_model_embeddings() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);

        let emb_b = vec![1.0f32; 768];
        let emb_l = vec![2.0f32; 1024];

        store.store("asset-1", &emb_b, "siglip-vit-b16-256").unwrap();
        store.store("asset-1", &emb_l, "siglip-vit-l16-256").unwrap();

        // Both stored — count is 2
        assert_eq!(store.count(), 2);

        // Each model returns its own embedding
        let got_b = store.get("asset-1", "siglip-vit-b16-256").unwrap().unwrap();
        assert_eq!(got_b.len(), 768);
        assert!((got_b[0] - 1.0).abs() < 1e-6);

        let got_l = store.get("asset-1", "siglip-vit-l16-256").unwrap().unwrap();
        assert_eq!(got_l.len(), 1024);
        assert!((got_l[0] - 2.0).abs() < 1e-6);

        // has_embedding is model-specific
        assert!(store.has_embedding("asset-1", "siglip-vit-b16-256"));
        assert!(store.has_embedding("asset-1", "siglip-vit-l16-256"));
        assert!(!store.has_embedding("asset-1", "nonexistent-model"));

        // find_similar is model-scoped
        store.store("asset-2", &vec![0.9f32; 768], "siglip-vit-b16-256").unwrap();
        let results = store.find_similar(&emb_b, 10, None, "siglip-vit-b16-256").unwrap();
        // Should find asset-1 and asset-2 (both b16), not the l16 embedding
        assert_eq!(results.len(), 2);

        // Remove is model-specific
        store.remove("asset-1", "siglip-vit-b16-256").unwrap();
        assert!(!store.has_embedding("asset-1", "siglip-vit-b16-256"));
        assert!(store.has_embedding("asset-1", "siglip-vit-l16-256"));
    }

    #[test]
    fn migrate_old_schema() {
        let conn = Connection::open_in_memory().unwrap();

        // Create old-style table with single PK
        conn.execute_batch(
            "CREATE TABLE embeddings (
                asset_id TEXT PRIMARY KEY,
                embedding BLOB NOT NULL,
                model TEXT NOT NULL DEFAULT 'siglip-vit-b16-256'
            )"
        ).unwrap();

        // Insert some data
        let emb = embedding_to_blob(&vec![0.5f32; 768]);
        conn.execute(
            "INSERT INTO embeddings (asset_id, embedding) VALUES ('a1', ?1)",
            rusqlite::params![emb],
        ).unwrap();

        // Run initialize — should migrate
        EmbeddingStore::initialize(&conn).unwrap();

        // Data should be preserved
        let store = EmbeddingStore::new(&conn);
        let got = store.get("a1", "siglip-vit-b16-256").unwrap().unwrap();
        assert_eq!(got.len(), 768);
        assert!((got[0] - 0.5).abs() < 1e-6);

        // Should now support multi-model
        store.store("a1", &vec![1.0; 1024], "siglip-vit-l16-256").unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn blob_round_trip() {
        let original = vec![1.5f32, -2.3, 0.0, std::f32::consts::PI];
        let blob = embedding_to_blob(&original);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(original, recovered);
    }

    #[test]
    fn index_load_and_search() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);

        // Store some normalized embeddings (dim=4 for simplicity)
        let a = vec![1.0f32, 0.0, 0.0, 0.0];
        let mut b = vec![0.9, 0.1, 0.0, 0.0];
        let c = vec![0.0, 0.0, 0.0, 1.0];
        // Normalize
        let norm_b = (b.iter().map(|x| x * x).sum::<f32>()).sqrt();
        b.iter_mut().for_each(|x| *x /= norm_b);

        store.store("a", &a, TEST_MODEL).unwrap();
        store.store("b", &b, TEST_MODEL).unwrap();
        store.store("c", &c, TEST_MODEL).unwrap();

        let index = EmbeddingIndex::load(&conn, TEST_MODEL, 4).unwrap();
        assert_eq!(index.len(), 3);

        let results = index.search(&a, 2, Some("a"));
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "b"); // most similar to a
        assert!(results[0].1 > results[1].1);
    }

    #[test]
    fn index_upsert() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("a", &vec![1.0, 0.0, 0.0], TEST_MODEL).unwrap();

        let mut index = EmbeddingIndex::load(&conn, TEST_MODEL, 3).unwrap();
        assert_eq!(index.len(), 1);

        // Insert new
        index.upsert("b", &[0.0, 1.0, 0.0]);
        assert_eq!(index.len(), 2);

        // Update existing
        index.upsert("a", &[0.0, 0.0, 1.0]);
        assert_eq!(index.len(), 2);

        // Search should reflect update
        let results = index.search(&[0.0, 0.0, 1.0], 2, None);
        assert_eq!(results[0].0, "a");
    }

    #[test]
    fn index_search_empty() {
        let conn = setup_db();
        let index = EmbeddingIndex::load(&conn, TEST_MODEL, 768).unwrap();
        assert!(index.is_empty());
        let results = index.search(&vec![1.0; 768], 10, None);
        assert!(results.is_empty());
    }

    #[test]
    fn index_skips_wrong_dim() {
        let conn = setup_db();
        let store = EmbeddingStore::new(&conn);
        store.store("a", &vec![1.0; 768], TEST_MODEL).unwrap();
        store.store("b", &vec![1.0; 1024], TEST_MODEL).unwrap(); // wrong dim for 768 index

        let index = EmbeddingIndex::load(&conn, TEST_MODEL, 768).unwrap();
        assert_eq!(index.len(), 1); // only 'a' loaded
    }

    #[test]
    fn embedding_binary_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let embedding = vec![0.1f32, 0.2, 0.3, -0.5, std::f32::consts::PI];

        write_embedding_binary(dir.path(), TEST_MODEL, "asset-abc123", &embedding).unwrap();

        // Verify file exists at expected path
        let path = embedding_binary_path(dir.path(), TEST_MODEL, "asset-abc123");
        assert!(path.exists());
        assert!(path.to_str().unwrap().contains(&format!("{TEST_MODEL}/as/asset-abc123.bin")));

        // Read it back
        let loaded = read_embedding_binary(dir.path(), TEST_MODEL, "asset-abc123").unwrap().unwrap();
        assert_eq!(loaded, embedding);

        // Delete
        delete_embedding_binary(dir.path(), TEST_MODEL, "asset-abc123");
        assert!(!path.exists());
        assert!(read_embedding_binary(dir.path(), TEST_MODEL, "asset-abc123").unwrap().is_none());
    }

    #[test]
    fn scan_embedding_binaries_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let emb1 = vec![1.0f32; 768];
        let emb2 = vec![2.0f32; 768];

        write_embedding_binary(dir.path(), TEST_MODEL, "asset-aaa", &emb1).unwrap();
        write_embedding_binary(dir.path(), TEST_MODEL, "asset-bbb", &emb2).unwrap();

        let entries = scan_embedding_binaries(dir.path(), TEST_MODEL).unwrap();
        assert_eq!(entries.len(), 2);

        let ids: std::collections::HashSet<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains("asset-aaa"));
        assert!(ids.contains("asset-bbb"));
    }

    #[test]
    fn scan_embedding_binaries_empty() {
        let dir = tempfile::tempdir().unwrap();
        let entries = scan_embedding_binaries(dir.path(), TEST_MODEL).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn scan_embedding_binaries_ignores_other_models() {
        let dir = tempfile::tempdir().unwrap();
        write_embedding_binary(dir.path(), TEST_MODEL, "asset-1", &vec![1.0; 768]).unwrap();
        write_embedding_binary(dir.path(), "other-model", "asset-2", &vec![2.0; 512]).unwrap();

        let entries = scan_embedding_binaries(dir.path(), TEST_MODEL).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "asset-1");
    }
}
