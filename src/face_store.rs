//! SQLite-backed storage for face detections and people.
//!
//! Stores detected faces (bounding boxes, embeddings) and named people
//! for face recognition across assets.
//! Only compiled when the `ai` feature is enabled.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Store and query face detections and people in SQLite.
pub struct FaceStore<'a> {
    conn: &'a Connection,
}

/// A detected face stored in the database.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredFace {
    pub id: String,
    pub asset_id: String,
    pub person_id: Option<String>,
    pub bbox_x: f32,
    pub bbox_y: f32,
    pub bbox_w: f32,
    pub bbox_h: f32,
    pub confidence: f32,
    pub created_at: String,
}

/// A named person.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Person {
    pub id: String,
    pub name: Option<String>,
    pub representative_face_id: Option<String>,
    pub created_at: String,
}

/// Result of auto-clustering faces into people groups.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoClusterResult {
    pub people_created: u32,
    pub faces_assigned: u32,
    pub singletons_skipped: u32,
}

/// A face with its similarity score (from search).
#[derive(Debug, Clone)]
pub struct FaceMatch {
    pub face_id: String,
    pub person_id: Option<String>,
    pub asset_id: String,
    pub similarity: f32,
}

impl<'a> FaceStore<'a> {
    /// Create a new FaceStore backed by the given connection.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Access the underlying database connection.
    pub fn conn(&self) -> &Connection {
        self.conn
    }

    /// Initialize the faces and people tables (idempotent).
    pub fn initialize(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS people (
                id TEXT PRIMARY KEY,
                name TEXT,
                representative_face_id TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS faces (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL,
                person_id TEXT,
                bbox_x REAL NOT NULL,
                bbox_y REAL NOT NULL,
                bbox_w REAL NOT NULL,
                bbox_h REAL NOT NULL,
                embedding BLOB NOT NULL,
                confidence REAL NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (asset_id) REFERENCES assets(id),
                FOREIGN KEY (person_id) REFERENCES people(id)
            );

            CREATE INDEX IF NOT EXISTS idx_faces_asset ON faces(asset_id);
            CREATE INDEX IF NOT EXISTS idx_faces_person ON faces(person_id);",
        )
        .context("Failed to create faces/people tables")?;
        Ok(())
    }

    /// Store a detected face with its embedding.
    pub fn store_face(
        &self,
        id: &str,
        asset_id: &str,
        bbox_x: f32,
        bbox_y: f32,
        bbox_w: f32,
        bbox_h: f32,
        embedding: &[f32],
        confidence: f32,
    ) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO faces (id, asset_id, person_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, confidence, created_at)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![id, asset_id, bbox_x, bbox_y, bbox_w, bbox_h, blob, confidence, now],
        )?;
        Ok(())
    }

    /// Get all faces for a given asset.
    pub fn faces_for_asset(&self, asset_id: &str) -> Result<Vec<StoredFace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, asset_id, person_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, created_at
             FROM faces WHERE asset_id = ?1 ORDER BY bbox_x",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            Ok(StoredFace {
                id: row.get(0)?,
                asset_id: row.get(1)?,
                person_id: row.get(2)?,
                bbox_x: row.get(3)?,
                bbox_y: row.get(4)?,
                bbox_w: row.get(5)?,
                bbox_h: row.get(6)?,
                confidence: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to read faces")
    }

    /// Get a face by ID, including its embedding.
    pub fn get_face_embedding(&self, face_id: &str) -> Result<Option<Vec<f32>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT embedding FROM faces WHERE id = ?1")?;
        let mut rows = stmt.query(rusqlite::params![face_id])?;
        match rows.next()? {
            Some(row) => {
                let blob: Vec<u8> = row.get(0)?;
                Ok(Some(blob_to_embedding(&blob)))
            }
            None => Ok(None),
        }
    }

    /// Check if an asset already has detected faces.
    pub fn has_faces(&self, asset_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM faces WHERE asset_id = ?1 LIMIT 1",
                rusqlite::params![asset_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Count faces for an asset.
    pub fn face_count(&self, asset_id: &str) -> usize {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM faces WHERE asset_id = ?1",
                rusqlite::params![asset_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count total faces in the database.
    pub fn total_faces(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM faces", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// Count total people in the database.
    pub fn total_people(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM people", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// Find faces similar to a query embedding using cosine similarity.
    /// Returns matches above the threshold, sorted by similarity descending.
    pub fn find_similar_faces(
        &self,
        query_emb: &[f32],
        threshold: f32,
        limit: usize,
        exclude_face_id: Option<&str>,
    ) -> Result<Vec<FaceMatch>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, asset_id, person_id, embedding FROM faces")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let asset_id: String = row.get(1)?;
            let person_id: Option<String> = row.get(2)?;
            let blob: Vec<u8> = row.get(3)?;
            Ok((id, asset_id, person_id, blob))
        })?;

        let mut results: Vec<FaceMatch> = Vec::new();
        for row in rows {
            let (id, asset_id, person_id, blob) = row?;
            if exclude_face_id == Some(id.as_str()) {
                continue;
            }
            let emb = blob_to_embedding(&blob);
            let sim = crate::ai::cosine_similarity(query_emb, &emb);
            if sim >= threshold {
                results.push(FaceMatch {
                    face_id: id,
                    person_id,
                    asset_id,
                    similarity: sim,
                });
            }
        }

        results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    /// Assign a face to a person.
    pub fn assign_face_to_person(&self, face_id: &str, person_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE faces SET person_id = ?1 WHERE id = ?2",
            rusqlite::params![person_id, face_id],
        )?;
        Ok(())
    }

    /// Create a new person. Returns the person ID.
    pub fn create_person(&self, name: Option<&str>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO people (id, name, representative_face_id, created_at)
             VALUES (?1, ?2, NULL, ?3)",
            rusqlite::params![id, name, now],
        )?;
        Ok(id)
    }

    /// Name (or rename) a person.
    pub fn name_person(&self, person_id: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE people SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, person_id],
        )?;
        Ok(())
    }

    /// Set the representative face for a person.
    pub fn set_representative_face(&self, person_id: &str, face_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE people SET representative_face_id = ?1 WHERE id = ?2",
            rusqlite::params![face_id, person_id],
        )?;
        Ok(())
    }

    /// Get a person by ID.
    pub fn get_person(&self, person_id: &str) -> Result<Option<Person>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, representative_face_id, created_at FROM people WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![person_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Person {
                id: row.get(0)?,
                name: row.get(1)?,
                representative_face_id: row.get(2)?,
                created_at: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    /// List all people with optional face counts.
    pub fn list_people(&self) -> Result<Vec<(Person, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, p.representative_face_id, p.created_at,
                    (SELECT COUNT(*) FROM faces f WHERE f.person_id = p.id)
             FROM people p ORDER BY p.name NULLS LAST, p.created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                Person {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    representative_face_id: row.get(2)?,
                    created_at: row.get(3)?,
                },
                row.get::<_, usize>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to list people")
    }

    /// Merge two people: reassign all faces from source to target, delete source.
    pub fn merge_people(&self, target_id: &str, source_id: &str) -> Result<u32> {
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM faces WHERE person_id = ?1",
            rusqlite::params![source_id],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "UPDATE faces SET person_id = ?1 WHERE person_id = ?2",
            rusqlite::params![target_id, source_id],
        )?;

        self.conn.execute(
            "DELETE FROM people WHERE id = ?1",
            rusqlite::params![source_id],
        )?;

        Ok(count)
    }

    /// Delete all faces for an asset (e.g., before re-detection).
    pub fn delete_faces_for_asset(&self, asset_id: &str) -> Result<u32> {
        let count = self.conn.execute(
            "DELETE FROM faces WHERE asset_id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(count as u32)
    }

    /// Delete a person and unassign their faces.
    pub fn delete_person(&self, person_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE faces SET person_id = NULL WHERE person_id = ?1",
            rusqlite::params![person_id],
        )?;
        self.conn.execute(
            "DELETE FROM people WHERE id = ?1",
            rusqlite::params![person_id],
        )?;
        Ok(())
    }

    /// Get all face embeddings grouped by person (for clustering).
    /// Returns (face_id, person_id, embedding) tuples.
    pub fn all_face_embeddings(&self) -> Result<Vec<(String, Option<String>, Vec<f32>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, person_id, embedding FROM faces")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let person_id: Option<String> = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            Ok((id, person_id, blob))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (id, person_id, blob) = row?;
            let emb = blob_to_embedding(&blob);
            result.push((id, person_id, emb));
        }
        Ok(result)
    }

    /// Unassign a face from its person (set person_id = NULL).
    pub fn unassign_face(&self, face_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE faces SET person_id = NULL WHERE id = ?1",
            rusqlite::params![face_id],
        )?;
        Ok(())
    }

    /// Get a single face by ID (without embedding).
    pub fn get_face(&self, face_id: &str) -> Result<Option<StoredFace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, asset_id, person_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, created_at
             FROM faces WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![face_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(StoredFace {
                id: row.get(0)?,
                asset_id: row.get(1)?,
                person_id: row.get(2)?,
                bbox_x: row.get(3)?,
                bbox_y: row.get(4)?,
                bbox_w: row.get(5)?,
                bbox_h: row.get(6)?,
                confidence: row.get(7)?,
                created_at: row.get(8)?,
            })),
            None => Ok(None),
        }
    }

    /// Find asset IDs that have faces assigned to a person (by name).
    pub fn find_person_asset_ids(&self, name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT f.asset_id FROM faces f
             JOIN people p ON f.person_id = p.id
             WHERE p.name = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![name], |row| row.get(0))?;
        rows.collect::<Result<Vec<String>, _>>().context("Failed to find person asset IDs")
    }

    /// Find asset IDs that have faces assigned to a person (by person ID).
    pub fn find_person_asset_ids_by_id(&self, person_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT f.asset_id FROM faces f WHERE f.person_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![person_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<String>, _>>().context("Failed to find person asset IDs")
    }

    /// Cluster unassigned faces into groups using greedy single-linkage.
    ///
    /// Returns clusters (each is a list of face_ids) where each cluster has ≥2 faces.
    pub fn cluster_faces(&self, threshold: f32) -> Result<Vec<Vec<String>>> {
        let all = self.all_face_embeddings()?;
        // Only cluster unassigned faces
        let unassigned: Vec<(String, Vec<f32>)> = all
            .into_iter()
            .filter(|(_, pid, _)| pid.is_none())
            .map(|(id, _, emb)| (id, emb))
            .collect();

        if unassigned.is_empty() {
            return Ok(Vec::new());
        }

        // Greedy clustering: each cluster has a centroid (average embedding)
        let mut clusters: Vec<(Vec<String>, Vec<f32>)> = Vec::new(); // (face_ids, centroid)

        for (face_id, emb) in &unassigned {
            let mut best_idx = None;
            let mut best_sim = threshold;

            for (i, (_, centroid)) in clusters.iter().enumerate() {
                let sim = crate::ai::cosine_similarity(emb, centroid);
                if sim > best_sim {
                    best_sim = sim;
                    best_idx = Some(i);
                }
            }

            if let Some(idx) = best_idx {
                // Add to existing cluster, update centroid (running average)
                let (ref mut ids, ref mut centroid) = clusters[idx];
                let n = ids.len() as f32;
                for (j, val) in emb.iter().enumerate() {
                    centroid[j] = (centroid[j] * n + val) / (n + 1.0);
                }
                ids.push(face_id.clone());
            } else {
                // Start a new cluster
                clusters.push((vec![face_id.clone()], emb.clone()));
            }
        }

        // Return only clusters with ≥2 faces
        Ok(clusters
            .into_iter()
            .filter(|(ids, _)| ids.len() >= 2)
            .map(|(ids, _)| ids)
            .collect())
    }

    /// Auto-cluster unassigned faces and create people for each cluster.
    pub fn auto_cluster(&self, threshold: f32) -> Result<AutoClusterResult> {
        let clusters = self.cluster_faces(threshold)?;

        let mut people_created = 0u32;
        let mut faces_assigned = 0u32;

        // Count unassigned singletons (faces not in any cluster with ≥2 members)
        let total_unassigned: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM faces WHERE person_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let clustered_count: usize = clusters.iter().map(|c| c.len()).sum();
        let singletons_skipped = (total_unassigned - clustered_count) as u32;

        for face_ids in &clusters {
            let person_id = self.create_person(None)?;

            // Find the highest-confidence face for representative
            let mut best_face_id = &face_ids[0];
            let mut best_confidence = 0.0f32;

            for fid in face_ids {
                self.assign_face_to_person(fid, &person_id)?;
                faces_assigned += 1;

                // Get confidence for representative selection
                let conf: f32 = self
                    .conn
                    .query_row(
                        "SELECT confidence FROM faces WHERE id = ?1",
                        rusqlite::params![fid],
                        |row| row.get(0),
                    )
                    .unwrap_or(0.0);
                if conf > best_confidence {
                    best_confidence = conf;
                    best_face_id = fid;
                }
            }

            self.set_representative_face(&person_id, best_face_id)?;
            people_created += 1;
        }

        Ok(AutoClusterResult {
            people_created,
            faces_assigned,
            singletons_skipped,
        })
    }
}

/// Convert a float embedding to a byte blob (little-endian).
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

/// Convert a byte blob back to a float embedding.
fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Serialize people to YAML for persistence.
pub fn save_people_yaml(
    people: &[(Person, Vec<StoredFace>)],
    catalog_root: &std::path::Path,
) -> Result<()> {
    let path = catalog_root.join("people.yaml");
    let yaml = serde_yaml::to_string(
        &people
            .iter()
            .map(|(p, faces)| {
                serde_yaml::to_value(serde_json::json!({
                    "id": p.id,
                    "name": p.name,
                    "representative_face_id": p.representative_face_id,
                    "created_at": p.created_at,
                    "face_ids": faces.iter().map(|f| &f.id).collect::<Vec<_>>(),
                }))
                .unwrap()
            })
            .collect::<Vec<_>>(),
    )?;
    std::fs::write(&path, yaml).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Create minimal assets table for foreign key
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (id TEXT PRIMARY KEY, name TEXT, created_at TEXT, asset_type TEXT)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type) VALUES ('asset-1', 'Test', '2024-01-01', 'image')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type) VALUES ('asset-2', 'Test2', '2024-01-02', 'image')",
            [],
        )
        .unwrap();
        FaceStore::initialize(&conn).unwrap();
        conn
    }

    #[test]
    fn initialize_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        FaceStore::initialize(&conn).unwrap();
        FaceStore::initialize(&conn).unwrap();
    }

    #[test]
    fn store_and_retrieve_face() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let emb = vec![0.1f32; 512];
        store
            .store_face("face-1", "asset-1", 0.1, 0.2, 0.3, 0.4, &emb, 0.95)
            .unwrap();

        let faces = store.faces_for_asset("asset-1").unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].id, "face-1");
        assert!((faces[0].bbox_x - 0.1).abs() < 1e-6);
        assert!((faces[0].confidence - 0.95).abs() < 1e-6);
    }

    #[test]
    fn has_faces_true_false() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        assert!(!store.has_faces("asset-1"));

        store
            .store_face("face-1", "asset-1", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.9)
            .unwrap();

        assert!(store.has_faces("asset-1"));
        assert!(!store.has_faces("asset-2"));
    }

    #[test]
    fn face_count() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        assert_eq!(store.face_count("asset-1"), 0);

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.3, 0.3, &vec![0.0; 512], 0.9)
            .unwrap();
        store
            .store_face("f2", "asset-1", 0.5, 0.0, 0.3, 0.3, &vec![0.0; 512], 0.8)
            .unwrap();

        assert_eq!(store.face_count("asset-1"), 2);
    }

    #[test]
    fn get_face_embedding() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let emb = vec![0.5f32; 512];
        store
            .store_face("face-1", "asset-1", 0.0, 0.0, 0.5, 0.5, &emb, 0.9)
            .unwrap();

        let retrieved = store.get_face_embedding("face-1").unwrap().unwrap();
        assert_eq!(retrieved.len(), 512);
        assert!((retrieved[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn get_face_embedding_nonexistent() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);
        assert!(store.get_face_embedding("nonexistent").unwrap().is_none());
    }

    #[test]
    fn create_and_get_person() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let pid = store.create_person(Some("Alice")).unwrap();
        let person = store.get_person(&pid).unwrap().unwrap();
        assert_eq!(person.name.as_deref(), Some("Alice"));
        assert!(person.representative_face_id.is_none());
    }

    #[test]
    fn name_person() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let pid = store.create_person(None).unwrap();
        assert!(store.get_person(&pid).unwrap().unwrap().name.is_none());

        store.name_person(&pid, "Bob").unwrap();
        assert_eq!(
            store.get_person(&pid).unwrap().unwrap().name.as_deref(),
            Some("Bob")
        );
    }

    #[test]
    fn assign_face_to_person() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.9)
            .unwrap();
        let pid = store.create_person(Some("Alice")).unwrap();
        store.assign_face_to_person("f1", &pid).unwrap();

        let faces = store.faces_for_asset("asset-1").unwrap();
        assert_eq!(faces[0].person_id.as_deref(), Some(pid.as_str()));
    }

    #[test]
    fn list_people() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        store.create_person(Some("Alice")).unwrap();
        store.create_person(Some("Bob")).unwrap();

        let people = store.list_people().unwrap();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].0.name.as_deref(), Some("Alice"));
        assert_eq!(people[1].0.name.as_deref(), Some("Bob"));
    }

    #[test]
    fn merge_people() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let pid1 = store.create_person(Some("Alice")).unwrap();
        let pid2 = store.create_person(Some("Also Alice")).unwrap();

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.9)
            .unwrap();
        store
            .store_face("f2", "asset-2", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.8)
            .unwrap();
        store.assign_face_to_person("f1", &pid1).unwrap();
        store.assign_face_to_person("f2", &pid2).unwrap();

        let moved = store.merge_people(&pid1, &pid2).unwrap();
        assert_eq!(moved, 1);

        // All faces now assigned to pid1
        let faces = store.faces_for_asset("asset-2").unwrap();
        assert_eq!(faces[0].person_id.as_deref(), Some(pid1.as_str()));

        // pid2 is deleted
        assert!(store.get_person(&pid2).unwrap().is_none());
    }

    #[test]
    fn delete_faces_for_asset() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.3, 0.3, &vec![0.0; 512], 0.9)
            .unwrap();
        store
            .store_face("f2", "asset-1", 0.5, 0.0, 0.3, 0.3, &vec![0.0; 512], 0.8)
            .unwrap();

        let deleted = store.delete_faces_for_asset("asset-1").unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(store.face_count("asset-1"), 0);
    }

    #[test]
    fn delete_person_unassigns_faces() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let pid = store.create_person(Some("Alice")).unwrap();
        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.9)
            .unwrap();
        store.assign_face_to_person("f1", &pid).unwrap();

        store.delete_person(&pid).unwrap();

        assert!(store.get_person(&pid).unwrap().is_none());
        let faces = store.faces_for_asset("asset-1").unwrap();
        assert!(faces[0].person_id.is_none());
    }

    #[test]
    fn find_similar_faces() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let emb_a = vec![1.0f32; 512];
        let emb_b = vec![0.9f32; 512];
        let mut emb_c = vec![0.0f32; 512];
        emb_c[0] = 1.0;

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.5, 0.5, &emb_a, 0.9)
            .unwrap();
        store
            .store_face("f2", "asset-1", 0.5, 0.0, 0.5, 0.5, &emb_b, 0.8)
            .unwrap();
        store
            .store_face("f3", "asset-2", 0.0, 0.0, 0.5, 0.5, &emb_c, 0.7)
            .unwrap();

        let results = store
            .find_similar_faces(&emb_a, 0.5, 10, Some("f1"))
            .unwrap();
        // emb_b is very similar to emb_a, emb_c is different
        assert!(!results.is_empty());
        assert_eq!(results[0].face_id, "f2");
        assert!(results[0].similarity > 0.9);
    }

    #[test]
    fn total_counts() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        assert_eq!(store.total_faces(), 0);
        assert_eq!(store.total_people(), 0);

        store
            .store_face("f1", "asset-1", 0.0, 0.0, 0.5, 0.5, &vec![0.0; 512], 0.9)
            .unwrap();
        store.create_person(Some("Alice")).unwrap();

        assert_eq!(store.total_faces(), 1);
        assert_eq!(store.total_people(), 1);
    }

    #[test]
    fn blob_round_trip() {
        let original = vec![1.5f32, -2.3, 0.0, std::f32::consts::PI];
        let blob = embedding_to_blob(&original);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(original, recovered);
    }
}
