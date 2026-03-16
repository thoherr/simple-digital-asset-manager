---
layout: single
title: "Kontextmanagement: Wie CLAUDE.md und Proposals den AI-Workflow steuern"
date: 2026-03-09
categories:
  - tipps
tags:
  - Agentic Coding
  - AI Coding
  - Claude
  - Claude Code
  - Kontextmanagement
  - CLAUDE.md
  - Pair Programming
---

Im [ersten Artikel dieser Serie](/tipps/2026/03/04/dam-erfahrungsbericht/) habe ich beschrieben, wie in 17 Tagen ein vollständiger Digital Asset Manager entstanden ist — mit Claude Code als Programmierpartner. Heute geht es um die vielleicht wichtigste Erkenntnis aus diesem Projekt: **Ohne systematisches Kontextmanagement wäre das Ganze gescheitert.**

Ein LLM wie Claude hat kein Langzeitgedächtnis. Jede Session startet bei Null. Die Qualität des Outputs hängt direkt davon ab, wie gut der Kontext ist, den man mitgibt. In einem Projekt mit 34.500 Zeilen Code und 30 Modulen kann Claude nicht alles gleichzeitig im Blick haben. Also muss man ihm sagen, worauf es ankommt.

Drei Instrumente haben sich dabei bewährt: Die **CLAUDE.md**, **Proposals** und eine **Roadmap**. Zusammen bilden sie ein System, das über Wochen und hunderte Commits hinweg für Konsistenz sorgt.

## Die CLAUDE.md: Claudes Gedächtnis

Die Datei `CLAUDE.md` im Projektstammverzeichnis ist das zentrale Steuerungsinstrument. Claude Code liest sie automatisch bei jeder Session. Sie enthält nicht nur eine Projektbeschreibung, sondern ein detailliertes Regelwerk:

```
┌──────────────────────────────────────────────────────┐
│                    CLAUDE.md                         │
├──────────────────────────────────────────────────────┤
│  Projektübersicht                                    │
│    → Design-Ziele, Kernkonzepte                      │
│                                                      │
│  Technologie-Stack                                   │
│    → Sprache, Crates, externe Tools                  │
│                                                      │
│  Architektur                                         │
│    → Verweis auf Detaildokumente                     │
│                                                      │
│  Implementierte Befehle                              │
│    → Vollständige Liste mit Status                   │
│                                                      │
│  Detailliertes Verhalten                             │
│    → Import, XMP, Previews, Suche, ...               │
│    → Edge Cases, Präzedenzregeln                     │
│    → Datenmodell-Details                             │
│                                                      │
│  Konfiguration                                       │
│    → maki.toml Struktur mit Defaults                  │
│                                                      │
│  Coding-Konventionen                                 │
│    → Patterns, Migrations, Anti-Patterns             │
└──────────────────────────────────────────────────────┘
```

### Was steht da konkret drin?

Die CLAUDE.md ist kein Readme. Sie ist eine **maschinenlesbare Spezifikation**. Ein Ausschnitt aus dem Abschnitt über den Import-Prozess:

> **Stem-based auto-grouping**: Files sharing the same filename stem in the same directory are grouped into one Asset during import. RAW files take priority as the primary variant. Additional media files become extra variants on the same asset.

> **Precedence chain**: EXIF (highest, direct assignment) > embedded XMP (middle, `or_insert`) > sidecar `.xmp` (lowest, `or_insert`). For tags, all sources merge (union). For description/rating/label, the first source that provides a value wins.

Das ist nicht Prosa — das ist eine Spezifikation, die Claude direkt in Code umsetzen kann. Wenn ich sage "Füge XMP-Label-Support hinzu", weiß Claude aus der CLAUDE.md:
- Welche Präzedenz Labels gegenüber anderen Quellen haben
- Dass Labels in Title-Case gespeichert werden
- Dass sie in den YAML-Sidecar *und* in SQLite persistiert werden
- Dass Änderungen in die XMP-Datei zurückgeschrieben werden

### Wie wächst die CLAUDE.md?

Die CLAUDE.md ist ein **lebendes Dokument**. Sie wächst mit dem Projekt. Am Anfang war sie kurz — Projektbeschreibung, Tech-Stack, Architekturskizze. Aber mit jedem implementierten Feature kamen Details hinzu.

```
Zeitverlauf der CLAUDE.md:

Tag 1:   ~15 Zeilen   Projektbeschreibung, Grundkonzepte
Tag 3:   ~30 Zeilen   + Import-Verhalten, Gruppierung
Tag 7:   ~50 Zeilen   + XMP-Integration, Such-Filter, Web UI
Tag 12:  ~80 Zeilen   + Stacks, Smart Previews, Dark Mode
Tag 17: ~107 Zeilen   + AI-Integration, Konfiguration

Heute: >200 Zeilen    + Face Recognition, Similarity Search
```

Entscheidend: Die CLAUDE.md wächst **nicht** unkontrolliert. Veraltete Informationen werden entfernt oder aktualisiert. Es ist kein Changelog, sondern der **aktuelle Stand** des Systems.

### Die drei wichtigsten Regeln in der CLAUDE.md

Manche Einträge steuern nicht das *Was*, sondern das *Wie*:

**1. "Avoid over-engineering"** — Claude neigt dazu, Abstraktionen einzubauen, die erst bei hypothetischer zukünftiger Nutzung Sinn ergeben. Drei ähnliche Zeilen Code sind besser als eine vorzeitige Abstraktion. Diese Regel steht wortwörtlich in der CLAUDE.md und wirkt.

**2. "Denormalized columns are updated in all write paths"** — Ein technisches Pattern, das in mehreren Modulen konsistent eingehalten werden muss. Ohne diese Regel würde Claude bei einem neuen Feature vergessen, die denormalisierten Spalten zu aktualisieren.

**3. "Dual storage: YAML sidecars (source of truth) + SQLite catalog (derived cache)"** — Eine fundamentale Architekturentscheidung, die sich durch das gesamte Projekt zieht. Claude muss bei jeder Änderung beide Speicherorte bedienen.

## Proposals: Strukturierte Feature-Planung

Für größere Features reicht die CLAUDE.md nicht. Hier kommen **Proposals** ins Spiel — kurze Planungsdokumente im `doc/proposals/`-Verzeichnis.

```
doc/proposals/
├── proposal-photo-workflow-integration.md    (239 Zeilen, 19 Features)
├── proposal-export-based-previews.md         (112 Zeilen, 3 Phasen)
├── proposal-storage-workflow.md              (441 Zeilen, 5 Teilfeatures)
├── proposal-ai-autotagging.md                (566 Zeilen, Entscheidungsmatrix)
├── proposal-future-enhancements.md            (44 Zeilen, Ideensammlung)
├── roadmap.md                                (243 Zeilen, priorisiert)
├── enhancements.md                           (303 Zeilen, 16 Ideen)
└── idea-notebook.md                           (50 Zeilen, Rohmaterial)
```

### Anatomie eines Proposals

Ein gutes Proposal hat eine klare Struktur. Hier das Beispiel des Storage-Workflow-Proposals:

```
┌───────────────────────────────────────────────┐
│  Proposal: Storage Workflow                   │
├───────────────────────────────────────────────┤
│  1. Problem                                   │
│     "Wie weiß ich, ob meine Fotos sicher      │
│      auf mehreren Volumes verteilt sind?"      │
│                                               │
│  2. Lösung: Volume Purpose                    │
│     Neues Konzept: Working / Archive / Backup  │
│     → Warum nicht Backup-Variante? (begründet) │
│                                               │
│  3. Darauf aufbauende Features                │
│     Part 1: Volume Purpose Enum          ✅   │
│     Part 2: Enhanced Duplicates Filter   ✅   │
│     Part 3: Dedup Command                ✅   │
│     Part 4: Backup Status               ✅   │
│     Part 5: Web UI Integration           ✅   │
│                                               │
│  4. Implementierungsplan                      │
│     Datei-für-Datei Aufschlüsselung            │
│     mit geschätzten Zeilenänderungen           │
│                                               │
│  5. Status                                    │
│     Alle 5 Teile implementiert (v1.4.0-v1.4.1)│
└───────────────────────────────────────────────┘
```

### Der Feedback-Loop

Das Entscheidende an Proposals ist der **Feedback-Loop**: Wenn ein Feature implementiert ist, wird das Proposal aktualisiert. Versionsnummern werden hinzugefügt. Status-Markierungen (✅) zeigen den Fortschritt. Spätere Claude-Sessions können sehen, was bereits erledigt ist und was noch offen steht.

```
Proposal Lifecycle:

  Idee          → idea-notebook.md (2-3 Zeilen)
       ↓
  Ausarbeitung  → proposal-xyz.md (detaillierter Plan)
       ↓
  Implementierung → Claude liest Proposal, setzt um
       ↓
  Update        → Proposal wird mit ✅ und Version annotiert
       ↓
  Abschluss     → Kerndetails fließen in CLAUDE.md ein
```

### Beispiel: Wie ein Proposal Claude steuert

Das AI-Autotagging-Proposal (566 Zeilen) enthält eine detaillierte Entscheidungsmatrix:

| Aspekt | ONNX Runtime | Python | Ollama |
|--------|:---:|:---:|:---:|
| Neue Abhängigkeit | `ort` Crate | Python 3.8+ | Ollama |
| Binary-Größe | +50-150 MB | ~0 | ~0 |
| Modelldateien | 150-600 MB | identisch | 1-6 GB |
| CPU-Inferenz | 50-200 ms | ähnlich | 5-36 s |
| Plattformen | alle | alle | alle |

Diese Matrix hat Claude nicht selbst erstellt — ich habe sie geschrieben, nachdem ich die Optionen recherchiert hatte. Aber Claude konnte daraus die richtige Implementierung ableiten: Option A (ONNX Runtime) mit Feature-Flag, damit die AI-Funktionalität optional bleibt.

Ohne das Proposal hätte Claude vermutlich den naheliegendsten Weg gewählt — einen Python-Subprocess, weil das weniger Komplexität in der Build-Pipeline bedeutet. Die bessere Entscheidung (eingebettete ONNX Runtime, keine externe Abhängigkeit zur Laufzeit) kam aus dem Proposal.

## Die Roadmap: Priorisierung und Überblick

Die Roadmap (`doc/proposals/roadmap.md`) gibt den strategischen Überblick:

```
Tier 1 — Höchste Priorität
─────────────────────────────────
[✅] Side-by-side Compare View
[✅] Map View mit GPS-Daten
[✅] Smart Previews (2560px)
[✅] AI Auto-Tagging (SigLIP)

Tier 2 — Wichtig
─────────────────────────────────
[ ] Import Profiles
[ ] Watch Mode (Dateisystem-Watcher)
[✅] Export Command
[ ] Undo History

Tier 3 — Nice to Have
─────────────────────────────────
[✅] IPTC Metadata
[ ] Drag-and-Drop
[✅] Statistics Dashboard
[✅] Faceted Sidebar
```

Die Roadmap verhindert, dass Claude und ich uns in Details verlieren. Wenn ich an Tag 14 überlege, was als nächstes kommt, schaue ich auf die Roadmap — nicht in eine endlose Backlog-Liste.

## Das Zusammenspiel: Drei Ebenen des Kontexts

```
┌─────────────────────────────────────────────────────────┐
│                                                         │
│  Ebene 3: Roadmap                                       │
│  ═══════════════                                        │
│  "Was bauen wir als nächstes?"                          │
│  Priorisierung, Überblick, Strategie                    │
│                                                         │
│  ┌─────────────────────────────────────────────────┐    │
│  │                                                 │    │
│  │  Ebene 2: Proposals                             │    │
│  │  ════════════════                               │    │
│  │  "Wie genau bauen wir dieses Feature?"          │    │
│  │  Entscheidungsmatrix, Phasen, Schnittstellen    │    │
│  │                                                 │    │
│  │  ┌─────────────────────────────────────────┐    │    │
│  │  │                                         │    │    │
│  │  │  Ebene 1: CLAUDE.md                     │    │    │
│  │  │  ══════════════════                     │    │    │
│  │  │  "Was ist der aktuelle Stand?"          │    │    │
│  │  │  Patterns, Konventionen, Datenmodell    │    │    │
│  │  │  Edge Cases, Präzedenzregeln            │    │    │
│  │  │                                         │    │    │
│  │  └─────────────────────────────────────────┘    │    │
│  │                                                 │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

**Ebene 1 (CLAUDE.md)** definiert das "Jetzt" — den aktuellen Stand des Systems, Patterns und Regeln. Claude liest sie bei jeder Session.

**Ebene 2 (Proposals)** definiert das "Wie" — detaillierte Pläne für einzelne Features. Claude liest sie, wenn ich ein Feature implementieren will.

**Ebene 3 (Roadmap)** definiert das "Was" — die strategische Richtung. Ich lese sie, um Prioritäten zu setzen.

## Lessons Learned

### 1. Investiere in die CLAUDE.md — es zahlt sich exponentiell aus

Je sauberer die CLAUDE.md, desto weniger Korrekturen braucht der generierte Code. Der Aufwand für die Pflege ist minimal (ein paar Zeilen nach jedem größeren Feature), der Nutzen ist enorm (konsistenter Code über hunderte Commits).

### 2. Sei spezifisch, nicht vage

Schlecht: *"Die App speichert Metadaten."*

Gut: *"Dual storage: YAML sidecars (source of truth) + SQLite catalog (derived cache). YAML is human-readable and survives rebuild-catalog. SQLite is fast for queries. Both must be updated on every write."*

Der zweite Satz gibt Claude genug Information, um jedes neue Feature korrekt zu implementieren — inklusive der beiden Speicherorte.

### 3. Dokumentiere Entscheidungen, nicht nur Ergebnisse

Das Storage-Workflow-Proposal erklärt, *warum* Volume Purpose besser ist als eine Backup-Varianten-Rolle. Wenn Claude sechs Wochen später ein verwandtes Feature baut, kann es die Begründung lesen und die gleiche Design-Philosophie anwenden.

### 4. Halte Proposals aktuell

Ein veraltetes Proposal ist schlimmer als kein Proposal. Wenn Claude einen Plan liest, in dem Feature X als "geplant" markiert ist, obwohl es seit drei Versionen implementiert ist, produziert das Verwirrung und doppelten Code.

### 5. Die CLAUDE.md ist kein Ersatz für guten Code

Letztlich muss der Code selbst lesbar und konsistent sein. Die CLAUDE.md hilft Claude, sich in einem großen Codebase zu orientieren — aber sie kann schlecht strukturierten Code nicht kompensieren. Die besten Ergebnisse erzielt man, wenn CLAUDE.md und Code die gleiche Geschichte erzählen.

## Fazit

Kontextmanagement ist der entscheidende Unterschied zwischen "Claude schreibt brauchbaren Code" und "Claude schreibt Code, der nahtlos ins Projekt passt". Die CLAUDE.md mit ihren gut 200 Zeilen und die acht Proposals mit zusammen 2.300 Zeilen waren das beste Investment in diesem Projekt — nicht gemessen an der Schreibzeit, sondern an den hunderten Korrekturen, die dadurch unnötig wurden.

Im [nächsten Artikel](/tipps/2026/03/14/architekturentscheidungen/) geht es darum, wie man Claude bei Architekturentscheidungen führt statt folgt — denn auch die beste CLAUDE.md ersetzt nicht die menschliche Designkompetenz.

---

*Thomas Herrmann ist Geschäftsführer der [42ways GmbH](https://42ways.de) und beschäftigt sich mit dem praktischen Einsatz von KI in der Softwareentwicklung.*
