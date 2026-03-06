---
layout: single
title: "In 17 Tagen zum fertigen Digital Asset Manager: Ein Erfahrungsbericht mit Claude Code"
date: 2026-03-04
categories:
  - tipps
tags:
  - Agentic Coding
  - AI Coding
  - Claude
  - Claude Code
  - Rust
  - Pair Programming
  - Erfahrungsbericht
---

Als Fotograf mit einer wachsenden Sammlung von RAW-Dateien über mehrere externe Festplatten hinweg hatte ich ein konkretes Problem: Kein existierendes Tool konnte meine Bilder zuverlässig verwalten, wenn die Datenträger nicht angeschlossen waren. Lightroom und CaptureOne setzen voraus, dass die Medien erreichbar sind. Was ich brauchte, war ein schlankes System, das Metadaten, Vorschaubilder und Suchfunktionen bietet, auch wenn die eigentlichen Dateien auf einer Festplatte im Schrank liegen.

Also habe ich es selbst gebaut. Mit Claude Code als Programmierpartner. In 17 Tagen, nebenbei, in Arbeitspausen und Abendstunden.

## Das Ergebnis in Zahlen

Bevor ich auf den Prozess eingehe, ein Blick auf das, was dabei herausgekommen ist:

- **34.500 Zeilen Rust-Quellcode** in 30 Modulen
- **6.200 Zeilen HTML-Templates** für die Web-Oberfläche
- **2.900 Zeilen CSS** inklusive Dark Mode
- **8.500 Zeilen Integrationstests**
- **693 automatisierte Tests** (465 Unit-Tests, 228 Integrationstests)
- **32 CLI-Befehle** mit Subbefehlen
- **310 Commits**, **30 Releases** von v1.0.0 bis v2.1.0
- **17 Arbeitstage** vom ersten `git init` bis zum AI-gestützten Auto-Tagging

Das System kann importieren, deduplizieren, taggen, bewerten, nach Ort und Zeit filtern, Duplikate auflösen, Backups analysieren, Bilder mit KI verschlagworten, und das alles sowohl über die Kommandozeile als auch über eine Web-Oberfläche mit Lightbox, Kartenansicht, Kalender-Heatmap und Vergleichsansicht.

Und: Das Ganze ist kein Vollzeit-Projekt. Es ist komplett "nebenbei" entstanden, in 10-Minuten-Pausen zwischen Meetings, abends auf dem Sofa, und zu einem großen Teil auf zwei Zugfahrten zwischen München und Berlin. Wer schon einmal fünf Stunden im ICE saß und WLAN hatte, weiß: Das ist produktive Arbeitszeit, wenn man einen fokussierten Dialog mit einem KI-Assistenten führt statt E-Mails zu beantworten.

Inzwischen ist das System im produktiven Einsatz. In der letzten Woche habe ich über 250.000 Fotodateien importiert: RAW-Dateien, JPEGs, TIFFs, Videos aus über zehn Jahren Fotografie, verteilt auf ein halbes Dutzend externe Festplatten. Das System hilft mir jetzt dabei, diese Menge an Assets zu überblicken, zu klassifizieren, zu sortieren und auszumisten. Genau dafür habe ich es gebaut.

## Wie fängt man so ein Projekt an?

Nicht mit Code. Sondern mit einer Diskussion. Mein erster Schritt war, Claude mein Problem zu beschreiben: Große Fotosammlung, mehrere Volumes, Offline-Browsing, Content-Hashing, Metadaten in Textdateien. Claude hat daraufhin eine Architektur vorgeschlagen, Content-addressable Storage mit SHA-256, YAML-Sidecar-Dateien als "Source of Truth", SQLite als abgeleiteten Cache, und wir haben die Details durchdiskutiert.

Aus dieser Diskussion entstand ein 30-seitiges Architekturdokument und eine detaillierte Komponentenspezifikation, noch bevor die erste Zeile Code geschrieben wurde. Das war keine verlorene Zeit: Diese Dokumente haben Claude während der gesamten Entwicklung als Kontext gedient. Sie sind der Grund, warum neue Features konsistent zur bestehenden Architektur passen.

## Pair Programming mit einem KI-Partner

Die Arbeitsweise hat sich schnell eingependelt: Ich beschreibe, was ich will. Claude schlägt eine Implementierung vor, oft im "Plan Mode", einem Modus, in dem zunächst die betroffenen Dateien, Code-Stellen und Änderungen aufgelistet werden, bevor Code geschrieben wird. Ich gebe Feedback, wir justieren, dann wird implementiert.

Das erinnert an Pair Programming mit einem guten Junior-Entwickler, der die Dokumentation auswendig kennt, aber manchmal den Überblick über das große Ganze verliert.

### Wo Claude stark ist

**Konsistenz im Codebase.** Claude liest bestehenden Code und passt neue Features an vorhandene Patterns an. Als ich zum Beispiel sagte "Füge einen Auto-Tag-Button in die Batch-Toolbar ein", hat Claude selbstständig das bestehende Muster der Batch-Operationen (Tag, Rating, Label) analysiert und den neuen Button inklusive Confirm-Dialog, Fehlerbehandlung und Grid-Refresh exakt nach dem gleichen Schema implementiert.

**Präzise Anforderungsformulierung.** In einer Session habe ich relativ ausführlich ein Problem mit verwaisten XMP-Dateien beschrieben: Wenn man in CaptureOne arbeitet, landen die XMP-Sidecars manchmal in anderen Verzeichnissen als die zugehörigen RAW-Dateien. Beim Import werden sie dann als eigenständige Assets erfasst statt als Rezepte an das richtige Bild angehängt.

Claude hat meine Problembeschreibung auf eine knappe, präzise formulierte Anforderung eingedampft: *Standalone recipe resolution, when a recipe file is imported without a co-located media file, the system finds the parent variant by matching the filename stem and directory, and attaches the recipe to it.* Diese Formulierung war nicht nur kürzer, sie war auch besser. Sie hat Edge Cases abgedeckt (compound extensions wie `DSC_001.NRW.xmp`), die ich in meiner Beschreibung nicht erwähnt hatte, weil mir klar war, was ich meinte, aber dem Code war es nicht klar.

**Testabdeckung.** Claude schreibt automatisch Tests für neue Features. Nicht weil ich es jedes Mal verlange, sondern weil es zum etablierten Pattern gehört. Nach 17 Tagen hatte das Projekt 693 automatisierte Tests, die bei jedem Build durchlaufen, eine Abdeckung, die ich bei einem Solo-Projekt dieser Geschwindigkeit manuell nicht erreicht hätte.

### Wo Claude Unterstützung braucht

**Architekturentscheidungen.** Wenn eine Änderung mehrere Dateien betrifft und es verschiedene Lösungswege gibt, schlägt Claude manchmal den naheliegenden, aber nicht den besten Weg vor. Bei der Implementierung der denormalisierten Spalten auf der `assets`-Tabelle (um teure JOINs in der Suche zu vermeiden) musste ich die Richtung vorgeben. Claude hätte von sich aus weiter mit JOINs gearbeitet.

**Schleichende Komplexität.** Ab einer gewissen Projektgröße musste ich aufpassen, dass neue Features nicht unnötig komplex werden. Claudes Tendenz, "sicherheitshalber" zusätzliche Abstraktionen oder Fallback-Pfade einzubauen, muss man aktiv bremsen. Ein einfaches "Keep it simple" in der Konfigurationsdatei (`CLAUDE.md`) hat hier gut funktioniert.

**Feature-übergreifende Wechselwirkungen.** Wenn ein Feature existierenden Code auf subtile Weise verändert, passieren Fehler. Beispiel: Beim Hinzufügen der AI-Auto-Tag-Funktion für die Web-Oberfläche gab es einen Compilerfehler, weil der Axum-Router seinen Typ ändert, wenn man `with_state()` aufruft. Claude hat den Fehler erst beim dritten Anlauf richtig gelöst, zunächst wurde der falsche Variable-Typ wiederverwendet, dann ein `mut`-Warning ignoriert. Solche Fehler findet der Compiler zuverlässig, aber es zeigt, dass man den generierten Code nicht blind übernehmen sollte.

## Die Entwicklungskurve

Die Progression war recht gleichmäßig:

**Tag 1–2 (15.–16. Feb):** Grundgerüst. `init`, `volume`, `import`, `search`, `show`, `tag`, `group`, EXIF-Extraktion, `rebuild-catalog`. 20 Commits. Am Ende von Tag 2 war das System als CLI benutzbar.

**Tag 3–8 (17.–23. Feb):** Kernfunktionen. XMP-Integration, Duplikaterkennung, Web-UI mit Browse/Detail/Tags, Suche mit Filtern, Batch-Operationen, Collections, Saved Searches. Ein großer Teil davon ist auf einer Zugfahrt nach Berlin (20. Feb) und zurück nach München (22. Feb) entstanden, jeweils fünf Stunden konzentrierter Dialog mit Claude im ICE. Release v1.0.0 am 23. Februar.

**Tag 9–13 (24.–28. Feb):** Erweiterte UI. Dark Mode, Lightbox, Kalender-Heatmap, Stacks, hierarchische Tags, Compare View, Smart Previews, Deduplizierungs-UI. Releases v1.3 bis v1.7.

**Tag 14–16 (1.–2. März):** Map View, Facetten-Sidebar, Export, Format-Filter, Delete-Command. Releases v1.8.0 bis v1.8.9.

**Tag 17 (3. März):** AI-Autotagging mit SigLIP via ONNX Runtime, Visual Similarity Search, Web-UI-Integration. Releases v2.0.0 bis v2.1.0.

Im Schnitt wurden pro Tag **18 Commits** gemacht und fast **2 Releases** veröffentlicht. Das entspricht nicht dem typischen Solo-Entwickler-Tempo.

## Wie steuert man ein solches Projekt?

Drei Mechanismen haben sich bewährt:

### 1. Die CLAUDE.md als lebendes Dokument

Die Datei `CLAUDE.md` im Projektstamm ist Claudes Gedächtnis. Sie enthält Architekturentscheidungen, Coding-Konventionen, das Datenmodell und den aktuellen Projektstand. Claude liest diese Datei bei jeder Session und richtet sich danach. Wenn ich "Avoid over-engineering" in die CLAUDE.md schreibe, hält sich Claude daran. Wenn ich ein neues Pattern etabliere (z.B. "denormalized columns are updated in all write paths"), wird es dort dokumentiert und von Claude in Zukunft beachtet.

Diese Datei ist auf über 100 Zeilen gewachsen, ein fein granulares Regelwerk, das die Konsistenz des Codes über Wochen und hunderte Commits hinweg sicherstellt.

### 2. Plan Mode vor jeder größeren Änderung

Für nicht-triviale Features nutze ich Claudes "Plan Mode": Claude analysiert den bestehenden Code, identifiziert die betroffenen Dateien und skizziert die Änderungen. Ich gebe das OK, bevor Code geschrieben wird. Das verhindert, dass Claude in die falsche Richtung läuft und ich nach 200 geänderten Zeilen feststelle, dass der Ansatz nicht passt.

### 3. Proposals und Roadmap

Für größere Features schreibe ich Proposals, kurze Dokumente, die das Problem, den Lösungsansatz und die Schnittstellen beschreiben. Claude kann diese lesen und daraus eine Implementierung ableiten. Die Roadmap-Datei im Projektverzeichnis zeigt den Überblick und hilft, Prioritäten zu setzen. Von 14 geplanten Enhancements sind 13 implementiert, das letzte (Drag-and-Drop) steht noch aus.

## Was habe ich gelernt?

**Claude ist kein Autopilot.** Es ist ein Werkzeug, das die Produktivität vervielfacht, aber Erfahrung und Urteilsvermögen auf der menschlichen Seite voraussetzt. Ich musste Architekturentscheidungen treffen, Prioritäten setzen, und regelmäßig "Nein, einfacher" sagen.

**Die Qualität hängt vom Kontext ab.** Je besser die Spezifikation und je sauberer die bestehende Codebasis, desto besser wird Claudes Output. Das Investment in Architektur-Dokumente und CLAUDE.md zahlt sich aus, mit jedem Feature mehr.

**Rust als Sprache passt hervorragend.** Der Compiler fängt viele Fehler ab, die bei dynamischen Sprachen durchrutschen würden. Wenn Claude einen Typ falsch verwendet oder einen Ownership-Fehler macht, sagt `cargo build` sofort Bescheid. Das ist ein Sicherheitsnetz, das bei Python oder JavaScript fehlt.

**Eine Vervielfachung meiner Entwicklungsgeschwindigkeit ist tatsächlich erreichbar.** 34.500 Zeilen produktiver Rust-Code in 17 Tagen, mit Tests, Dokumentation und Web-UI, entstanden in Arbeitspausen, Abendstunden und auf Zugfahrten. Keine einzige Vollzeit-Woche. Das wäre ohne AI-Unterstützung nicht möglich gewesen, jedenfalls nicht in dieser Qualität und diesem Zeitraum.

## Ausblick

Dieses Projekt ist ein Einzelfall, ein technisch versierter Entwickler mit einem klar definierten Problem und der richtigen Sprache für den Job. Aber es zeigt, was mit den heutigen Tools möglich ist, wenn man sie richtig einsetzt. In weiteren Artikeln dieser Serie werde ich auf einzelne Aspekte eingehen:

- **Kontextmanagement**: Wie die CLAUDE.md und Proposals als Steuerungsinstrument funktionieren
- **Architekturentscheidungen**: Wie man Claude bei Design-Fragen führt statt folgt
- **Testing und Qualität**: Wie automatisierte Tests den AI-Workflow absichern
- **Web-UI-Entwicklung**: Wie Claude mit Askama-Templates, CSS und JavaScript umgeht
- **AI-Integration**: Wie wir mit Claude ein SigLIP-Modell in eine Rust-Anwendung integriert haben

Das DAM-Projekt ist [Open Source auf GitHub](https://github.com/thoherr/simple-digital-asset-manager), wer sich den Code und die Commit-Historie ansehen möchte, kann die gesamte Entwicklung nachvollziehen.

## Weiterlesen

- [Mein KI-Praktikant Claude: Erfahrungen aus der täglichen Softwareentwicklung]({% post_url 2025-10-14-claude_experience %}) — ein persönlicher Erfahrungsbericht über den Einsatz von Claude in der täglichen Entwicklungsarbeit.
- [Whitepaper: Agentic Coding]({% post_url 2026-01-28-agentic_coding_whitepaper %}) — unser kostenloses Whitepaper erklärt, was Agentic Coding bedeutet und wie es sich vom klassischen Prompting unterscheidet.
- [Schneller als man prompten kann: Die Entwicklung von Coding-Modellen am Beispiel Claude]({% post_url 2026-02-24-claude-history %}) — die rasante Entwicklung der KI-Modelle für die Softwareentwicklung nachgezeichnet an der Claude-Familie.

Sie möchten KI-gestützte Entwicklung in Ihrem Team einführen? [Sprechen Sie uns an](/contact/) — wir unterstützen Sie bei der Auswahl und dem Einsatz der richtigen Tools.

---

*Thomas Herrmann ist Geschäftsführer der [42ways GmbH](https://42ways.de) und beschäftigt sich mit dem praktischen Einsatz von KI in der Softwareentwicklung.*
