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

    /// Assign a face to a person. Auto-sets representative if the person has none.
    pub fn assign_face_to_person(&self, face_id: &str, person_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE faces SET person_id = ?1 WHERE id = ?2",
            rusqlite::params![person_id, face_id],
        )?;
        self.ensure_representative(person_id)?;
        Ok(())
    }

    /// Ensure a person has a representative face. If NULL, picks the highest-confidence face.
    pub fn ensure_representative(&self, person_id: &str) -> Result<()> {
        let has_rep: bool = self.conn.query_row(
            "SELECT representative_face_id IS NOT NULL FROM people WHERE id = ?1",
            rusqlite::params![person_id],
            |row| row.get(0),
        ).unwrap_or(false);
        if !has_rep {
            let best: Option<String> = self.conn.query_row(
                "SELECT id FROM faces WHERE person_id = ?1 ORDER BY confidence DESC LIMIT 1",
                rusqlite::params![person_id],
                |row| row.get(0),
            ).ok();
            if let Some(face_id) = best {
                self.set_representative_face(person_id, &face_id)?;
            }
        }
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
    /// Auto-backfills missing representative faces.
    pub fn list_people(&self) -> Result<Vec<(Person, usize)>> {
        // Backfill any people missing a representative face
        self.conn.execute_batch(
            "UPDATE people SET representative_face_id = (
                SELECT id FROM faces WHERE faces.person_id = people.id
                ORDER BY confidence DESC LIMIT 1
             ) WHERE representative_face_id IS NULL
               AND EXISTS (SELECT 1 FROM faces WHERE faces.person_id = people.id)"
        )?;

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

    /// Delete a single face by ID. Returns the asset_id if the face existed.
    pub fn delete_face(&self, face_id: &str) -> Result<Option<String>> {
        let asset_id: Option<String> = self
            .conn
            .query_row(
                "SELECT asset_id FROM faces WHERE id = ?1",
                rusqlite::params![face_id],
                |row| row.get(0),
            )
            .ok();
        if asset_id.is_some() {
            self.conn.execute(
                "DELETE FROM faces WHERE id = ?1",
                rusqlite::params![face_id],
            )?;
        }
        Ok(asset_id)
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
        self.face_embeddings_scoped(None)
    }

    /// Get face embeddings, optionally scoped to specific asset IDs.
    /// Returns (face_id, person_id, embedding) tuples.
    pub fn face_embeddings_scoped(&self, asset_ids: Option<&[String]>) -> Result<Vec<(String, Option<String>, Vec<f32>)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match asset_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT id, person_id, embedding FROM faces WHERE asset_id IN ({})",
                    placeholders.join(",")
                );
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids.iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                (sql, params)
            }
            _ => {
                ("SELECT id, person_id, embedding FROM faces".to_string(), Vec::new())
            }
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
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

    /// Find asset IDs that have faces assigned to a person.
    ///
    /// Accepts either a person name or a person ID (UUID). If `value` parses
    /// as a UUID, looks up by ID first (this covers unnamed clusters where
    /// `people.name` is NULL); otherwise looks up by name. Falling through to
    /// name lookup also handles the unlikely edge case of a named person
    /// whose name happens to look like a UUID.
    pub fn find_person_asset_ids(&self, value: &str) -> Result<Vec<String>> {
        if uuid::Uuid::parse_str(value).is_ok() {
            let ids = self.find_person_asset_ids_by_id(value)?;
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT f.asset_id FROM faces f
             JOIN people p ON f.person_id = p.id
             WHERE p.name = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![value], |row| row.get(0))?;
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
    /// If `asset_ids` is provided, only faces from those assets are considered.
    /// Returns (clusters, unassigned_count) where each cluster is a list of face_ids with ≥2 faces.
    pub fn cluster_faces(&self, threshold: f32, asset_ids: Option<&[String]>) -> Result<(Vec<Vec<String>>, usize)> {
        let all = self.face_embeddings_scoped(asset_ids)?;
        // Only cluster unassigned faces
        let unassigned: Vec<(String, Vec<f32>)> = all
            .into_iter()
            .filter(|(_, pid, _)| pid.is_none())
            .map(|(id, _, emb)| (id, emb))
            .collect();
        let unassigned_count = unassigned.len();

        if unassigned.is_empty() {
            return Ok((Vec::new(), 0));
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
        let result: Vec<Vec<String>> = clusters
            .into_iter()
            .filter(|(ids, _)| ids.len() >= 2)
            .map(|(ids, _)| ids)
            .collect();
        Ok((result, unassigned_count))
    }

    /// Auto-cluster unassigned faces and create people for each cluster.
    ///
    /// If `asset_ids` is provided, only faces from those assets are considered.
    pub fn auto_cluster(&self, threshold: f32, asset_ids: Option<&[String]>) -> Result<AutoClusterResult> {
        let (clusters, total_unassigned) = self.cluster_faces(threshold, asset_ids)?;

        let mut people_created = 0u32;
        let mut faces_assigned = 0u32;

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

// ── YAML persistence for faces and people ────────────────────────────

/// A face record for YAML persistence (no embedding — stored as binary).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FaceRecord {
    pub id: String,
    pub asset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_id: Option<String>,
    pub bbox_x: f32,
    pub bbox_y: f32,
    pub bbox_w: f32,
    pub bbox_h: f32,
    pub confidence: f32,
    pub created_at: String,
}

/// A person record for YAML persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersonRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_face_id: Option<String>,
    pub created_at: String,
}

/// Wrapper for faces.yaml.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FacesFile {
    pub faces: Vec<FaceRecord>,
}

/// Wrapper for people.yaml.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PeopleFile {
    pub people: Vec<PersonRecord>,
}

/// Load faces from the YAML file. Returns empty list if file doesn't exist.
pub fn load_faces_yaml(catalog_root: &std::path::Path) -> Result<FacesFile> {
    let path = catalog_root.join("faces.yaml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let file: FacesFile = serde_yaml::from_str(&contents)?;
        Ok(file)
    } else {
        Ok(FacesFile::default())
    }
}

/// Save faces to the YAML file.
pub fn save_faces_yaml(catalog_root: &std::path::Path, file: &FacesFile) -> Result<()> {
    let path = catalog_root.join("faces.yaml");
    let contents = serde_yaml::to_string(file)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

/// Load people from the YAML file. Returns empty list if file doesn't exist.
pub fn load_people_yaml(catalog_root: &std::path::Path) -> Result<PeopleFile> {
    let path = catalog_root.join("people.yaml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let file: PeopleFile = serde_yaml::from_str(&contents)?;
        Ok(file)
    } else {
        Ok(PeopleFile::default())
    }
}

/// Save people to the YAML file.
pub fn save_people_yaml(catalog_root: &std::path::Path, file: &PeopleFile) -> Result<()> {
    let path = catalog_root.join("people.yaml");
    let contents = serde_yaml::to_string(file)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

// ── ArcFace binary embedding I/O ────────────────────────────────────

/// Compute the path for an ArcFace face embedding binary file.
/// Layout: `embeddings/arcface/<2-char prefix>/<face_id>.bin`
pub fn arcface_binary_path(
    catalog_root: &std::path::Path,
    face_id: &str,
) -> std::path::PathBuf {
    let prefix = &face_id[..2.min(face_id.len())];
    catalog_root
        .join("embeddings")
        .join("arcface")
        .join(prefix)
        .join(format!("{face_id}.bin"))
}

/// Write a face embedding as raw little-endian f32 bytes.
pub fn write_arcface_binary(
    catalog_root: &std::path::Path,
    face_id: &str,
    embedding: &[f32],
) -> Result<()> {
    let path = arcface_binary_path(catalog_root, face_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = embedding_to_blob(embedding);
    std::fs::write(&path, bytes)?;
    Ok(())
}

/// Delete an ArcFace embedding binary file (if it exists).
pub fn delete_arcface_binary(catalog_root: &std::path::Path, face_id: &str) {
    let path = arcface_binary_path(catalog_root, face_id);
    let _ = std::fs::remove_file(path);
}

/// Scan all ArcFace embedding binaries.
/// Returns `(face_id, embedding)` pairs.
pub fn scan_arcface_binaries(
    catalog_root: &std::path::Path,
) -> Result<Vec<(String, Vec<f32>)>> {
    let base = catalog_root.join("embeddings").join("arcface");
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
            let face_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if face_id.is_empty() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let embedding = blob_to_embedding(&bytes);
            results.push((face_id, embedding));
        }
    }
    Ok(results)
}

// ── FaceStore export/import methods ─────────────────────────────────

impl<'a> FaceStore<'a> {
    /// Export all faces from SQLite to a FacesFile struct.
    pub fn export_all_faces(&self) -> Result<FacesFile> {
        let mut stmt = self.conn.prepare(
            "SELECT id, asset_id, person_id, bbox_x, bbox_y, bbox_w, bbox_h, confidence, created_at
             FROM faces ORDER BY asset_id, bbox_x",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FaceRecord {
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
        let faces = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(FacesFile { faces })
    }

    /// Export all people from SQLite to a PeopleFile struct.
    pub fn export_all_people(&self) -> Result<PeopleFile> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, representative_face_id, created_at FROM people ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PersonRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                representative_face_id: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        let people = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(PeopleFile { people })
    }

    /// Import faces from a FacesFile into SQLite (INSERT OR REPLACE).
    /// Inserts with an empty embedding blob placeholder.
    pub fn import_faces_from_yaml(&self, file: &FacesFile) -> Result<u32> {
        let empty_blob = embedding_to_blob(&[]);
        let mut count = 0u32;
        for f in &file.faces {
            self.conn.execute(
                "INSERT OR REPLACE INTO faces (id, asset_id, person_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    f.id, f.asset_id, f.person_id, f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h,
                    empty_blob, f.confidence, f.created_at
                ],
            )?;
            count += 1;
        }
        Ok(count)
    }

    /// Import people from a PeopleFile into SQLite (INSERT OR REPLACE).
    pub fn import_people_from_yaml(&self, file: &PeopleFile) -> Result<u32> {
        let mut count = 0u32;
        for p in &file.people {
            self.conn.execute(
                "INSERT OR REPLACE INTO people (id, name, representative_face_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![p.id, p.name, p.representative_face_id, p.created_at],
            )?;
            count += 1;
        }
        Ok(count)
    }

    /// Update the embedding blob for a face by ID (used during rebuild-catalog).
    pub fn import_face_embedding(&self, face_id: &str, embedding: &[f32]) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        self.conn.execute(
            "UPDATE faces SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![blob, face_id],
        )?;
        Ok(())
    }

    /// Convenience: export all faces+people from SQLite and save both YAML files.
    pub fn save_all_yaml(&self, catalog_root: &std::path::Path) -> Result<()> {
        let faces_file = self.export_all_faces()?;
        save_faces_yaml(catalog_root, &faces_file)?;
        let people_file = self.export_all_people()?;
        save_people_yaml(catalog_root, &people_file)?;
        Ok(())
    }
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

    #[test]
    fn faces_yaml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let faces_file = FacesFile {
            faces: vec![
                FaceRecord {
                    id: "face-1".into(),
                    asset_id: "asset-1".into(),
                    person_id: Some("person-1".into()),
                    bbox_x: 0.1,
                    bbox_y: 0.2,
                    bbox_w: 0.3,
                    bbox_h: 0.4,
                    confidence: 0.95,
                    created_at: "2024-01-01T00:00:00Z".into(),
                },
                FaceRecord {
                    id: "face-2".into(),
                    asset_id: "asset-1".into(),
                    person_id: None,
                    bbox_x: 0.5,
                    bbox_y: 0.6,
                    bbox_w: 0.1,
                    bbox_h: 0.1,
                    confidence: 0.8,
                    created_at: "2024-01-02T00:00:00Z".into(),
                },
            ],
        };
        save_faces_yaml(dir.path(), &faces_file).unwrap();
        let loaded = load_faces_yaml(dir.path()).unwrap();
        assert_eq!(loaded.faces.len(), 2);
        assert_eq!(loaded.faces[0].id, "face-1");
        assert_eq!(loaded.faces[0].person_id, Some("person-1".into()));
        assert_eq!(loaded.faces[1].person_id, None);
        assert!((loaded.faces[0].confidence - 0.95).abs() < 1e-6);
    }

    #[test]
    fn people_yaml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let people_file = PeopleFile {
            people: vec![
                PersonRecord {
                    id: "person-1".into(),
                    name: Some("Alice".into()),
                    representative_face_id: Some("face-1".into()),
                    created_at: "2024-01-01T00:00:00Z".into(),
                },
                PersonRecord {
                    id: "person-2".into(),
                    name: None,
                    representative_face_id: None,
                    created_at: "2024-01-02T00:00:00Z".into(),
                },
            ],
        };
        save_people_yaml(dir.path(), &people_file).unwrap();
        let loaded = load_people_yaml(dir.path()).unwrap();
        assert_eq!(loaded.people.len(), 2);
        assert_eq!(loaded.people[0].name, Some("Alice".into()));
        assert_eq!(loaded.people[1].name, None);
    }

    #[test]
    fn export_import_round_trip() {
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        // Create test data
        let emb = vec![0.5f32; 512];
        store.store_face("face-1", "asset-1", 0.1, 0.2, 0.3, 0.4, &emb, 0.95).unwrap();
        store.store_face("face-2", "asset-2", 0.5, 0.6, 0.1, 0.1, &emb, 0.80).unwrap();
        let person_id = store.create_person(Some("Alice")).unwrap();
        store.assign_face_to_person("face-1", &person_id).unwrap();

        // Export
        let faces_file = store.export_all_faces().unwrap();
        let people_file = store.export_all_people().unwrap();
        assert_eq!(faces_file.faces.len(), 2);
        assert_eq!(people_file.people.len(), 1);
        assert_eq!(people_file.people[0].name, Some("Alice".into()));

        // Clear and reimport
        conn.execute_batch("DELETE FROM faces; DELETE FROM people;").unwrap();
        assert_eq!(store.total_faces(), 0);
        assert_eq!(store.total_people(), 0);

        let people_imported = store.import_people_from_yaml(&people_file).unwrap();
        let faces_imported = store.import_faces_from_yaml(&faces_file).unwrap();
        assert_eq!(people_imported, 1);
        assert_eq!(faces_imported, 2);

        // Verify data restored (with empty embedding placeholder)
        let faces = store.faces_for_asset("asset-1").unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].person_id.as_deref(), Some(person_id.as_str()));

        // Import embedding for a face
        store.import_face_embedding("face-1", &emb).unwrap();
        let restored_emb = store.get_face_embedding("face-1").unwrap().unwrap();
        assert_eq!(restored_emb.len(), 512);
    }

    #[test]
    fn arcface_binary_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let embedding = vec![0.1f32, 0.2, 0.3, -0.5, std::f32::consts::PI];

        write_arcface_binary(dir.path(), "face-abc123", &embedding).unwrap();

        // Verify file exists at expected path
        let path = arcface_binary_path(dir.path(), "face-abc123");
        assert!(path.exists());
        let path_str = path.to_str().unwrap().replace('\\', "/");
        assert!(path_str.contains("arcface/fa/face-abc123.bin"));

        // Read it back via scan
        let entries = scan_arcface_binaries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "face-abc123");
        assert_eq!(entries[0].1, embedding);

        // Delete
        delete_arcface_binary(dir.path(), "face-abc123");
        assert!(!path.exists());
    }

    #[test]
    fn save_all_yaml_convenience() {
        let dir = tempfile::tempdir().unwrap();
        let conn = setup_db();
        let store = FaceStore::new(&conn);

        let emb = vec![0.5f32; 512];
        store.store_face("face-1", "asset-1", 0.1, 0.2, 0.3, 0.4, &emb, 0.95).unwrap();
        store.create_person(Some("Bob")).unwrap();

        store.save_all_yaml(dir.path()).unwrap();

        // Verify both files written
        assert!(dir.path().join("faces.yaml").exists());
        assert!(dir.path().join("people.yaml").exists());

        let faces = load_faces_yaml(dir.path()).unwrap();
        let people = load_people_yaml(dir.path()).unwrap();
        assert_eq!(faces.faces.len(), 1);
        assert_eq!(people.people.len(), 1);
    }

    #[test]
    fn load_yaml_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let faces = load_faces_yaml(dir.path()).unwrap();
        let people = load_people_yaml(dir.path()).unwrap();
        assert!(faces.faces.is_empty());
        assert!(people.people.is_empty());
    }
}
