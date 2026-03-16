---
layout: single
title: "AI-Integration: Wie wir mit Claude ein SigLIP-Modell in eine Rust-Anwendung integriert haben"
date: 2026-03-29
categories:
  - tipps
tags:
  - Agentic Coding
  - AI Coding
  - Claude
  - Claude Code
  - SigLIP
  - ONNX Runtime
  - Rust
  - Machine Learning
---

Am Tag 17 des [DAM-Projekts](/tipps/2026/03/04/dam-erfahrungsbericht/) haben wir AI-gestütztes Auto-Tagging implementiert: Ein Benutzer klickt auf "Suggest tags", und das System schlägt automatisch passende Schlagwörter vor — "landscape", "sunset", "ocean" — basierend auf dem Bildinhalt. Keine Cloud-API, keine Internet-Verbindung nötig. Alles läuft lokal, in unter 200 Millisekunden pro Bild.

Dieser Artikel zeigt den Weg von der Idee zum funktionierenden Feature — inklusive der Entscheidungsfindung, der technischen Herausforderungen und der Architektur, die es ermöglicht.

## Die Entscheidung: Drei Optionen, eine Wahl

Bevor eine Zeile Code geschrieben wurde, stand ein [Proposal-Dokument](https://github.com/thoherr/simple-digital-asset-manager/blob/main/doc/proposals/proposal-ai-autotagging.md) mit 566 Zeilen. Darin wurden drei Ansätze verglichen:

```
┌──────────────────────────────────────────────────────────┐
│            Entscheidungsmatrix: AI Auto-Tagging          │
├──────────────┬────────────┬────────────┬─────────────────┤
│              │  Option A  │  Option B  │    Option C     │
│              │ ONNX embed.│  Python    │    Ollama       │
├──────────────┼────────────┼────────────┼─────────────────┤
│ Abhängigkeit │ ort Crate  │ Python 3.8+│ Ollama Server   │
│ Binary +     │ +50-150 MB │ ~0         │ ~0              │
│ Modell       │ 150-600 MB │ identisch  │ 1-6 GB          │
│ RAM          │ 400-800 MB │ ähnlich    │ 2-8 GB          │
│ Geschw./Bild │ 50-200 ms  │ ähnlich    │ 5-36 Sekunden   │
│ Compile-Zeit │ +2-5 Min   │ ~0         │ ~0              │
│ Offline      │ ✓          │ ✓          │ ✓ (Ollama nötig)│
│ Self-contain.│ ✓          │ ✗          │ ✗               │
└──────────────┴────────────┴────────────┴─────────────────┘
```

**Entscheidung: Option A** — ONNX Runtime, eingebettet über das `ort`-Crate. Gründe:

1. **Self-contained:** Keine externe Runtime nötig (Python, Ollama)
2. **Schnell:** 50-200 ms pro Bild, nicht 5-36 Sekunden
3. **Kontrollierbar:** Feature-Flag `--features ai` hält die Build-Pipeline schlank
4. **Klein:** SigLIP ViT-B/16 braucht nur 340 MB für Modell + Runtime

## Das Modell: SigLIP statt CLIP

Bei der Modellwahl wurde ebenfalls im Proposal verglichen:

```
Modellvergleich (Zero-Shot Image Classification):

  Modell            │ ImageNet │ Download │ Inferenz
  ──────────────────┼──────────┼──────────┼──────────
  CLIP ViT-B/32     │  63.2%   │  350 MB  │  ~120 ms
  OpenCLIP ViT-B/32 │  66.6%   │  350 MB  │  ~120 ms
  SigLIP ViT-B/16   │  79.1%   │  340 MB  │  ~200 ms  ←
  MobileCLIP-S2     │  67.5%   │  120 MB  │   ~50 ms
  TinyCLIP ViT-8    │  41.1%   │   30 MB  │   ~15 ms
```

**SigLIP ViT-B/16-256** wurde gewählt: Bestes Verhältnis aus Genauigkeit (79.1%) und Geschwindigkeit (~200 ms). Der entscheidende Unterschied zu CLIP: SigLIP verwendet **Sigmoid-basiertes** statt Softmax-basiertes Scoring.

```
CLIP vs. SigLIP — Scoring-Unterschied:

CLIP (Softmax):
  P("sunset") + P("forest") + P("car") + ... = 1.0
  → Wahrscheinlichkeiten summieren sich zu 1
  → Gut für "Was ist das Hauptmotiv?"

SigLIP (Sigmoid):
  P("sunset") = 0.85    ← unabhängig
  P("ocean")  = 0.72    ← unabhängig
  P("car")    = 0.02    ← unabhängig
  → Jedes Label wird separat bewertet
  → Gut für "Welche Tags passen?" (Multi-Label)
```

Für Auto-Tagging ist Sigmoid ideal: Ein Foto kann gleichzeitig "sunset", "ocean" und "landscape" sein — bei CLIP würde das Softmax die Wahrscheinlichkeiten untereinander aufteilen.

## Die Architektur

```
┌─────────────────────────────────────────────────────┐
│                    Web UI                            │
│  [Suggest Tags]  [Find Similar]  [Auto-Tag (Batch)] │
│        │               │               │            │
├────────┴───────────────┴───────────────┴────────────┤
│                  Axum Routes                         │
│  POST /api/asset/{id}/suggest-tags                  │
│  POST /api/asset/{id}/similar                        │
│  POST /api/batch/auto-tag                            │
│        │                                             │
├────────┴─────────────────────────────────────────────┤
│              AI Module (src/ai.rs)                    │
│  ┌─────────────────────────────────────────────┐     │
│  │  SigLipModel                                │     │
│  │  ├── vision: Session  (Vision Encoder)      │     │
│  │  ├── text: Session    (Text Encoder)        │     │
│  │  └── tokenizer: Tokenizer                   │     │
│  │                                             │     │
│  │  encode_image(path) → Vec<f32>              │     │
│  │  encode_texts(labels) → Vec<Vec<f32>>       │     │
│  │  classify(img_emb, labels, ...) → Vec<Tag>  │     │
│  └─────────────────────────────────────────────┘     │
│                                                      │
├──────────────────────────────────────────────────────┤
│          Embedding Store (SQLite)                     │
│  ┌────────────────────────────────────────────┐      │
│  │ asset_id │ model              │ embedding  │      │
│  │ abc-123  │ siglip-vit-b16-256 │ BLOB       │      │
│  └────────────────────────────────────────────┘      │
│  store_embedding() / find_similar()                   │
│                                                      │
├──────────────────────────────────────────────────────┤
│          Model Manager                               │
│  ~/.cache/maki/models/                                │
│  ├── onnx/vision_model_quantized.onnx    (~150 MB)  │
│  ├── onnx/text_model_quantized.onnx      (~150 MB)  │
│  └── tokenizer.json                      (~2 MB)    │
│                                                      │
│  Download via curl von HuggingFace                   │
└──────────────────────────────────────────────────────┘
```

## Implementierung: Schritt für Schritt

### 1. Feature-Flag in Cargo.toml

```toml
[features]
default = []
ai = ["ort", "ndarray", "tokenizers"]

[dependencies]
ort = { version = "2.0.0-rc.11", optional = true,
        default-features = false,
        features = ["std", "ndarray", "download-binaries",
                     "tls-native", "copy-dylibs"] }
ndarray = { version = "0.17", optional = true }
tokenizers = { version = "0.20", optional = true,
               default-features = false, features = ["onig"] }
```

`cargo build` kompiliert in 15 Sekunden. `cargo build --features ai` in 3 Minuten — aber nur beim ersten Mal, danach inkrementell.

### 2. Modell-Spezifikation

Jedes unterstützte Modell wird als statische Konstante definiert:

```rust
pub struct ModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub hf_repo: &'static str,
    pub embedding_dim: usize,
    pub image_size: usize,
    pub logit_scale: f32,     // SigLIP-spezifisch
    pub logit_bias: f32,      // SigLIP-spezifisch
    pub max_text_len: usize,
    pub pad_token_id: u32,
}

pub const MODEL_SPECS: &[ModelSpec] = &[
    ModelSpec {
        id: "siglip-vit-b16-256",
        display_name: "SigLIP ViT-B/16-256",
        hf_repo: "Xenova/siglip-base-patch16-256",
        embedding_dim: 768,
        image_size: 256,
        logit_scale: 4.7129,   // ln(111.57)
        logit_bias: -12.9283,
        max_text_len: 64,
        pad_token_id: 1,
    },
];
```

`logit_scale` und `logit_bias` sind trainierte Hyperparameter des SigLIP-Modells. Sie steuern die Sigmoid-Funktion:

```
confidence = sigmoid(exp(logit_scale) × dot(image, text) + logit_bias)
           = sigmoid(111.57 × cosine_similarity - 12.93)
```

### 3. Bild-Encoding

```rust
pub fn encode_image(&mut self, image_path: &Path) -> Result<Vec<f32>> {
    // 1. Bild laden und auf 256×256 skalieren
    let tensor = preprocess_image(image_path, self.spec.image_size)?;

    // 2. Vision-Encoder ausführen
    let input = Tensor::from_array(tensor)?;
    let outputs = self.vision.run(
        ort::inputs!["pixel_values" => input],
    )?;

    // 3. Pooled Embedding extrahieren und L2-normalisieren
    let emb = extract_pooled_embedding(&outputs, ...)?;
    Ok(l2_normalize(&emb))
}
```

```
Bild-Preprocessing Pipeline:

  Originalbild (beliebige Größe)
       ↓
  Resize auf 256×256 (Lanczos)
       ↓
  RGB-Kanäle → f32 [0.0, 1.0]
       ↓
  Normalisierung: (pixel - 0.5) / 0.5
       ↓
  Tensor [1, 3, 256, 256]  (NCHW)
       ↓
  ONNX Vision Encoder
       ↓
  768-dimensionaler Embedding-Vektor
       ↓
  L2-Normalisierung → Einheitsvektor
```

### 4. Klassifikation mit Sigmoid

Die Klassifikation berechnet die Ähnlichkeit zwischen Bild-Embedding und Text-Embeddings:

```rust
fn classify_impl(&self, image_emb: &[f32], labels: &[String],
                 label_embs: &[Vec<f32>], threshold: f32)
    -> Vec<AutoTagSuggestion>
{
    let scale = self.spec.logit_scale.exp();  // 111.57

    labels.iter().zip(label_embs.iter())
        .filter_map(|(label, label_emb)| {
            // Skalarprodukt (= Cosine Similarity bei normalisierten Vektoren)
            let dot: f32 = image_emb.iter()
                .zip(label_emb.iter())
                .map(|(a, b)| a * b)
                .sum();

            // SigLIP Sigmoid-Scoring
            let logit = scale * dot + self.spec.logit_bias;
            let confidence = 1.0 / (1.0 + (-logit).exp());

            if confidence >= threshold {
                Some(AutoTagSuggestion {
                    tag: label.clone(),
                    confidence,
                })
            } else {
                None
            }
        })
        .collect()
}
```

### 5. Lazy Loading im Web Server

Das Modell wird nicht beim Serverstart geladen, sondern erst beim ersten Request:

```rust
pub struct AppState {
    // ... andere Felder ...

    #[cfg(feature = "ai")]
    pub ai_model: tokio::sync::Mutex<Option<SigLipModel>>,

    #[cfg(feature = "ai")]
    pub ai_label_cache: tokio::sync::RwLock<Option<(Vec<String>, Vec<Vec<f32>>)>>,
}
```

```
Erster Request: POST /api/asset/abc/suggest-tags

  1. Mutex lock auf ai_model       ← ~0 ms
  2. Model ist None → Laden        ← ~800 ms (einmalig)
  3. Label-Embeddings prüfen
     → Noch nicht gecacht           ← ~200 ms (einmalig)
     → Labels aus Datei lesen
     → encode_texts() für alle Labels
     → In RwLock cachen
  4. Bild encodieren                ← ~200 ms
  5. Klassifikation                 ← ~1 ms
  6. Ergebnis zurückgeben

Folgende Requests:

  1. Mutex lock auf ai_model       ← ~0 ms
  2. Model ist Some → weiter       ← ~0 ms
  3. Label-Embeddings aus Cache     ← ~0 ms
  4. Bild encodieren                ← ~200 ms
  5. Klassifikation                 ← ~1 ms
```

Der Trick: `tokio::sync::Mutex` statt `std::sync::Mutex`, weil die ONNX-Inferenz in `spawn_blocking` läuft und der Mutex über `.await`-Grenzen hinweg gehalten werden muss. Und `RwLock` für die Label-Embeddings, weil sie von mehreren Requests gleichzeitig gelesen werden.

### 6. Embedding-Speicherung

Einmal berechnete Bild-Embeddings werden in SQLite gespeichert:

```rust
pub fn store_embedding(&self, asset_id: &str, model: &str,
                       embedding: &[f32]) -> Result<()>
{
    let blob = embedding_to_blob(embedding);
    self.conn.execute(
        "INSERT OR REPLACE INTO embeddings
         (asset_id, model, embedding) VALUES (?1, ?2, ?3)",
        params![asset_id, model, blob],
    )?;
    Ok(())
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}
```

768 Dimensionen × 4 Bytes = 3072 Bytes pro Embedding. Bei 250.000 Assets wären das ~730 MB — akzeptabel für lokale SQLite-Speicherung.

### 7. Ähnlichkeitssuche

Die Similarity Search iteriert über alle gespeicherten Embeddings und berechnet die Cosine-Ähnlichkeit:

```rust
pub fn find_similar(&self, query_emb: &[f32], limit: usize,
                    exclude_id: Option<&str>, model: &str)
    -> Result<Vec<(String, f32)>>
{
    let mut stmt = self.conn.prepare(
        "SELECT asset_id, embedding FROM embeddings
         WHERE model = ?1")?;

    let mut results: Vec<(String, f32)> = Vec::new();
    for row in stmt.query_map(params![model], ...)? {
        let (id, blob) = row?;
        if exclude_id == Some(id.as_str()) { continue; }
        let emb = blob_to_embedding(&blob);
        let sim = cosine_similarity(query_emb, &emb);
        results.push((id, sim));
    }

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    results.truncate(limit);
    Ok(results)
}
```

Ja, das ist Brute-Force — O(n) über alle Embeddings. Bei 250.000 Assets dauert das trotzdem nur ~100 ms, weil die Berechnung rein CPU-gebunden ist (768 Multiplikationen × 250.000 = 192 Millionen FLOPs — eine Aufgabe, die moderne CPUs in unter 100 ms lösen). Für größere Sammlungen wäre ein Approximate Nearest Neighbor Index (HNSW, IVF) nötig.

## Die Web-UI-Integration

Auf der Asset-Detailseite erscheint ein "Suggest tags"-Button:

```
┌─────────────────────────────────────────────┐
│  sunset_beach.jpg                           │
│  ┌──────────────────────────────────┐       │
│  │                                  │       │
│  │        [Vorschaubild]            │       │
│  │                                  │       │
│  └──────────────────────────────────┘       │
│                                             │
│  Tags: landscape, nature                    │
│                                             │
│  AI Tag Suggestions                         │
│  [Suggest tags]                             │
│                                             │
│  ┌──────┐ ┌────────┐ ┌───────┐ ┌─────────┐ │
│  │sunset│ │  ocean │ │ beach │ │coastline│ │
│  │ 94%  │ │  87%   │ │  72%  │ │  61%    │ │
│  │ ✓  × │ │  ✓  ×  │ │ ✓  × │ │  ✓   ×  │ │
│  └──────┘ └────────┘ └───────┘ └─────────┘ │
│                                             │
│  [Accept all]                               │
│                                             │
│  Similar images                             │
│  [Find similar]                             │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐       │
│  │      │ │      │ │      │ │      │       │
│  │ 92%  │ │ 87%  │ │ 81%  │ │ 76%  │       │
│  └──────┘ └──────┘ └──────┘ └──────┘       │
└─────────────────────────────────────────────┘
```

Der JavaScript-Code für "Suggest tags":

```javascript
function suggestTags() {
    var btn = document.getElementById('suggest-tags-btn');
    btn.disabled = true;
    btn.textContent = 'Analyzing...';

    fetch('/api/asset/{{ asset_id }}/suggest-tags',
          { method: 'POST' })
        .then(r => r.json())
        .then(suggestions => {
            // Chips mit Confidence-Badges rendern
            suggestions.forEach(function(s) {
                var chip = document.createElement('span');
                chip.className = 'tag-chip suggested';
                chip.innerHTML = s.tag
                    + ' <small>' + Math.round(s.confidence * 100)
                    + '%</small>'
                    + ' <button onclick="acceptTag(...)">✓</button>'
                    + ' <button onclick="dismissTag(...)">×</button>';
                container.appendChild(chip);
            });
        });
}
```

## Claudes Rolle bei der Implementierung

Claude hat den Großteil des AI-Moduls geschrieben — aber die Richtung kam aus dem Proposal:

```
Was ich vorgegeben habe:        Was Claude implementiert hat:
─────────────────────────       ─────────────────────────────
Option A (ONNX Runtime)    →    ort-Crate-Integration
SigLIP ViT-B/16-256       →    ModelSpec mit Hyperparametern
Feature-Flag               →    Cargo.toml + cfg(feature)
SQLite für Embeddings      →    EmbeddingStore mit BLOB-Storage
Lazy Loading               →    tokio::sync::Mutex Pattern
Sigmoid statt Softmax      →    classify() mit logit_scale/bias
Labels aus Textdatei       →    resolve_labels() + Prompt-Template
```

**Wo Claude besonders stark war:**
- Die ONNX-Session-Konfiguration (Provider-Reihenfolge, Thread-Pool)
- Tensor-Handling mit `ndarray` (Reshape, Transposition, Normalisierung)
- Tokenizer-Integration (Padding, Truncation, Batch-Verarbeitung)
- Fehlerbehandlung (fehlende Modelle, ungültige Bilder, ONNX-Fehler)

**Wo ich eingreifen musste:**
- Die Entscheidung ONNX vs. Python vs. Ollama
- Die Modellwahl (SigLIP vs. CLIP vs. MobileCLIP)
- Die Architektur (Lazy Loading, Feature-Flag, Embedding-Storage)
- Das Datenmodell (Embeddings per Model-ID, nicht global)

## Konfiguration

```toml
# maki.toml
[ai]
model = "siglip-vit-b16-256"      # Welches Modell
threshold = 0.5                    # Mindest-Confidence
labels = "labels.txt"              # Eigene Label-Liste
model_dir = "~/.cache/maki/models"  # Modell-Verzeichnis
prompt = "a photo of"              # Prefix für Text-Encoding
```

Die Label-Liste ist anpassbar — ein Naturfotograf braucht andere Labels als ein Eventfotograf:

```
# labels.txt (Beispiel Naturfotografie)
landscape
sunset
sunrise
ocean
mountains
forest
wildlife
birds
flowers
macro
```

Der `prompt`-Parameter steuert das Text-Encoding: "a photo of sunset" funktioniert besser als nur "sunset", weil SigLIP auf natürlichsprachliche Beschreibungen trainiert wurde.

## Ergebnisse

```
Performance-Messungen (Apple M1 Pro):

  Modell laden:          ~800 ms (einmalig)
  Labels encodieren:     ~200 ms (einmalig, 150 Labels)
  Bild encodieren:       ~180 ms
  Klassifikation:        ~1 ms
  Gesamt (erster Call):  ~1200 ms
  Gesamt (folgende):     ~180 ms

  Batch (100 Bilder):    ~20 Sekunden
  Batch (1000 Bilder):   ~3 Minuten
```

Die Qualität der Vorschläge ist überraschend gut. Bei einem Test mit 500 manuell getaggten Fotos:
- **Precision:** ~85% (von den vorgeschlagenen Tags waren 85% korrekt)
- **Recall:** ~60% (von den manuellen Tags wurden 60% gefunden)

Der limitierende Faktor ist die Label-Liste: SigLIP kann nur Labels vorschlagen, die in der Liste stehen. Ein reichhaltigeres Vokabular verbessert den Recall — auf Kosten der Precision.

## Fazit: AI-Features mit Claude bauen

Die Integration eines ML-Modells in eine Rust-Anwendung klingt einschüchternd — und ohne AI-Unterstützung wäre es ein Mehrwochenprojekt gewesen. Mit Claude Code und einem guten Proposal war es **ein Arbeitstag**.

Die Schlüssel zum Erfolg:
1. **Gründliches Proposal** — Die Entscheidung für ONNX/SigLIP war getroffen, bevor Code geschrieben wurde
2. **Feature-Flag** — AI-Code kompiliert nur auf Anforderung
3. **Lazy Loading** — Modell wird erst bei Bedarf geladen
4. **Embedding Cache** — Berechnete Embeddings werden gespeichert, nicht jedes Mal neu berechnet
5. **Brute-Force zuerst** — Keine vorzeitige Optimierung der Similarity Search

Die nächste Phase — **Face Recognition** — baut auf der gleichen ONNX-Infrastruktur auf: YuNet für die Gesichtserkennung, ArcFace für die Gesichts-Embeddings, Chinese Whispers für das Clustering. Dank der Vorarbeit reduziert sich der geschätzte Aufwand von 10+ Tagen auf 7-8 Tage.

---

*Dies ist der letzte Artikel der Serie "AI-gestütztes Pair Programming". Die anderen Teile: [Erfahrungsbericht](/tipps/2026/03/04/dam-erfahrungsbericht/), [Kontextmanagement](/tipps/2026/03/09/kontextmanagement/), [Architekturentscheidungen](/tipps/2026/03/14/architekturentscheidungen/), [Testing und Qualität](/tipps/2026/03/19/testing-und-qualitaet/), [Web-UI-Entwicklung](/tipps/2026/03/24/web-ui-entwicklung/).*

*Thomas Herrmann ist Geschäftsführer der [42ways GmbH](https://42ways.de) und beschäftigt sich mit dem praktischen Einsatz von KI in der Softwareentwicklung. Das DAM-Projekt ist [Open Source auf GitHub](https://github.com/thoherr/simple-digital-asset-manager).*
