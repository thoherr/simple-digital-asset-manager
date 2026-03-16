---
layout: single
title: "Testing und Qualität: Wie automatisierte Tests den AI-Workflow absichern"
date: 2026-03-19
categories:
  - tipps
tags:
  - Agentic Coding
  - AI Coding
  - Claude
  - Claude Code
  - Testing
  - Rust
  - Qualitätssicherung
---

Wenn eine KI Code schreibt, wer prüft dann die Qualität? Die Antwort: **Tests, Compiler und der Mensch** — in dieser Reihenfolge der Häufigkeit. Im [DAM-Projekt](/tipps/2026/03/04/dam-erfahrungsbericht/) haben 778 automatisierte Tests und Rusts Typsystem dafür gesorgt, dass der Code trotz hoher Geschwindigkeit stabil blieb. Dieser Artikel zeigt, wie.

## Die Test-Pyramide

Das Projekt hat zwei Test-Ebenen:

```
                    ┌────────────────────┐
                    │   243 Integration  │
                    │      Tests         │
                    │                    │
                    │  CLI-Befehle als   │
                    │  Black-Box-Tests   │
                    └────────┬───────────┘
                             │
          ┌──────────────────┴──────────────────┐
          │          535 Unit-Tests              │
          │                                     │
          │   Einzelne Module isoliert testen    │
          │   Suchparser, XMP, Katalog, AI, ... │
          └─────────────────────────────────────┘
```

**Unit-Tests** (`cargo test --lib`) prüfen einzelne Funktionen und Module: Parst der Suchparser `rating:4+` korrekt? Erzeugt die XMP-Aktualisierung valides XML? Findet die Cosine-Similarity die richtigen Nachbarn?

**Integrationstests** (`cargo test --test cli`) prüfen das Gesamtsystem über die CLI: Import einer Datei, Suche, Tag-Änderung, Verifizierung — alles über `maki`-Befehle, als würde ein Benutzer sie eintippen.

## Wie Claude Tests schreibt

Claude schreibt Tests nicht, weil ich es jedes Mal verlange. Es ist zum **etablierten Pattern** geworden. Wenn Claude ein Feature implementiert und dabei bestehende Tests sieht, schreibt es automatisch neue Tests im gleichen Stil.

```
Der Feedback-Loop:

  Feature implementieren
         ↓
  Bestehende Tests als Muster lesen
         ↓
  Neue Tests im gleichen Stil schreiben
         ↓
  cargo test → Fehler finden
         ↓
  Code korrigieren
         ↓
  Alle Tests grün ✓
         ↓
  CLAUDE.md aktualisieren
```

### Beispiel: Suchparser-Tests

Der Suchparser ist das Herzstück der Abfragesprache. Jeder Filter braucht einen Test:

```rust
#[test]
fn parse_quoted_tag_with_spaces() {
    let p = parse_search_query(r#"tag:"Fools Theater" rating:4+"#);
    assert_eq!(p.tags, vec!["Fools Theater"]);
    assert_eq!(p.rating_min, Some(4));
    assert!(p.text.is_none());
}

#[test]
fn parse_mixed_filters_with_text() {
    let p = parse_search_query("camera:fuji sunset iso:400 landscape");
    assert_eq!(p.cameras, vec!["fuji"]);
    assert_eq!(p.iso_min, Some(400));
    assert_eq!(p.iso_max, Some(400));
    assert_eq!(p.text.as_deref(), Some("sunset landscape"));
}

#[test]
fn parse_geo_bounding_box() {
    let p = parse_search_query("geo:48.0,11.0,48.5,11.8");
    assert!(p.geo.is_some());
    // ... Bounding-Box-Parameter prüfen
}
```

Über 40 solcher Tests decken die Suchsyntax ab. Jeder ist klein, fokussiert und sofort verständlich. Claude hat die meisten davon geschrieben — nach dem Muster, das die ersten drei Tests vorgegeben haben.

## Test-Infrastruktur: Helpers und Patterns

### Unit-Test-Helpers

Für Unit-Tests nutzen wir In-Memory-SQLite und minimale Setup-Funktionen:

```rust
/// Katalog mit einem Asset für Suchtest-Zwecke.
fn setup_search_catalog() -> Catalog {
    let catalog = Catalog::open_in_memory().unwrap();
    catalog.initialize().unwrap();

    let mut asset = Asset::new(AssetType::Image, "sha256:search1");
    asset.name = Some("sunset photo".to_string());
    asset.tags = vec!["landscape".to_string(), "nature".to_string()];

    let variant = Variant {
        content_hash: "sha256:search1".to_string(),
        asset_id: asset.id.clone(),
        role: VariantRole::Original,
        format: "jpg".to_string(),
        file_size: 5000,
        original_filename: "sunset_beach.jpg".to_string(),
        source_metadata: Default::default(),
        locations: vec![],
    };
    asset.variants.push(variant.clone());
    catalog.insert_asset(&asset).unwrap();
    catalog.insert_variant(&variant).unwrap();
    catalog
}
```

Wichtig: `asset.variants` muss befüllt sein *bevor* `insert_asset()` aufgerufen wird — weil die denormalisierten Spalten (`best_variant_hash`, `variant_count`) bei der Insertion berechnet werden. Das ist ein Stolperstein, den Claude gelernt hat und der in der CLAUDE.md dokumentiert ist.

### Integrations-Test-Helpers

Für CLI-Tests wird jeder Test in einem temporären Verzeichnis ausgeführt:

```rust
/// Katalog initialisieren und ein Volume registrieren.
fn init_catalog(dir: &Path) -> PathBuf {
    let canonical = dir.canonicalize().expect("canonicalize tempdir");
    maki().current_dir(&canonical).arg("init").assert().success();
    maki()
        .current_dir(&canonical)
        .args(["volume", "add", "test-vol",
               canonical.to_str().unwrap()])
        .assert()
        .success();
    canonical
}

/// Testdatei mit beliebigem Inhalt erstellen.
fn create_test_file(dir: &Path, name: &str, content: &[u8])
    -> PathBuf
{
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}
```

Diese Helpers sind **komponierbar**: Ein Test, der zwei Volumes braucht, ruft `init_two_volumes()` auf. Einer, der XMP-Metadaten testen will, erstellt eine `.xmp`-Datei mit `create_test_file()` und importiert sie.

## Drei Beispiel-Tests, die den Unterschied machen

### 1. XMP-Roundtrip: Schreiben und Zurücklesen

Dieser Test prüft den kompletten Zyklus: XMP importieren → Rating ändern → XMP-Datei prüfen:

```rust
#[test]
fn import_xmp_applies_metadata() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root.join("photos"), "DSC_100.nef",
                     b"raw image bytes");

    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3" xmp:Label="Yellow">
   <dc:subject><rdf:Bag>
    <rdf:li>wildlife</rdf:li>
    <rdf:li>birds</rdf:li>
   </rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&root.join("photos"), "DSC_100.xmp",
                     xmp.as_bytes());

    // Import
    maki().current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported")
            .and(predicate::str::contains("1 recipe")));

    // Verify metadata
    let output = maki().current_dir(&root)
        .args(["search", "DSC_100"]).output().unwrap();
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace().next().unwrap().to_string();

    maki().current_dir(&root)
        .args(["show", &id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("wildlife")
                .and(predicate::str::contains("birds"))
                .and(predicate::str::contains("3"))
                .and(predicate::str::contains("Yellow")));
}
```

Dieser Test sichert ab, dass der komplette XMP-Import-Pfad funktioniert — von der Datei über den Parser bis in die Datenbank. Wenn Claude den XMP-Parser ändert, fällt das hier sofort auf.

### 2. Korruptionserkennung: Datenintegrität

```rust
#[test]
fn verify_detects_corruption() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "corrupt.jpg",
                                 b"original data");

    maki().current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert().success();

    // Datei nach Import korrumpieren
    std::fs::write(&file, b"corrupted data!!!").unwrap();

    maki().current_dir(&root)
        .arg("verify")
        .assert()
        .failure()  // Exit-Code 1!
        .stdout(predicate::str::contains("FAILED"));
}
```

Dieser Test ist **kritisch für einen DAM**. Ein Digital Asset Manager, der korrupte Dateien nicht erkennt, ist nutzlos. Der Test stellt sicher, dass die SHA-256-Verifikation funktioniert — auch nach Code-Änderungen.

### 3. XMP-Update mit Preservation

```rust
#[test]
fn update_rating_preserves_other_content() {
    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="...">
  <rdf:Description rdf:about=""
    xmp:Rating="2" xmp:Label="Blue">
   <dc:subject><rdf:Bag>
    <rdf:li>landscape</rdf:li>
   </rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let result = update_rating_in_string(xmp, "5");
    assert!(result.contains(r#"xmp:Rating="5""#));  // geändert
    assert!(result.contains(r#"xmp:Label="Blue""#)); // erhalten
    assert!(result.contains("landscape"));            // erhalten
}
```

Dieser Test prüft sowohl die **Transformation** (Rating geändert) als auch die **Preservation** (alles andere unberührt). Bei XMP-Bearbeitung ist das entscheidend — ein Fehler hier würde CaptureOne-Metadaten zerstören.

## Rusts Typsystem als Qualitätssicherung

Tests sind nur eine Seite der Medaille. Die andere ist **Rusts Compiler**:

```
Fehlerklassen und wer sie fängt:

┌──────────────────────────┬────────┬──────────┬─────────┐
│ Fehlerklasse             │Compiler│Unit-Test │Int-Test │
├──────────────────────────┼────────┼──────────┼─────────┤
│ Typ-Fehler               │   ✓    │          │         │
│ Ownership/Borrow-Fehler  │   ✓    │          │         │
│ Fehlende Match-Arms      │   ✓    │          │         │
│ Unused Variables         │   ✓    │          │         │
│ Logik-Fehler             │        │    ✓     │         │
│ Parser-Bugs              │        │    ✓     │         │
│ SQL-Fehler               │        │    ✓     │   ✓     │
│ Dateisystem-Probleme     │        │          │   ✓     │
│ CLI-Argument-Parsing     │        │          │   ✓     │
│ Cross-Module-Interaktion │        │          │   ✓     │
│ XMP-Kompatibilität       │        │    ✓     │   ✓     │
│ Architektur-Fehler       │        │          │  Mensch │
└──────────────────────────┴────────┴──────────┴─────────┘
```

### Ein konkretes Beispiel

Bei der Implementierung der AI-Auto-Tag-Funktion für die Web-Oberfläche gab es einen Compiler-Fehler:

```
error[E0308]: mismatched types
  --> src/web/mod.rs:85:5
   |
85 |     let app = app.route(...)
   |     ^^^^^^^^ expected `Router<Arc<AppState>>`,
   |              found `Router<()>`
```

Claude hatte `with_state()` an der falschen Stelle aufgerufen. Der Axum-Router ändert seinen Typen nach `with_state()` — ein subtiler Fehler, den Python oder JavaScript nicht gefunden hätten. Der Compiler hat ihn sofort gemeldet.

**Das ist der eigentliche Vorteil von Rust in einem AI-Workflow:** Der Compiler fängt eine ganze Klasse von Fehlern ab, die in dynamischen Sprachen erst zur Laufzeit — oder nie — auffallen.

## Test-Qualitäts-Patterns

Aus 17 Tagen Pair Programming haben sich folgende Patterns etabliert:

### 1. Positive UND negative Tests

```rust
// Positiv: Rating-Filter findet Assets
#[test]
fn search_by_rating_minimum() {
    // ... asset mit rating 4 ...
    let results = catalog.search("rating:3+");
    assert_eq!(results.len(), 1);  // gefunden
}

// Negativ: Falscher Filter findet nichts
#[test]
fn search_by_rating_no_match() {
    // ... asset mit rating 2 ...
    let results = catalog.search("rating:4+");
    assert!(results.is_empty());  // nicht gefunden
}
```

### 2. Seiteneffekte prüfen, nicht nur Rückgabewerte

```rust
#[test]
fn relocate_with_remove_source() {
    // ... Asset von vol1 nach vol2 verschieben ...

    // Nicht nur den Exit-Code prüfen:
    assert!(vol2.join("photo.jpg").exists());   // Datei auf Ziel
    assert!(!vol1.join("photo.jpg").exists());   // Datei von Quelle weg
}
```

### 3. Hierarchische Daten: Alle Ebenen testen

```rust
#[test]
fn search_by_tag_hierarchical() {
    // Tag: animals|birds|eagles

    // Eltern-Tag findet Kind
    let r = catalog.search("tag:animals");
    assert_eq!(r.len(), 1);

    // Zwischen-Tag findet Kind
    let r = catalog.search("tag:animals/birds");
    assert_eq!(r.len(), 1);

    // Exakter Tag findet sich selbst
    let r = catalog.search("tag:animals/birds/eagles");
    assert_eq!(r.len(), 1);

    // Falscher Eltern-Tag findet nichts
    let r = catalog.search("tag:cats");
    assert!(r.is_empty());
}
```

## Testabdeckung über die Zeit

```
Entwicklung der Testzahl:

Tag  1-2:   ~40 Tests     Grundfunktionen
Tag  3-5:  ~120 Tests     XMP, Suche, Tags
Tag  6-8:  ~250 Tests     Web UI, Batch, Collections
Tag  9-12: ~400 Tests     Stacks, Smart Previews, Calendar
Tag 13-15: ~550 Tests     Map, Facets, Duplicates
Tag 16-17: ~700 Tests     AI-Integration, Similarity
Heute:     ~778 Tests     + Face Recognition

Durchschnitt: ~45 neue Tests pro Arbeitstag
```

Claude hat den Großteil dieser Tests geschrieben — nicht weil ich jedes Mal "Schreib auch Tests" gesagt habe, sondern weil es zum Pattern gehört. Die ersten Tests habe ich manuell vorgegeben, danach hat Claude das Muster übernommen.

## Der Test-getriebene Korrektur-Zyklus

Wenn Claude Code produziert, der einen bestehenden Test bricht, passiert folgendes:

```
Claude schreibt neuen Code
        ↓
cargo test → 3 Tests rot
        ↓
Claude sieht die Fehlermeldungen
        ↓
Claude analysiert die Ursache
        ↓
Claude korrigiert den Code
        ↓
cargo test → alle grün ✓
```

Das funktioniert in den meisten Fällen automatisch. Der Compiler und die Tests bilden zusammen ein **Sicherheitsnetz**, das es erlaubt, schnell zu entwickeln, ohne ständig manuell zu prüfen.

### Wann das Sicherheitsnetz versagt

Tests fangen Logik-Fehler und Regressions ab. Was sie *nicht* fangen:

- **Architektur-Drift** — Code, der funktioniert, aber nicht ins Gesamtbild passt
- **Performance-Regressionen** — ein Query, der 10x langsamer ist, aber korrekte Ergebnisse liefert
- **UX-Probleme** — ein Button, der funktioniert, aber an der falschen Stelle sitzt

Hier muss der Mensch eingreifen. Tests geben Vertrauen in die Korrektheit — nicht in die Qualität im weiteren Sinne.

## Lessons Learned

**1. Die ersten Tests setzen den Standard.** Claude übernimmt den Stil der existierenden Tests. Investiere in saubere, gut strukturierte Anfangstests — der Rest folgt automatisch.

**2. Rust multipliziert den Wert von Tests.** Der Compiler fängt Typ- und Ownership-Fehler. Die Tests fangen Logik-Fehler. Zusammen decken sie 95% der möglichen Bugs ab. In Python bräuchte man deutlich mehr Tests für die gleiche Sicherheit.

**3. In-Memory-SQLite ist Gold wert.** `Catalog::open_in_memory()` macht Unit-Tests schnell und isoliert. Kein Dateisystem-Overhead, kein Aufräumen, keine Seiteneffekte zwischen Tests.

**4. Integrationstests sind der wahre Schutz.** Unit-Tests prüfen Module. Integrationstests prüfen, ob das Zusammenspiel funktioniert. Bei einem System mit 30 Modulen ist das Zusammenspiel das, was am häufigsten bricht.

**5. 778 Tests in 17 Tagen sind realistisch.** Nicht *trotz* AI-Unterstützung, sondern *wegen*. Claude schreibt Tests schneller als jeder Mensch, und es vergisst keine Edge Cases — vorausgesetzt, das Pattern ist etabliert.

Im [nächsten Artikel](/tipps/2026/03/24/web-ui-entwicklung/) schauen wir uns an, wie Claude mit Askama-Templates, CSS und JavaScript umgeht — und warum eine Web-Oberfläche das schwierigste Feature des Projekts war.

---

*Thomas Herrmann ist Geschäftsführer der [42ways GmbH](https://42ways.de) und beschäftigt sich mit dem praktischen Einsatz von KI in der Softwareentwicklung.*
