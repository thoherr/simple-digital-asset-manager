---
layout: single
title: "Web-UI-Entwicklung: Wie Claude mit Askama-Templates, CSS und JavaScript umgeht"
date: 2026-03-24
categories:
  - tipps
tags:
  - Agentic Coding
  - AI Coding
  - Claude
  - Claude Code
  - Web UI
  - Askama
  - htmx
  - Rust
---

Die Web-Oberfläche des [DAM-Projekts](/tipps/2026/03/04/dam-erfahrungsbericht/) umfasst 6.200 Zeilen HTML-Templates und 2.900 Zeilen CSS — mit Dark Mode, Lightbox, Kartenansicht, Kalender-Heatmap, Keyboard-Navigation und Batch-Operationen. Alles entstanden in Pair Programming mit Claude Code. Dieser Artikel zeigt die Patterns, die dabei funktioniert haben — und die Stellen, wo es knifflig wurde.

## Der Tech-Stack: Minimalistisch aber mächtig

```
┌───────────────────────────────────────────────────────┐
│                    Browser                            │
│  ┌────────────┐  ┌──────────┐  ┌───────────────────┐ │
│  │  HTML       │  │  CSS     │  │  JavaScript       │ │
│  │  (Askama)   │  │  (Vanilla)│  │  (htmx + Vanilla)│ │
│  └─────┬──────┘  └──────────┘  └────────┬──────────┘ │
│        │                                │             │
├────────┴────────────────────────────────┴─────────────┤
│                   Axum Server                         │
│  ┌────────────────────┐  ┌─────────────────────────┐  │
│  │ Routes             │  │ Static Assets           │  │
│  │  (HTML + JSON)     │  │  (htmx.min.js,          │  │
│  │                    │  │   style.css, Leaflet)   │  │
│  └────────────────────┘  └─────────────────────────┘  │
│                                                       │
│  Kein React. Kein Webpack. Kein npm.                  │
└───────────────────────────────────────────────────────┘
```

Bewusst kein Frontend-Framework. Die gesamte Interaktivität kommt von **htmx** (für partielle Seitenaktualisierungen) und **Vanilla JavaScript** (für Keyboard-Navigation, Batch-Operationen, Lightbox). Statische Assets werden **zur Compile-Zeit eingebettet** — kein Dateisystem-Zugriff zur Laufzeit nötig.

## Pattern 1: Askama Template-Vererbung

Askama ist ein Compile-Time-Template-System für Rust. Templates werden in HTML geschrieben und zur Compile-Zeit in Rust-Code übersetzt. Fehler in Templates werden vom Compiler gefunden, nicht erst zur Laufzeit.

```
templates/
├── base.html              ← Grundlayout (Nav, Theme, Help)
├── browse.html            ← Suchseite (~3000 Zeilen)
├── asset.html             ← Detailseite
├── results.html           ← Ergebnis-Fragment (für htmx)
├── rating_fragment.html   ← Sterne-Widget
├── label_fragment.html    ← Farbpunkte-Widget
├── name_fragment.html     ← Editierbarer Name
├── tags.html              ← Tag-Übersicht
├── collections.html       ← Sammlungen
├── compare.html           ← Vergleichsansicht
├── duplicates.html        ← Duplikate-Seite
├── saved_searches.html    ← Gespeicherte Suchen
└── backup.html            ← Backup-Status
```

Die Basis-Vorlage (`base.html`) definiert das Layout:

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <!-- FOUC-Prevention: Theme vor CSS laden -->
    <script>
        var t = localStorage.getItem('maki-theme');
        if (t) document.documentElement.setAttribute('data-theme', t);
    </script>
    <link rel="stylesheet" href="/static/style.css">
    <script src="/static/htmx.min.js"></script>
</head>
<body>
    <nav><!-- Navigation --></nav>
    {% block content %}{% endblock %}
    {% block scripts %}{% endblock %}
</body>
</html>
```

Ein entscheidendes Detail: Das **Theme-Script steht vor dem CSS**. Sonst blitzt die Seite kurz im hellen Modus auf, bevor Dark Mode aktiviert wird (FOUC — Flash of Unstyled Content).

## Pattern 2: htmx für partielle Updates

Die Browse-Seite nutzt htmx für Suche, Pagination und Sortierung — ohne vollständige Seitenneuladezeit.

```
Seitenaufruf: GET /?q=sunset&sort=date_desc

  Browser                           Server
    │                                  │
    ├── GET / ─────────────────────▶   │
    │   (normaler Request)             │
    │                                  ├── Volle BrowsePage rendern
    │   ◀──── HTML (Nav+Search+Grid) ──┤
    │                                  │
    │                                  │
    ├── GET /?q=sunset&page=2 ─────▶   │
    │   HX-Request: true               │
    │                                  ├── Nur ResultsPartial rendern
    │   ◀──── HTML (nur Grid+Paging) ──┤
    │                                  │
    │   htmx tauscht #results aus      │
    │                                  │
```

Die Server-Logik erkennt htmx-Requests am Header:

```rust
pub async fn browse_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let is_htmx = headers.get("HX-Request").is_some();

    // ... Suche ausführen ...

    if is_htmx {
        // Nur das Grid + Pagination zurückgeben
        ResultsPartial { cards, total, page, ... }.render()
    } else {
        // Volle Seite mit Nav, Suchleiste, Grid
        BrowsePage { cards, total, page, ... }.render()
    }
}
```

**Warum das wichtig ist:** Wenn ein Benutzer die Zurück-Taste drückt oder ein Bookmark öffnet, bekommt er die **volle Seite** — nicht ein Fragment ohne CSS und Navigation. htmx-Requests bekommen nur das Fragment, was die Interaktion schnell macht.

Das HTML-Formular sammelt alle Filter-Werte per JavaScript:

```html
<form class="search-bar"
      hx-get="/"
      hx-target="#results"
      hx-push-url="true"
      hx-vals='js:{
          "q": document.querySelector("[name=q]").value,
          "tag": getSelectedTags().join(","),
          "rating": getFilterRating(),
          "label": getFilterLabel(),
          "sort": getCurrentSort(),
          "page": "1"
      }'>
```

`hx-push-url="true"` aktualisiert die Browser-URL — dadurch funktionieren Bookmarks und die Browser-Historie korrekt.

## Pattern 3: Dark Mode mit CSS Custom Properties

Statt für jedes Element eigene Dark-Mode-Regeln zu schreiben, nutzen wir CSS Custom Properties:

```css
/* Helles Theme (Standard) */
:root {
    --bg: #ffffff;
    --bg-card: #f8f9fa;
    --text: #000000;
    --border: #dee2e6;
    --nav-bg: #212529;
}

/* Dunkles Theme */
[data-theme="dark"] {
    --bg: #1a1b2e;
    --bg-card: #252640;
    --text: #e1e4eb;
    --border: #3d3e5c;
    --nav-bg: #13142a;
    color-scheme: dark;
}

/* Systemeinstellung als Fallback */
@media (prefers-color-scheme: dark) {
    html:not([data-theme]) {
        --bg: #1a1b2e;
        --text: #e1e4eb;
        /* ... */
    }
}
```

Komponenten verwenden nur die Variablen:

```css
.asset-card {
    background: var(--bg-card);
    color: var(--text);
    border: 1px solid var(--border);
}
```

```
Theme-Hierarchie:

  1. Explizite Wahl (localStorage)     ← höchste Priorität
                ↓
  2. data-theme Attribut auf <html>
                ↓
  3. OS-Einstellung (prefers-color-scheme)  ← Fallback
                ↓
  4. Hell (Standard)                   ← Default
```

Der Toggle-Button im Header speichert die Wahl in `localStorage` und setzt das `data-theme`-Attribut:

```javascript
btn.addEventListener('click', function() {
    var next = getEffective() === 'dark' ? 'light' : 'dark';
    document.documentElement.setAttribute('data-theme', next);
    localStorage.setItem('maki-theme', next);
});
```

## Pattern 4: Grid Density mit CSS-Variablen

Die Browse-Ansicht bietet drei Dichten: Compact, Normal, Large. Statt drei verschiedene Grid-Layouts:

```css
/* Eine einzige Grid-Definition */
.results-grid {
    display: grid;
    grid-template-columns:
        repeat(auto-fill, minmax(var(--grid-min, 200px), 1fr));
    gap: var(--grid-gap, 1rem);
}

/* Dichte steuert nur die Variable */
[data-density="compact"] { --grid-min: 120px; --grid-gap: 0.5rem; }
[data-density="large"]   { --grid-min: 300px; }

/* Und selektiv Elemente ein-/ausblenden */
[data-density="compact"] .card-meta { display: none; }
```

Claude hat dieses Pattern von sich aus vorgeschlagen — CSS Custom Properties sind eine Stärke von Claude, weil es die Browser-Spezifikationen gut kennt.

## Pattern 5: Keyboard-Navigation

Die Keyboard-Navigation war das technisch anspruchsvollste Feature. Sie muss mit dem CSS-Grid, der Lightbox, der Batch-Auswahl und htmx-Updates zusammenspielen.

```
Tastatur-Interaktionen und ihre Guards:

  Taste gedrückt
       ↓
  ┌── Lightbox offen? ──▶ Lightbox-Shortcuts (Esc, ←, →, d)
  │        nein
  │
  ├── Help-Panel offen? ──▶ Nur Esc zum Schließen
  │        nein
  │
  ├── Input-Feld fokussiert? ──▶ Normale Texteingabe
  │        nein
  │
  └── Grid-Navigation
       ├── Pfeiltasten → Fokus verschieben (spaltenbasiert!)
       ├── Enter/l → Lightbox öffnen
       ├── d → Detailseite
       ├── Space → Selektion toggeln
       ├── 0-5 → Rating setzen (Fokus oder Batch)
       ├── Alt+1-7 → Label setzen
       └── r/o/y/g/b/p/u → Label per Farbanfangsbuchstabe
```

Die Spaltenberechnung für Pfeil-Auf/Ab ist ein typisches Detail, das Claude korrekt implementiert hat:

```javascript
function getColCount() {
    var grid = document.querySelector('.results-grid');
    if (!grid) return 1;
    return getComputedStyle(grid)
        .gridTemplateColumns.split(' ').length;
}

// Pfeil nach oben: nicht -1, sondern -cols
if (e.key === 'ArrowUp')
    setFocus(focusedIndex - getColCount());
```

### Das Zusammenspiel der JavaScript-Module

Die Web-Oberfläche besteht aus mehreren JavaScript-IIFEs, die über globale Objekte kommunizieren:

```
┌──────────────────────────────────────────────────────┐
│                  window.damBatch                      │
│  ┌────────────────────────────────────────────────┐  │
│  │ selected: Set<assetId>                         │  │
│  │ updateToolbar()                                │  │
│  │ clearSelection()                               │  │
│  │ refreshResults()                               │  │
│  └────────────────────┬───────────────────────────┘  │
│                       │ liest                         │
│  ┌────────────────────┴───────────────────────────┐  │
│  │              window.damKeyNav                   │  │
│  │  setFocusById(id)                              │  │
│  │  getFocusedId()                                │  │
│  └────────────────────┬───────────────────────────┘  │
│                       │ öffnet                        │
│  ┌────────────────────┴───────────────────────────┐  │
│  │              window.damLightbox                 │  │
│  │  open(index)                                   │  │
│  │  isOpen()                                      │  │
│  └────────────────────┬───────────────────────────┘  │
│                       │ registriert                   │
│  ┌────────────────────┴───────────────────────────┐  │
│  │              window.damHelp                     │  │
│  │  registerShortcuts(shortcuts)                  │  │
│  │  isOpen()                                      │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │              window.damFacets                   │  │
│  │  toggle()                                      │  │
│  │  isOpen()                                      │  │
│  └────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────┘
```

Jedes Modul prüft die anderen vor der Aktion: Keyboard-Navigation ignoriert Tasten, wenn die Lightbox offen ist. Die Lightbox ignoriert Tasten, wenn das Help-Panel offen ist. Batch-Operationen werden gesperrt, wenn gerade ein API-Call läuft.

## Pattern 6: State-Persistenz über Navigation hinweg

Ein subtiles Problem: Wenn ein Benutzer auf der Browse-Seite 5 Assets ausgewählt hat, dann ein Asset im Detail anschaut und zurückkommt — soll die Auswahl noch da sein.

```
Browse (5 ausgewählt)
    │
    ├── Asset-Detail öffnen
    │       │
    │       ├── sessionStorage: maki-browse-selection = [id1,...,id5]
    │       ├── sessionStorage: maki-browse-focus = id3
    │       ├── sessionStorage: maki-browse-url = /?q=sunset&page=2
    │       │
    │       └── Zurück-Button / Escape
    │               │
    └── pagehide Event ◀───┘
            │
            └── Auswahl aus sessionStorage wiederherstellen
```

```javascript
// Vor dem Verlassen: State speichern
window.addEventListener('pagehide', function() {
    sessionStorage.setItem('maki-browse-selection',
        JSON.stringify(Array.from(selected)));
});

// Beim Zurückkehren (bfcache): State wiederherstellen
window.addEventListener('pageshow', function(e) {
    if (e.persisted) {
        var saved = sessionStorage.getItem('maki-browse-selection');
        if (saved) {
            selected = new Set(JSON.parse(saved));
            updateToolbar();
        }
    }
});
```

## Wo Claude bei Web-UI Unterstützung braucht

### 1. Inline-Styles vs. CSS-Klassen

Claude neigt dazu, bei schnellen Fixes `style="..."` direkt ins HTML zu schreiben. Das muss man aktiv korrigieren — Inline-Styles brechen das Dark-Mode-System, weil sie keine CSS-Variablen nutzen.

### 2. Event-Delegation nach htmx-Swaps

Wenn htmx Teile der Seite ersetzt, gehen Event-Listener verloren, die direkt auf den Elementen registriert waren. Claude hat das anfangs nicht bedacht und z.B. Click-Handler auf Karten registriert, die nach dem nächsten Seitenwechsel nicht mehr funktionierten.

Die Lösung: **Event-Delegation** auf einem stabilen Parent-Element:

```javascript
// Falsch: Handler auf jedem Element
document.querySelectorAll('.card').forEach(function(card) {
    card.addEventListener('click', handler);  // geht nach htmx-Swap verloren
});

// Richtig: Delegation auf stabilem Container
document.getElementById('results')
    .addEventListener('click', function(e) {
        var card = e.target.closest('.asset-card');
        if (card) handler(card);
    });
```

### 3. Die 3000-Zeilen-Template-Datei

`browse.html` ist mit ~3000 Zeilen die größte Datei im Projekt. Sie enthält HTML, CSS und JavaScript für die Browse-Seite, Lightbox, Keyboard-Navigation, Batch-Operationen, Kartenansicht, Kalenderansicht und Facetten-Sidebar.

Das ist zu groß. In einem Folgeprojekt würde ich die JavaScript-Module in separate Dateien auslagern. Aber Claude hat das monolithische Template korrekt verwaltet — es findet die richtigen Stellen für Änderungen auch in einer 3000-Zeilen-Datei.

## Metriken

```
Web-UI Komponenten:

  Browse-Seite ────────── ~3000 Zeilen (HTML/CSS/JS)
  Asset-Detail ────────── ~900 Zeilen
  Vergleichsansicht ───── ~650 Zeilen
  Duplikate-Seite ─────── ~500 Zeilen
  Basis-Template ──────── ~150 Zeilen
  Styles ──────────────── ~2900 Zeilen CSS
  Weitere Templates ───── ~1000 Zeilen

  Gesamt: ~9100 Zeilen Frontend-Code

  Features:
  ─────────
  ✓ Dark Mode mit OS-Fallback
  ✓ 3 Grid-Dichten
  ✓ Navigierbare Lightbox mit Zoom
  ✓ Keyboard-Navigation (30+ Shortcuts)
  ✓ Batch-Operationen (Tag, Rating, Label)
  ✓ Inline-Editing (Name, Description, Date)
  ✓ Kalender-Heatmap
  ✓ OpenStreetMap-Integration
  ✓ Facetten-Sidebar
  ✓ Vergleichsansicht (2-4 Assets)
  ✓ Gespeicherte Suchen
  ✓ Collections mit Drag-Target
```

## Lessons Learned

**1. htmx ist ideal für AI-gestütztes Pair Programming.** Kein Build-System, keine Kompilierung, kein State-Management-Framework. Claude schreibt HTML-Attribute und ein paar Zeilen Server-Code — die Interaktivität kommt gratis.

**2. CSS Custom Properties vor Inline-Styles.** Einmal als Konvention etabliert, hält sich Claude daran. Aber man muss die Konvention in der CLAUDE.md verankern.

**3. Event-Delegation von Anfang an.** Jeder Event-Handler, der auf dynamischem Content arbeitet, muss auf einem stabilen Parent-Element delegiert werden. Das ist ein häufiger Fehler, den Claude macht und der erst beim zweiten htmx-Swap auffällt.

**4. Globale Namespaces für JS-Module.** `window.damBatch`, `window.damKeyNav`, `window.damLightbox` — einfach, explizit, ohne Bundler. Claude kann die Module referenzieren und erweitern, ohne ein Import-System zu verstehen.

**5. Template-Fragmente für Inline-Editing.** Statt JavaScript-State für jeden editierbaren Wert: Ein Server-gerendertes HTML-Fragment, das htmx bei Bedarf austauscht. Rating-Sterne, Color-Labels, Names — alles Fragmente.

Im [nächsten und letzten Artikel](/tipps/2026/03/29/ai-integration/) dieser Serie geht es um die Integration des SigLIP-Modells — wie wir mit Claude ein ONNX-Modell in eine Rust-Anwendung eingebaut haben.

---

*Thomas Herrmann ist Geschäftsführer der [42ways GmbH](https://42ways.de) und beschäftigt sich mit dem praktischen Einsatz von KI in der Softwareentwicklung.*
