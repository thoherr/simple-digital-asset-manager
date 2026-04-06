# Tagging Guide

Tags are the backbone of discoverability in a large photo catalog. Ratings tell you how good an image is; tags tell you what it *is*. A thoughtful tagging strategy pays compound interest -- every tag you apply today makes thousands of future searches faster. This chapter covers principles, a recommended vocabulary structure, and practical techniques for building and maintaining tags across a growing collection.

For the mechanics of adding, removing, and searching tags, see [Organizing Assets](04-organize.md). This chapter focuses on *what* to tag and *how to think about it*.

---

## Why Tags Matter

A 100-image portfolio doesn't need tags. A 10,000-image catalog can get by with folder names and memory. Beyond that, you need structured metadata to find what you're looking for -- and tags are the most flexible tool for the job.

Tags serve three purposes:

1. **Search** -- finding images by subject, location, technique, or concept. "Show me all concert photos from 2023 with dramatic lighting."
2. **Navigation** -- browsing by category. The tags page gives you an overview of what your catalog contains.
3. **Interoperability** -- IPTC keywords (`dc:subject`) are the industry standard for metadata exchange. Tags written by MAKI travel with your images to Lightroom, CaptureOne, stock agencies, and archives.

---

## Principles

### Tag what you see, not what you know

Primary tags should describe the visible content of the image: *sunset*, *concert*, *portrait*, *bridge*. Secondary tags can capture context that isn't visible: event names, project names, or moods. But the foundation is always observable content.

### Be specific and general at the same time

Hierarchical tags handle this naturally. Tagging an image `subject|animal|bird|heron` automatically stores all ancestor paths (`subject`, `subject|animal`, `subject|animal|bird`) alongside the full tag -- matching the CaptureOne/Lightroom convention. This means the image will match searches for `tag:subject`, `tag:subject|animal`, `tag:subject|animal|bird`, and `tag:subject|animal|bird|heron`.

### Use one language consistently

Mixed-language tags (Konzert *and* concert, München *and* Munich) double your vocabulary without adding information. Pick one language for your tag vocabulary and stick with it. English is the practical default for descriptive terms -- IPTC keywords, stock agencies, and most tools expect English.

The exception is **place names**: use **English for countries**, and **local names from regions downward**. Countries always have stable, unambiguous English names (`Germany`, `Italy`, `Japan`). Below that, local names avoid the constant judgment call of whether a place "has" an English name — famous cities do, small towns don't, and the cutoff is arbitrary. Local names match what you see on signs, maps, and receipts.

- Country: English — `Germany` not `Deutschland`
- Region: local — `Bayern` not `Bavaria`
- City: local — `München` not `Munich`, `Holzkirchen` stays `Holzkirchen`
- Venue: local — `Kulturbühne Hinterhalt`

**Note on regions:** whether to use English (`Bavaria`) or local (`Bayern`) for well-known regions is a matter of preference. Both are defensible. The important thing is to decide once and apply it consistently across your vocabulary. The recommendation above (local from regions down) is the simplest rule with the fewest judgment calls.

The same principle applies to venue names and event names — use the original language.

### Use singular, lowercase forms -- except proper nouns

Pick a convention and stay consistent:

- `concert` not `concerts` or `Concert`
- `mountain` not `mountains` or `Mountain`
- `black and white` not `Black And White` or `B&W`

The one exception is **proper nouns**: place names, person names, venue names, and project names keep their natural capitalization. This follows normal language rules that everyone already knows -- no ambiguity about what gets capitalized:

- `location|Germany|Bayern|Gelting` -- country in English, local names below
- `subject|nature|landscape|mountain` -- generic terms lowercase
- `person|artist|Peter Schneider` -- person name capitalized
- `project|Focus on Music` -- project name capitalized

MAKI's tag search is case-insensitive for queries, but the stored tags should be consistent. A canonical form prevents duplicates from creeping in.

This convention applies to **all levels of the hierarchy**, including structural nodes. There is no visual distinction between "category" and "leaf" in casing -- that's a UI concern handled by indentation and tree display, not by the data.

### Separate content tags from workflow tags

Content tags describe the image: *landscape*, *wedding*, *jazz*. Workflow tags describe the image's state in your process: *unreviewed*, *to edit*, *portfolio candidate*. Keep them apart. Use a prefix like `@` for workflow tags, or better yet, use MAKI's ratings, color labels, and saved searches for workflow state instead of tags.

### Don't tag what metadata already captures

EXIF records the camera, lens, date, and GPS coordinates. MAKI indexes file paths and folder names. Don't duplicate this information as tags -- you'd just create a maintenance burden. Use search filters instead: `camera:Z9`, `lens:50mm`, `date:2024`, `path:Capture/2024-08-15`.

---

## Recommended Vocabulary Structure

A well-structured tag vocabulary has two layers:

1. **Top-level facets** that partition the world into non-overlapping categories (the "what / where / who / how" of an image).
2. **Hierarchical terms** within each facet, going 2-4 levels deep.

### The facets

```
subject          what is in the image?
location         where was it taken?
person           who is in it? (named individuals)
technique        how was it made?
project          what project or assignment does it belong to?
```

These five facets cover most tagging needs. Not every image needs all five -- a landscape might only need subject and location; a studio portrait might need subject, person, and technique.

### subject hierarchy

Subject is the largest facet. A starting structure for photography:

```
subject
├── nature
│   ├── landscape    (mountain, forest, beach, desert, valley, ...)
│   ├── flora        (flower, tree, leaf, moss, mushroom, ...)
│   ├── sky          (sunset, sunrise, cloud, fog, storm, aurora, ...)
│   └── water        (ocean, river, lake, waterfall, ...)
├── animal
│   ├── mammal       (deer, fox, bear, seal, whale, ...)
│   ├── bird         (eagle, owl, heron, kingfisher, swan, ...)
│   ├── reptile      (lizard, snake, turtle, frog, ...)
│   ├── invertebrate (butterfly, dragonfly, bee, spider, snail, ...)
│   ├── aquatic      (fish, jellyfish, coral, crab, ...)
│   └── domestic     (dog, cat, horse, cow, sheep, ...)
├── urban
│   ├── architecture (building, bridge, skyscraper, tower, facade, ...)
│   ├── street       (road, alley, graffiti, neon sign, shop front, ...)
│   └── transport    (car, bicycle, train, airplane, boat, ...)
├── person
│   ├── portrait     (headshot, environmental portrait, candid, ...)
│   ├── group        (family, couple, crowd, ...)
│   └── activity     (dance, sports, hiking, cooking, ...)
├── performing arts
│   ├── concert      (live music, musician, singer, guitarist, ...)
│   ├── theatre      (actor, stage set, rehearsal, costume, ...)
│   └── dance        (ballet, contemporary, ...)
├── event
│   ├── festival     (music festival, cultural festival, ...)
│   ├── exhibition   (art exhibition, photo exhibition, ...)
│   ├── wedding
│   ├── workshop     (photo workshop, ...)
│   └── sports event (marathon, match, tournament, ...)
├── object
│   ├── food         (coffee, wine, cake, cooking, restaurant, ...)
│   ├── instrument   (guitar, piano, drum, saxophone, ...)
│   └── other        (book, camera, flag, candle, sculpture, ...)
└── concept
    ├── travel
    ├── fashion
    ├── documentary
    └── abstract
```

You don't need all of these on day one. Start with the top two levels and add
leaf nodes as your collection demands them.

### location hierarchy

```
location
└── country
    └── region
        └── city
            └── venue   (optional, for recurring locations)
```

The structural levels are generic terms and lowercase. The actual values are proper nouns and capitalized:

Example: `location|Germany|Bayern|Gelting|Kulturbühne Hinterhalt`

Note how `location` (generic category) is lowercase, while `Germany` (country in English), `Bayern`, `Gelting`, and `Kulturbühne Hinterhalt` (local names) keep their natural capitalization.

Keep location tags for *significant* or *recurring* places. Don't tag every street corner -- GPS data and folder paths handle that. Location tags are most useful for:

- Recurring venues (concert halls, studios, favorite spots)
- Travel destinations
- Places without GPS data (scanned film, older cameras)

### person hierarchy

```
person
├── family
├── friend
├── artist         (musician, performer, model)
├── public figure
└── ensemble       (named groups of people)
    ├── band
    ├── choir
    ├── orchestra
    └── team
```

The `person` facet covers both individuals and named groups. Individual names go under their relationship category: `person|artist|musician|Peter Schneider`. Named groups go under `ensemble`: `person|ensemble|band|Alice Viola Trio`, `person|ensemble|choir|Vocalitas München`.

This distinction matters: a concert photo might be tagged both `person|ensemble|band|Alice Viola Trio` (the group) and `person|artist|musician|Alice Viola` (an individual member, possibly via face recognition).

Note: `subject|person` is a separate facet that describes *what the photo shows* (portrait, group, activity), not *who is in it*. An image can be `subject|person|portrait` AND `person|artist|musician|Peter Schneider`.

For collections with many named individuals, consider using MAKI's face recognition system instead of (or alongside) person tags. Face recognition scales better and doesn't require manual tagging. Use person tags for individuals who don't appear in photos (event organizers, clients) or as a complement to face recognition for search flexibility.

### technique hierarchy

```
technique
├── style       (black and white, high key, low key, infrared, ...)
├── exposure    (long exposure, double exposure, HDR, ...)
├── lighting    (natural light, flash, studio, golden hour, blue hour, ...)
├── composition (minimalist, symmetry, leading lines, ...)
└── effect      (bokeh, motion blur, silhouette, reflection, lens flare, ...)
```

### project hierarchy

```
project
├── 365 pictures 2018
├── Bricking Bavaria
├── Guido Karp Workshop LA 2019
└── (your projects)
```

Project names are proper nouns and keep their original capitalization. The category `project` itself is lowercase. Project tags are inherently personal and won't follow any standard. Use them to group assets that belong together by assignment or creative intent rather than by subject or location.

---

## How Many Tags?

### Per image

Aim for **5-15 tags per image**:

- 2-4 subject tags (what's in the image)
- 1-2 location tags (where, if relevant and not covered by GPS)
- 0-2 person tags (who, if relevant)
- 1-2 technique tags (how, if noteworthy)
- 0-1 project/event tag

Example: a concert photo might carry `subject|performing arts|concert`, `subject|performing arts|concert|guitarist`, `location|Germany|Bayern|Gelting|Kulturbühne Hinterhalt`, `technique|lighting|stage lighting` -- four tags, good discoverability.

Fewer tags means poor discoverability. More than 20 per image usually means you're tagging noise or duplicating information that belongs elsewhere.

### Total vocabulary

For a serious amateur with 100k+ images:

| Category | Expected range |
|----------|---------------|
| `subject/` terms | 150-300 |
| `location/` entries | 50-200 (grows with travel) |
| `person/` names | varies (consider face recognition) |
| `technique/` terms | 30-50 |
| `project/` entries | varies |
| **Total (excl. names)** | **250-500** |

If your unique tag count is climbing past 1,000 (excluding person names and event-specific tags), it's time to review for duplicates, typos, and over-specificity.

---

## Auto-Tagging and the Label Vocabulary

MAKI's auto-tagging uses a vision-language model (SigLIP) to suggest tags based on visual content. It works by matching image features against a list of text labels -- the **label vocabulary**.

The label vocabulary is different from your full tag vocabulary:

| | Label vocabulary (auto-tagging) | Full vocabulary (human tagging) |
|---|---|---|
| **Purpose** | What the AI model can *see* | What you want to *find* |
| **Structure** | Flat list, one term per line | Hierarchical tree |
| **Size** | 200-400 terms | 300-500+ terms |
| **Content** | Visually recognizable concepts | Includes abstract, contextual, personal terms |
| **Examples** | `sunset`, `concert`, `dog`, `bridge` | `subject/event/workshop`, `project/Bricking Bavaria` |

### Configuring labels

Create a text file with one label per line:

```
# my-labels.txt
landscape
portrait
concert
street photography
...
```

Reference it in `maki.toml`:

```toml
[ai]
labels = "my-labels.txt"
threshold = 0.3
```

A higher threshold (0.3-0.5) produces fewer but more confident suggestions. A lower threshold (0.1-0.2) casts a wider net at the cost of more false positives.

### Label design tips

- Use **short, concrete phrases**: `mountain` works better than `mountainous terrain`
- Match the prompt template: with the default `"a photograph of {}"`, labels should read naturally after "a photograph of" -- "a photograph of a sunset" (good), "a photograph of a subject|nature|sky|sunset" (bad)
- **Don't use hierarchy in labels** -- the model sees text, not structure. Use flat terms
- Include **genre labels** that vision models handle well: `concert`, `portrait`, `landscape`, `macro`, `street photography`, `architecture`
- Include **compositional labels**: `silhouette`, `reflection`, `bokeh`, `long exposure`
- Skip things the model can't see: person names, event names, abstract concepts, workflow state
- Test and iterate: run `maki auto-tag --asset <id> --log` on a few representative images to see what the model suggests at your current threshold

### Mapping labels to hierarchical tags

Auto-tagging produces flat labels (`concert`, `sunset`). You can manually place these into your hierarchy when accepting suggestions, or accept them flat and batch-reorganize later using tag rename:

```bash
# Rename flat tags into hierarchy (example)
maki tag rename "concert" "subject|performing arts|concert"
```

---

## Industry Standards and References

### IPTC Photo Metadata

The [IPTC Photo Metadata Standard](https://iptc.org/standards/photo-metadata/) defines the `dc:subject` field (keywords) and `lr:hierarchicalSubject` (hierarchical keywords). MAKI reads and writes both. Staying within this framework ensures your tags survive export to any IPTC-aware tool.

### How MAKI stores hierarchical tags (the roundtrip)

CaptureOne and Lightroom write two representations of the same hierarchy to XMP:

- **`dc:subject`**: flat individual components — `location`, `Germany`, `Bayern`, `Wolfratshausen`
- **`lr:hierarchicalSubject`**: all ancestor paths — `location`, `location|Germany`, `location|Germany|Bayern`, `location|Germany|Bayern|Wolfratshausen`

On import, MAKI keeps only the `lr:hierarchicalSubject` entries (the pipe-separated paths) and discards the flat `dc:subject` components that are part of any hierarchy. This avoids storing redundant standalone tags like `Germany` alongside `location|Germany`.

On writeback, MAKI regenerates both formats: flat components for `dc:subject`, ancestor paths for `lr:hierarchicalSubject`. CaptureOne and Lightroom see exactly what they expect — no data loss in the roundtrip.

Internally, MAKI stores: `location`, `location|Germany`, `location|Germany|Bayern`, `location|Germany|Bayern|Wolfratshausen`. Searching `tag:Germany` matches `location|Germany` via prefix matching. Standalone `Germany` is not stored because it's redundant.

### Controlled vocabularies

Several controlled vocabularies exist for photo tagging:

- **IPTC Media Topics** -- ~1,200 terms in a 5-level hierarchy, 17 top-level categories. Freely available in multiple formats and 13 languages. Designed for news but applicable to photography. The most authoritative free taxonomy. Available at [iptc.org/standards/media-topics](https://iptc.org/standards/media-topics/).
- **David Riecks' Controlled Vocabulary Keyword Catalog (CVKC)** -- ~11,000 terms, the most comprehensive photography-specific vocabulary. Commercial ($70/year). Includes Lightroom-compatible keyword lists. See [controlledvocabulary.com](https://www.controlledvocabulary.com/).
- **Open keyword lists on GitHub** -- several community-maintained lists in Lightroom's tab-indented text format, freely available:
  - [LightroomKeywordHierarchy](https://github.com/ericvaandering/LightroomKeywordHierarchy) (MIT license, based on IPTC subject codes)
  - [Open Keyword Catalog](https://github.com/markorosic/open-keyword-catalog)

You don't need to adopt any of these wholesale. Use them as references when building your own vocabulary -- they show proven ways to partition and name categories.

---

## Cleaning Up an Existing Catalog

If you've been tagging for years across different tools and approaches, you likely have duplicates, inconsistent casing, mixed languages, and orphaned tags. Here's a phased approach to cleanup.

### Phase 1: Audit

Start by understanding what you have:

```bash
# Total unique tags
maki stats --tags

# Find singleton tags (used only once -- often typos)
python3 scripts/tag-analysis.py

# List all tags with usage counts
maki stats --tags
```

In the web UI, the `/tags` page shows your complete tag tree with counts. Sort by count to see your most-used tags; sort by name to spot near-duplicates.

### Phase 2: Normalize

Fix the mechanical issues first -- these can be done in bulk:

1. **Case normalization** -- merge `Concert` into `concert`, `Blues` into `blues`, etc.
2. **Language normalization** -- pick one language and merge translations.
3. **Remove dead workflow tags** -- bulk-import markers, migration artifacts, tags from previous tools that no longer serve a purpose.

```bash
# Rename a tag across the entire catalog (case-insensitive matching)
maki tag rename "Concert" "concert"
maki tag rename "Konzert" "concert"
maki tag rename "Munich" "München"
```

All matching is case-insensitive — `maki tag rename "Concert" "concert"` catches "Concert", "CONCERT", and "concert".

### Phase 3: Structure

Once the duplicates are resolved, introduce hierarchy:

```bash
# Move flat tags into hierarchy (ancestors are auto-expanded)
maki tag rename "concert" "subject|performing arts|concert"
maki tag rename "landscape" "subject|nature|landscape"
maki tag rename "München" "location|Germany|Bayern|München"
```

When renaming to a hierarchical tag, all ancestor paths are automatically added. For example, the last command replaces "München" with `location|Germany|Bayern|München` and also adds `location`, `location|Germany`, and `location|Germany|Bayern`.

If you have existing tags that were created before ancestor expansion was added, run `maki tag expand-ancestors --apply` to retroactively add the missing ancestor paths.

Do this for your most-used tags first (the top 50-100 tags cover most of your catalog). The long tail can be restructured gradually.

### Phase 4: Prune

Review tags with very low usage (< 5 assets). For each one, decide:

- **Merge** into a broader tag (e.g., `nighthawk` → `bird`)
- **Fix** a typo (`landscpe` → `landscape`)
- **Keep** if it's genuinely specific and useful
- **Remove** if it's noise

### Phase 5: Enrich

Run auto-tagging on untagged or under-tagged assets to fill in descriptive tags:

```bash
# Auto-tag all images (already-tagged assets get additional suggestions)
maki auto-tag "type:image" --apply --log

# Auto-tag a specific shoot
maki auto-tag "path:Capture/2024-08-15" --apply --log
```

Review suggestions in the web UI -- accept good ones, dismiss bad ones, and adjust your label vocabulary or threshold if needed.

---

## Do's and Don'ts

**Do:**

- Define your vocabulary before you start tagging. Spend an hour on structure to save hundreds of hours of cleanup.
- Use hierarchy. It scales; flat lists don't.
- Tag new imports immediately. The longer you wait, the less you remember about the context.
- Use auto-tagging for broad descriptive categories, then refine manually.
- Review your tag list periodically. A 15-minute audit every few months catches drift early.

**Don't:**

- Don't create tags you'll use fewer than ~10 times. One-off tags are noise.
- Don't tag camera or lens information -- that's EXIF data.
- Don't tag dates or folder paths -- those are already searchable.
- Don't go deeper than 4-5 hierarchy levels. Deeper trees are hard to navigate and maintain.
- Don't use tags for workflow state. Use ratings, color labels, or the `rest` tag pattern from [Organizing & Culling](10-organizing-and-culling.md).
- Don't aim for perfection. A 90% consistent catalog that you actually maintain beats a theoretically perfect taxonomy that you abandon after a week.

---

## The Vocabulary File

MAKI creates a `vocabulary.yaml` file in your catalog root when you run `maki init`. This file defines your planned tag hierarchy — a skeleton of categories and terms you intend to use. MAKI reads it and offers these tags in autocomplete, even before you've tagged a single asset with them.

### Why it matters

Without a vocabulary file, autocomplete can only suggest tags that already exist on at least one asset. When you add the first image of a new category, there's no guidance — you have to remember (or look up) your planned hierarchy. The vocabulary file bridges this gap: your planned structure is always available in autocomplete.

### Editing the file

The file uses a nested YAML tree format:

```yaml
subject:
  nature:
    - landscape
    - flora
    - sky
  animal:
    - bird
    - mammal
technique:
  lighting:
    - natural light
    - stage lighting
```

Keys are hierarchy nodes, arrays are leaf lists. Edit the file in any text editor to add, remove, or reorganize categories as your collection grows. MAKI reads it on every command — changes take effect immediately.

### Bootstrapping from existing tags

If you already have a catalog with organic tags, export the current tag tree as a starting point:

```bash
maki tag export-vocabulary
```

This generates `vocabulary.yaml` from your existing tags, grouped into a nested tree. Edit it to add planned categories and remove unwanted entries.

### Vocabulary vs. auto-tagging labels

The vocabulary file and the auto-tagging label file (`labels` in `[ai]` config) serve different purposes:

| | `vocabulary.yaml` | AI label file |
|---|---|---|
| **Purpose** | Autocomplete guidance | Vision model classification |
| **Format** | Nested YAML tree | Flat text, one term per line |
| **Content** | Full hierarchy including abstract/personal terms | Only visually recognizable concepts |
| **Used by** | CLI and web UI autocomplete | `maki auto-tag` |

---

## Quick-Start Checklist

If you're starting fresh or resetting your tagging approach:

1. Choose a language (English recommended)
2. Choose a case convention (lowercase, proper nouns capitalized)
3. Define your top-level facets (subject, location, person, technique, project)
4. Edit `vocabulary.yaml` in your catalog root to reflect your planned hierarchy
5. Write 50-100 tags for your most common subjects
6. Save them as your label vocabulary for auto-tagging (`labels = "my-labels.txt"` in `maki.toml`)
7. Set a confidence threshold (`threshold = 0.3` is a reasonable start)
8. Tag your next import using the new vocabulary
9. Run auto-tagging on a test batch and review the suggestions
10. Expand `vocabulary.yaml` as new subjects appear
11. Schedule a quarterly review of your tag list and vocabulary file

---

Next: [The Archive Lifecycle](12-archive-lifecycle.md) -- storage strategy, backup workflows, and long-term library management.

Previous: [Organizing & Culling](10-organizing-and-culling.md) |
[Back to Manual](../index.md)
