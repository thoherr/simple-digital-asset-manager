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
event            which specific occasion is it from?
location         where was it taken?
person           who is in it? (named individuals)
technique        how was it made?
project          what project or assignment does it belong to?
color            what is the dominant color? (optional)
```

Not every image needs a tag from every facet -- a landscape might only need subject and location; a studio portrait might need subject, person, and technique. `event` and `color` are opt-in facets for photographers who care about those axes.

### Thinking in facets: when to promote a branch to top-level

Each facet answers a *distinct question* about the photo. That independence is the whole point. A concert photo taken at a specific wedding, of a specific musician, in a specific venue, with specific lighting, in a specific color palette carries a tag from each facet — and queries should be able to intersect any subset: "all photos of Alice at outdoor weddings in Bayern with golden-hour lighting."

When you're considering adding a new branch to your vocabulary, the test is simple: **what question does this tag answer about the photo?** If it answers a question that's genuinely distinct from what the existing facets answer, promote it to top-level. If it's a refinement of an existing axis, nest it. Facets are cheap to add; wrong nesting is expensive to fix later.

Two worked examples illustrate the choice.

#### Worked example: events

Many photographers start by filing events under `subject|event|*` — weddings, concerts, workshops. That feels natural at first, but there's friction: events are *date-driven instances*, while the rest of the subject tree is a *stable taxonomy*. New events get added every time one happens; the subject tree barely changes year over year. Jamming them together pollutes your subject vocabulary with names that are meaningful only in context ("Jane's Wedding 2025" is not a kind of thing photos can depict — it's a specific occasion).

The cleaner choice is a top-level `event` facet.

**Pros:**

- Keeps the subject tree stable and navigable. *Subject* answers "what's depicted"; *event* answers "what occasion is this from". Distinct questions, distinct facets.
- Event cardinality grows unboundedly (one new entry per wedding, per trip, per festival). Isolating it prevents the subject tree from getting skewed.
- Event names become their own namespace — easy to keep consistent (`event|wedding-jane-2025`, or grouped by year: `event|2025|wedding-jane`).
- Queries read clearly: `event:wedding-jane-2025 person:Alice` intersects two independent axes.
- Migration is cheap — `maki tag rename "subject|event|wedding-jane-2025" "event|wedding-jane-2025" --apply` handles the pure rename case. For the common situation where an old `subject|event|xxx` tag should become *both* the specific-occasion tag AND the generic scene-type tag, use `maki tag split "subject|event|wedding-jane-2025" "event|wedding-jane-2025" "subject|event|wedding" --apply` to replace one tag with both in a single pass.

**Cons:**

- One more top-level to decide on when tagging.
- You need to distinguish two different questions a photo can answer about an event: *what kind of scene is this* (generic type) vs *which particular occasion is this from* (specific instance). They coexist cleanly — a photo of Jane's wedding carries **both** `subject|event|wedding` (it's a wedding scene) and `event|wedding-jane-2025` (specifically hers). The three layers:
    - `subject|performing arts|concert` — photo depicts a *performance*.
    - `subject|event|wedding` — photo depicts a *ceremony or non-performance gathering*.
    - `event|wedding-jane-2025` — photo is from *this specific occasion*.
- Edge cases where event blurs with subject — a parade photo is arguably both (the event *is* the depicted scene). Tag both; this is a feature, not a bug.

Recommendation: **add `event|` as a top-level facet** for specific instances, and keep `subject|event|*` for generic ceremony/gathering scene types that aren't performances. See the [event hierarchy](#event-hierarchy) section below.

#### Worked example: color

Similar reasoning applies when you start tagging dominant color. You have three plausible homes:

- `subject|color|red` — treating color as a subject type
- `technique|style|red` — treating color as a photographic technique
- `color|red` — its own facet

Color isn't really a subject (unless the photo is *about* red — an abstract color study). It isn't really a technique (red isn't a choice you make, it's an attribute of the scene). What it *is* is an independent axis — orthogonal to what's depicted, where it was taken, how it was shot. So color gets its own facet: `color|red`, `color|blue`, `color|monochrome`.

One caveat: MAKI already has a first-class **color label** field (Red/Yellow/Green/Blue/Purple — the Lightroom-style editorial label). If you only ever tag the five standard colors, you're duplicating that label with more keystrokes. Content-color tagging earns its keep when you want finer distinctions (`teal`, `magenta`, `warm`, `cold`, `pastel`, `monochrome`) or dominant-color filtering across a catalog — which is a different use case from the workflow-oriented color label.

#### Worked example: striking objects that span subject and event

Some photo elements are *visually distinctive* (clearly subject material — they're what's in the frame) AND *almost always tied to a specific occasion* (clearly event material — they exist because of that gathering or festival). Fireworks, campfires, maypoles, parade floats, costumes, decorations, sparklers, bonfires — these all live on the boundary. The instinct to file them under one facet leaves the other axis unsearchable; the right move is to **always tag both**, with each tag answering its own facet's question.

Three concrete examples that come up often:

- **Fireworks.** Subject side: `subject|nature|sky|fireworks` (sibling to `aurora`, `lightning`, `sunset` — the photo *looks like* a sky shot, even though the source is human-made). Event side: `event|holiday|new-year`, `event|holiday|independence-day`, `event|festival|<name>|closing-fireworks`. A `setting|night` qualifier often comes along for the ride, useful when distinguishing dusk fireworks from full-dark.
- **Campfire.** Subject side: `subject|object|fire|campfire` (the `fire` parent earns its place because you'll eventually want bonfire, hearth, candle, gas lamp under it — useful for "show me all fire photos" cutting across kinds). Event side: `event|trip|<name>`, `event|gathering|<name>`, or just leave the occasion-facet empty if it's a generic camping shot.
- **Maypole / Maibaum.** Worth being deliberate here — the English "may pole" (ribbon-dance pole) and the Bavarian *Maibaum* (painted pole with guild shields, erected May 1, guarded against theft by the next village) are different cultural objects sharing one English name. If you photograph both, keep them distinct: `subject|tradition|maypole` (general) with optional `|maypole|bavarian` and `|maypole|english` children, OR commit to the local name as the leaf: `subject|tradition|maibaum`. Event-side pair: `event|tradition|maibaum-aufstellen` for the erection-ceremony photos specifically. Location facet usually rides along too, since many Bavarian villages have a permanent Maibaum on the central square — `location|Germany|Bayern|<village>` co-occurs naturally.

The general pattern: if a photo element is **both** distinctive enough to search for visually **and** tightly coupled to an occasion, give it a home under `subject|...` (for queries like "show me all fireworks photos") and pair it with `event|...` at tagging time (for "all fireworks at New Year"). Neither tag alone gives you both axes; both together does.

A few neighbours that follow the same pattern, with suggested homes:

- Parade float → `subject|vehicle|float` + `event|parade|<name>`
- Costume / traditional dress → `subject|object|costume|<style>` + relevant `event|`
- Christmas tree → `subject|object|decoration|christmas-tree` + `event|holiday|christmas|<year>`
- Lantern festival lanterns → `subject|object|light|lantern` + `event|festival|<name>`
- Sparklers → `subject|object|light|sparkler` (or `subject|nature|sky|sparklers` if photographed sky-style) + appropriate `event|`

These leaves don't need to ship in the default vocabulary on day one — add them when your catalog demands them, and the hierarchy organically grows along the axis your photos actually exercise.

#### Worked example: when *not* to add a tag — "action"

Some words look like obvious tags but actually compress three different questions into one ambiguous string. "Action" is the textbook case. Walked through carefully, it usually splits into:

1. **"There's a recognisable activity in the frame."** A runner, a surfer, a dog jumping, a dancer mid-leap. → Already covered by `subject|person|activity|*` (or `subject|animal|...`, or `subject|performing arts|...`). The activity-ness is *implicit* in the activity tag. Adding a separate `action` here is redundant and dilutes the activity branch.

2. **"The photo's defining quality is motion / a decisive moment, regardless of subject."** A panning shot of a passing train, a frozen-mid-jump portrait, a wave crashing — the dynamism *is* the photo, not whatever happens to be moving. → A genuine cross-cutting qualifier, sibling to `style` / `condition` / `mood`:

    ```
    subject
    └── motion
        ├── action      (dynamic moment, decisive moment)
        ├── still       (statically posed, calm composition)
        ├── panning     (subject sharp, background streaked)
        ├── frozen      (high-shutter mid-action)
        └── flowing     (silky-water long exposure, blurred crowd)
    ```

    Combines with any subject branch — `subject|nature|water|wave` + `subject|motion|action` is a different photo than `subject|nature|water|wave` + `subject|motion|flowing`.

3. **"I used a specific motion-emphasis technique."** Long exposure, panning, fast-shutter freeze, slow-sync flash. → That's `technique|motion|*`, not subject. Describes *how the shot was made*, not what's in it.

**The recommendation:**

- Don't add a bare `action` tag — it's not searchable for anything specific.
- For sense (1), use the specific activity tag. "All my action photos" becomes a saved search like `tag:subject|person|activity OR tag:subject|animal OR tag:subject|performing arts`.
- For sense (2), `subject|motion|*` earns its keep only if you actively want to slice the catalog by motion-quality independent of subject. If you're not sure, you don't need it.
- For sense (3), `technique|motion|*` follows the same rule as other technique tags — only worth it if you tag technique systematically.

The wider lesson: **a tag that means "kinda this, kinda that" isn't a tag, it's a placeholder**. When you reach for one, the question to ask isn't "where should it go in the hierarchy" but "which of the three questions am I actually answering, and is there already a facet that handles it?". Often the answer is "yes, and I don't need a new tag at all".

A few more words that look like tags but usually decompose:

- `candid` → either `subject|mood|*` (relaxed, joyful) or implicit in `subject|person|portrait` vs `subject|person|activity`. Rarely needs its own tag.
- `mood` (as a tag, not a facet) → use one of the mood leaves (`subject|mood|melancholic` etc.). The word `mood` itself answers nothing.
- `nature` → use the specific nature subbranch (`subject|nature|landscape`, `subject|nature|flora`). Bare `nature` is just the parent facet name.
- `interesting`, `nice`, `cool` → not tags. These are ratings or color labels in disguise. Use MAKI's rating (1–5) or color label fields.

#### The general rule

Ask: *what distinct question does this tag answer about the photo?* If the answer matches an existing facet, nest it. If it's a new question — a new way someone might want to slice the catalog — promote it to top-level.

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
│   └── street       (road, alley, graffiti, neon sign, shop front, ...)
├── vehicle          (cross-cuts every setting — not constrained to urban)
│   ├── car
│   ├── truck
│   ├── motorcycle
│   ├── bicycle
│   ├── train
│   ├── airplane
│   └── boat
├── person
│   ├── portrait     (headshot, environmental portrait, candid, ...)
│   ├── group        (family, couple, crowd, ...)
│   └── activity     (dance, sports, hiking, cooking, ...)
├── performing arts
│   ├── concert      (live music, musician, singer, guitarist, ...)
│   ├── theatre      (actor, stage set, rehearsal, costume, ...)
│   └── dance        (ballet, contemporary, ...)
├── event            (ceremony / gathering scene types — not performances)
│   ├── wedding
│   ├── exhibition   (art exhibition, photo exhibition, ...)
│   ├── workshop     (photo workshop, craft workshop, ...)
│   ├── sports event (marathon, match, tournament, ...)
│   └── festival     (food, cultural, craft — music festivals go under performing arts)
├── object
│   ├── food         (coffee, wine, cake, cooking, restaurant, ...)
│   ├── instrument   (guitar, piano, drum, saxophone, ...)
│   └── other        (book, camera, flag, candle, sculpture, ...)
├── concept
│   ├── travel
│   ├── fashion
│   ├── documentary
│   └── abstract
├── style          (visual era/aesthetic of the SUBJECT — see note below)
│   ├── vintage
│   ├── modern
│   ├── retro
│   ├── rustic
│   ├── industrial
│   └── classic
├── condition      (physical state of the subject)
│   ├── abandoned
│   ├── ruined
│   ├── restored
│   ├── weathered
│   ├── pristine
│   └── under construction
└── mood           (emotional quality of the scene)
    ├── dramatic
    ├── serene
    ├── playful
    ├── mysterious
    ├── melancholic
    └── joyful
```

You don't need all of these on day one. Start with the top two levels and add
leaf nodes as your collection demands them.

**Subject qualifiers — style, condition, mood:** These three branches are *cross-cutting qualifiers* that can be combined with any other subject category. They describe *how* the subject looks or feels, not *what* it is. Tag a vintage car photo as `subject|vehicle|car` + `subject|style|vintage`. Tag an abandoned factory as `subject|urban|architecture` + `subject|condition|abandoned` + `subject|mood|mysterious`.

Don't confuse `subject|style` (the subject's visual era — vintage, modern, retro) with `technique|style` (the photographic technique — black and white, high key, infrared). A vintage car shot in black and white gets both: `subject|style|vintage` + `technique|style|black and white`.

**When categories overlap:** `subject|person` describes the *composition* of the photo (a portrait, a group scene, someone doing an activity), not the profession or context. A musician on stage is `subject|performing arts|concert|musician` — the photo shows a performance, not a person doing a generic activity. The same musician backstage in a headshot is `subject|person|portrait`. The rule: ask "what is the photo *about*?" — if the answer is "a concert" use `performing arts`, if it's "a portrait of someone" use `person`.

Similarly, `subject|person|activity` is for general activities (hiking, cooking, sports) where the activity defines the image. A wedding guest dancing → `subject|person|activity|dance`. A ballet performance → `subject|performing arts|dance|ballet`. Context matters more than the literal content.

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

### event hierarchy

```
event
├── wedding-jane-2025
├── workshop-la-2019
├── concert-alice-viola-2024-11-15
└── (your events)
```

Specific occasions — weddings, workshops, trips, named concerts — go here, one tag per event. You have two naming styles to pick from:

- **Flat with date in name**: `event|wedding-jane-2025` — simplest, easy to type, each event is one leaf tag.
- **Grouped by year**: `event|2025|wedding-jane`, `event|2024|workshop-la` — browsable by year in the tags tree, and year becomes a queryable ancestor (`tag:event|2025` finds everything from 2025).

Pick one style and stay consistent. Flat naming works well for most people; year grouping pays off when you have many events per year and want to browse chronologically.

**What belongs under top-level `event|` vs `subject|event|*`:**

- **Specific instances** (`event|wedding-jane-2025`, `event|fuji-trip-2023`) — always top-level `event|`. This axis answers *which particular occasion is this photo from?*
- **Generic ceremony / gathering scene types** (`subject|event|wedding`, `subject|event|exhibition`, `subject|event|sports event`) — stay under the subject tree. This axis answers *what kind of scene is this?*
- **Performances** (concerts, theatre, dance) — `subject|performing arts|*`, not `subject|event|*`. Performances are distinct from ceremonies and gatherings; they get their own subject branch.
- **Recurring annual events** — treat each year as its own specific instance (`event|christmas-2024`, `event|christmas-2023`), or group them (`event|recurring|christmas|2024`) if you want a single branch for all Christmases.

A photo from Jane's wedding typically carries both `subject|event|wedding` and `event|wedding-jane-2025`: one tag describes the scene, the other the occasion.

Event names are inherently personal — you won't find standard event taxonomies. Name them however you'll remember them, and keep the style consistent.

### color (optional)

If you tag dominant color as a searchable attribute of the image, use it as its own facet:

```
color
├── red
├── orange
├── yellow
├── green
├── blue
├── purple
├── pink
├── brown
├── black
├── white
├── grey
├── monochrome
├── pastel
├── warm
└── cold
```

Low-cardinality vocabulary (15-30 terms covers almost everything). Keep it flat — a hierarchy like `color|warm|red` adds no real structure, since "red" is already obviously warm.

**When to use it:** content-color tagging earns its keep when you want finer distinctions than MAKI's built-in color label (Red/Yellow/Green/Blue/Purple — which is an editorial workflow field, not a content descriptor), or when you want to filter a catalog by dominant color. If you only ever tag the five standard colors and only use them as a workflow marker, use the color label field instead — that's what it's for.

---

## How Many Tags?

### Per image

Aim for **5-15 tags per image**:

- 2-4 subject tags (what's in the image)
- 1-2 location tags (where, if relevant and not covered by GPS)
- 0-2 person tags (who, if relevant)
- 1-2 technique tags (how, if noteworthy)
- 0-1 event tag (which specific occasion, if this photo belongs to one)
- 0-1 project tag
- 0-1 color tag (if you tag dominant color)

Example: a concert photo might carry `subject|performing arts|concert`, `subject|performing arts|concert|guitarist`, `location|Germany|Bayern|Gelting|Kulturbühne Hinterhalt`, `technique|lighting|stage lighting`, `event|concert-alice-viola-2024-11-15` — five intentional tags that you choose.

These counts refer to the **leaf-level tags you consciously assign**, not the total number of tags stored. MAKI automatically expands each hierarchical tag to include all ancestor paths (e.g., `subject|performing arts|concert|guitarist` also stores `subject`, `subject|performing arts`, and `subject|performing arts|concert`). The five tags in the example produce 15 stored entries — but you only think about the five leaves.

Fewer intentional tags means poor discoverability. More than 20 usually means you're tagging noise or duplicating information that belongs elsewhere.

### Total vocabulary

For a serious amateur with 100k+ images:

| Category | Expected range |
|----------|---------------|
| `subject/` terms | 150-300 |
| `location/` entries | 50-200 (grows with travel) |
| `person/` names | varies (consider face recognition) |
| `technique/` terms | 30-50 |
| `project/` entries | varies |
| `event/` entries | varies (grows with life) |
| `color/` terms | 15-30 (if used) |
| **Total (excl. names/events)** | **250-500** |

If your unique tag count is climbing past 1,000 (excluding person names and event-specific tags), it's time to review for duplicates, typos, and over-specificity.

---

## Auto-Tagging and the Label Vocabulary

MAKI's auto-tagging uses a vision-language model (SigLIP) to suggest tags based on visual content. It works by matching image features against a list of text labels -- the **AI vocabulary**.

The AI vocabulary is different from your **catalog vocabulary** (the tag tree this chapter is otherwise about), and the two interact:

| | AI vocabulary | Catalog vocabulary |
|---|---|---|
| **Purpose** | What the AI model can *see* | What you want to *find* |
| **Structure** | Flat list of labels, each mapped to one or more hierarchical tags | Hierarchical tree |
| **Size** | 100-400 labels | 300-500+ tags |
| **Content** | Visually recognizable concepts | Includes abstract, contextual, personal terms |
| **Examples** | `sunset`, `concert`, `dog`, `bridge` | `subject\|event\|workshop`, `project\|Bricking Bavaria` |

The AI suggests tags from a flat vocabulary; the mapping converts each hit into the canonical hierarchical tag MAKI actually applies. Both vocabularies ship with sensible defaults, and the defaults are coherent -- every tag the AI vocabulary maps to is also a leaf in the catalog vocabulary, so the two stay in sync.

### Configuring labels

The AI vocabulary lives in a single YAML file. Keys are the labels the model sees; values are the hierarchical tag(s) to suggest when a label scores above threshold:

```yaml
# my-labels.yaml
sunset:
  - subject|nature|sky
  - technique|lighting|golden hour
concert: subject|performing arts|concert
portrait: subject|person|portrait
dog: subject|animal|domestic
abstract: subject|concept|abstract
# null = "leave the label flat, no canonical mapping"
weather:
```

Point `maki.toml` at it:

```toml
[ai]
labels = "my-labels.yaml"
threshold = 0.3
```

A higher threshold (0.3-0.5) produces fewer but more confident suggestions. A lower threshold (0.1-0.2) casts a wider net at the cost of more false positives.

To start from the built-in default (which already maps 96 photographic labels into the canonical hierarchy described in this chapter), dump it and edit:

```bash
maki ai export-vocabulary --default > my-labels.yaml
```

Legacy `.txt` flat-list files (one label per line, identity mapping) still load — the loader picks format by extension. To migrate an existing flat list to YAML with identity mappings as a starting point:

```bash
maki ai export-vocabulary > my-labels.yaml   # converts the active .txt file
```

### Label design tips

- Use **short, concrete phrases**: `mountain` works better than `mountainous terrain`.
- Match the prompt template: with the default `"a photograph of {}"`, labels should read naturally after "a photograph of" -- "a photograph of a sunset" (good), "a photograph of a subject|nature|sky|sunset" (bad).
- **Don't use hierarchy in label keys** -- the model sees the key as text and scores the image against it. Use flat terms for keys; put hierarchy in the values.
- Include **genre labels** that vision models handle well: `concert`, `portrait`, `landscape`, `macro`, `street photography`, `architecture`.
- Include **compositional labels**: `silhouette`, `reflection`, `bokeh`, `long exposure`.
- Skip things the model can't see: person names, event names, abstract concepts, workflow state.
- Test and iterate: `maki auto-tag --asset <id> --log` on a few representative images shows what the model picks at your threshold.

### When AI specificity meets your catalog hierarchy

The shipped default vocabulary maps to canonical leaves: `dog` and `cat` both land on `subject|animal|domestic`, every kind of book/jewelry/furniture on `subject|object|other`. The mapping is deliberately conservative -- the default catalog vocabulary stops at the genus level for most branches, so the default AI vocabulary maps no deeper than that. It's a scaffold; no orphan tags appear in autocomplete out of the box. (One exception: the vehicle branch has species-level leaves out of the box -- see strategy 3 below for the reasoning.)

But the AI saw something more specific. The model picked `dog` because it actually *looks* like a dog; collapsing that to `domestic` loses information that your tag tree may want to capture. Three coherent strategies, depending on how much your catalog has grown:

**1. Conservative.** Keep the default mapping. Tag genus-level (`subject|animal|domestic`, `subject|object|other`). Filter by what's in the photo via search rather than tag hierarchy depth. Good when you're starting out, or when species-level distinctions don't drive how you browse.

**2. Extend both vocabularies together.** If your catalog already has `subject|animal|domestic|dog`, `subject|animal|domestic|cat`, etc. as real tags, point the AI mapping at those leaves so AI suggestions land where you want them. Two files to edit:

```yaml
# my-labels.yaml — point AI labels at the leaves
dog:   subject|animal|domestic|dog
cat:   subject|animal|domestic|cat
horse: subject|animal|domestic|horse
```

```yaml
# vocabulary.yaml — declare those leaves in the catalog vocabulary
subject:
  animal:
    domestic:
      - dog
      - cat
      - horse
      - cow
      - sheep
```

The two files evolve together: every leaf you tag actively becomes a leaf in `vocabulary.yaml`, and the AI mapping points at it. The pair stays consistent.

**3. Promote a branch.** When the canonical nesting doesn't fit, lift the branch to its own facet. Worked example, taken from MAKI's own defaults: vehicles aren't always urban — a tractor in a field, a fishing boat at sea, an airplane in flight don't belong under an "urban" branch. The default vocabulary therefore puts vehicles in their own top-level subject facet:

```yaml
subject:
  vehicle:
    - car
    - truck
    - motorcycle
    - bicycle
    - train
    - airplane
    - boat
```

and the default AI mapping points at those leaves directly:

```yaml
car:     subject|vehicle|car
bicycle: subject|vehicle|bicycle
train:   subject|vehicle|train
airplane: subject|vehicle|airplane
boat:    subject|vehicle|boat
```

Apply the same reasoning to your own catalog when you notice a leaf that you keep wanting to use across multiple parent branches — that's the signal it's an independent axis, not a refinement. See [Thinking in facets](#thinking-in-facets-when-to-promote-a-branch-to-top-level) for the underlying principle.

**Best practices**

- **Start with the default.** Run `maki ai export-vocabulary --default > my-labels.yaml` and look through it before customizing. The default is a working baseline that's coherent with the default catalog vocabulary.
- **Add leaves you actually browse by.** Don't pre-emptively add every species name the AI knows -- the catalog vocabulary should reflect axes you genuinely filter on, not theoretical completeness. Add a leaf the first time you'd want to filter by it.
- **Keep both files in sync.** When you add a leaf to `vocabulary.yaml`, update `my-labels.yaml` so the AI label maps to it. When you remove a leaf, scan `my-labels.yaml` for orphan references. (A future helper command may automate the consistency check.)
- **Use fan-out sparingly.** One label can map to multiple tags (sunset → sky subject AND golden-hour lighting). Useful for labels that genuinely span two facets, noisy otherwise -- if every "wedding" suggestion adds three tags you didn't want, you've over-fanned.
- **Null where it doesn't fit.** Labels with no clean canonical home (weather, seasonal, generic terms) stay `null` and surface flat. Decide what to do with each *when you see it on a real photo*, not in the vocabulary file ahead of time.

### Renaming legacy flat tags into hierarchy

If your catalog has flat tags from earlier AI runs (before the YAML mapping existed), batch-reorganize with `maki tag rename`:

```bash
maki tag rename "concert" "subject|performing arts|concert"
maki tag rename "dog" "subject|animal|domestic|dog"
```

Then update `my-labels.yaml` so future AI suggestions land at the new path directly.

---

## Industry Standards and References

### IPTC Photo Metadata

The [IPTC Photo Metadata Standard](https://iptc.org/standards/photo-metadata/) defines the `dc:subject` field (keywords) and `lr:hierarchicalSubject` (hierarchical keywords). MAKI reads and writes both. Staying within this framework ensures your tags survive export to any IPTC-aware tool.

### How MAKI stores hierarchical tags (the roundtrip)

**Prerequisite:** Your photo editor must be configured to write XMP sidecar files. Most tools don't do this by default — see [Working with External Tools](07-maintenance.md#prerequisite-enable-xmp-sidecar-writing-in-your-photo-editor) for per-tool settings.

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

**Find under- and over-tagged assets** with the `tagcount:` filter (counts intentional leaf tags, not auto-expanded ancestors):

```bash
maki search "tagcount:0" --format ids | wc -l     # completely untagged
maki search "tagcount:1 rating:4+"                # under-tagged keepers
maki search "tagcount:10+"                        # suspiciously heavy
maki search "tagcount:2-3 date:2024"              # recent light tagging
```

`tagcount:0` is the most useful for catching gaps — any asset that escaped tagging shows up.

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

### Reviewing suggestions before committing (web UI)

`maki auto-tag --apply` is unconditional: every suggestion above the configured threshold is written to every matching asset. For a curated collection -- a vacation set, a wedding shoot, anything where false positives accumulate noticeably -- the **Review tags…** button on the browse toolbar gives you a per-tag confirmation step without forcing you to select assets one by one.

Flow:

1. Filter the browse page down to the assets you want to review (or select a subset). Click **Review tags…**.
2. MAKI computes AI suggestions for every asset in scope -- roughly 1-2 s per image on CPU, faster with the `ai-gpu` feature. Progress shows in the toast, same as Embed or Auto-tag.
3. When the job finishes, the review modal opens. The left pane lists every suggested tag sorted by candidate count (most-suggested first). The right pane shows the candidate thumbnails for the selected tag, all pre-checked, each with its confidence score and -- when the AI's underlying label differs from the hierarchical tag MAKI mapped it to -- a small "from: *skyscraper*" hint.
4. Uncheck false positives. Drag the **Threshold** slider to bulk-toggle by confidence. Tick **Skip already-tagged** to hide assets that already carry the tag.
5. Click **Apply *building*** to write the tag to the checked set. The row strikes through and the next un-reviewed tag opens. Use **Skip** to leave a tag for later.

The collection pass runs at half your configured `[ai].threshold` so the slider has room to drag both above and below your normal auto-tag floor. Suggestions are held in server memory on the job's record (page-reload survival within the session), so you can close the modal, finish another task, and reopen the review via the toast badge -- a server restart drops them.

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

## Tag Input in the Web UI

The browse filter bar's tag input and the asset-detail tag chip both share the same autocomplete dropdown. Two behaviours worth knowing:

**Middle-of-hierarchy matches stay visible.** Type `subject` and the dropdown shows `subject` itself even if your catalogue also has `subject|nature`, `subject|nature|landscape`, etc. as deeper matches. Earlier MAKI versions hid the parent row in favour of the more specific children, which made it hard to land at exactly the level you intended. Now any node whose own name (any segment) matches the query stays in the list alongside its descendants.

**Drill into a hierarchy by clicking the prefix.** Each suggestion that has a hierarchy renders the prefix in a dimmed style. Clicking that dimmed prefix narrows the autocomplete to that level — useful when a query produces dozens of suggestions across unrelated parents (`bird` matches `subject|nature|animal|bird`, `event|birding-trip`, etc.) and you want to focus on one branch. Clicking the leaf segment commits the full path as usual.

The same dropdown is reused on the tag rename / split modals on the tags page, so the behaviour is consistent everywhere a tag value is typed.

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

To see what the built-in default vocabulary looks like (without mixing in your catalog's tags), use `--default`:

```bash
maki tag export-vocabulary --default --output defaults.yaml
```

This is useful after a MAKI upgrade that adds new default categories (e.g. `subject|style`, `subject|condition`, `subject|mood`) — you can compare the defaults with your existing vocabulary and merge what you want. The `--default` flag ignores both your catalog tags and your existing `vocabulary.yaml`; it outputs only the built-in starter vocabulary.

### Sharing your vocabulary with Lightroom and Capture One

The vocabulary you curate in MAKI is also useful inside your RAW processor — so that culling or initial tagging sessions in Lightroom or Capture One autocomplete against the same hierarchy. Export the tree as a tab-indented keyword text file:

```bash
maki tag export-vocabulary --format text --prune \
    --output ~/Desktop/maki-keywords.txt
```

Then import it:

- **Lightroom Classic**: *Metadata → Import Keywords…* and pick the file. Existing keywords are merged; the hierarchy is preserved.
- **Capture One**: *Image → Keywords → Import Keywords → Keyword Text File*, pick the file. The keywords become available in the Keywords tool and in autocomplete.

Re-run the export whenever your catalog grows enough new tags to be worth re-syncing. With `--prune`, you only push the subset that's actually in use; drop `--prune` to push the full planned hierarchy.

### Annotating exports with asset counts

Pass `--counts` to attach the per-tag asset count to every entry. In YAML output the count appears as a `# N assets` trailing comment; the file is still valid YAML and MAKI's autocomplete loader ignores comments. Useful for spotting candidates to consolidate or retire:

```bash
maki tag export-vocabulary --counts
```

```yaml
subject:  # 5 assets
  nature:  # 3 assets
    - landscape  # 2 assets
    - bird  # 1 asset
- legoland  # 1 asset
```

In **text** format (Lightroom / Capture One) the flag is silently ignored — those tools reject comments. In **json** format every node already has a `count` field; the flag is implied.

### Programmatic export (JSON)

For dashboards, integration scripts, or any tool that finds the keyword-text or YAML formats awkward to walk, export as nested JSON:

```bash
maki tag export-vocabulary --format json --prune > tags.json
```

Each node has a `count` field plus an optional `children` map keyed by child segment name. Leaf nodes omit `children`:

```json
{
  "subject": {
    "count": 5,
    "children": {
      "nature": {
        "count": 3,
        "children": {
          "landscape": {"count": 2},
          "bird": {"count": 1}
        }
      }
    }
  },
  "legoland": {"count": 1}
}
```

The structure mirrors the tag tree exactly — root keys are top-level tags, nested keys are sub-tags, and counts are the per-asset totals from the catalog. Identical to `maki tag export-vocabulary --format yaml --counts` in information content; the JSON shape is just easier to consume from code.

### Exporting from the web UI

The tags page (`/tags`) carries an **Export vocabulary…** button next to the page title — a one-click UI alternative to running `maki tag export-vocabulary` from a shell. Clicking it opens a small modal:

- Format radio: **YAML** (default), **Keyword text** (Lightroom / Capture One), or **JSON**.
- `Annotate with per-tag asset count` — equivalent to `--counts`. Auto-disabled when "Keyword text" is selected (LR / C1 reject comments).
- `Prune planned-but-unused entries` — equivalent to `--prune`.
- `Built-in default vocabulary only` — equivalent to `--default`. Greys out the counts and prune options since neither applies (the built-in tree has no asset counts).

Clicking **Download** triggers a browser save dialog with the appropriate filename (`vocabulary.yaml` / `.txt` / `.json`). The output is byte-identical to what the CLI command produces with the same flags — same rendering pipeline (`src/vocabulary.rs`), same sanitisation warnings (the Lightroom-import sanitisation step runs server-side; if any of your tags contain commas, semicolons, or XML entities, the export will silently rename them in the file but the catalog stays untouched).

### Vocabulary vs. auto-tagging labels

The vocabulary file and the auto-tagging label file (`labels` in `[ai]` config) serve different purposes:

| | `vocabulary.yaml` | AI label file |
|---|---|---|
| **Purpose** | Autocomplete guidance | Vision model classification |
| **Format** | Nested YAML tree | Flat text, one term per line |
| **Content** | Full hierarchy including abstract/personal terms | Only visually recognizable concepts |
| **Used by** | CLI and web UI autocomplete | `maki auto-tag` |

---

```{=latex}
\clearpage
```

## Putting it all together

Every principle in this chapter plays out on a single photo. Consider a shot from Jane's wedding reception in Bayern:

![One photo, nine tags, seven facets — each answering a different question.](../images/maki-tagging.png){width=90%}

Nine leaf tags across seven facets. Ancestor expansion bumps the stored count to roughly fifteen — but you only think about the nine leaves you picked.

Each tag answers a different question about the same image:

- **Subject tags** describe *what the photo depicts* — a wedding scene, a group of people, a joyful mood. These are conceptually the deepest tree; most of your vocabulary lives here.
- **Event** ties the photo to a *specific occasion* — Jane's 2025 wedding, not any wedding. A date-driven instance that doesn't belong in the stable subject taxonomy.
- **Location** says *where in the world* it was taken — structured from country (English) down to local venue names.
- **Person** names *who* is in the frame. On a large catalogue, face recognition handles most of this automatically; person tags are useful for named individuals who won't be picked up by faces (or for precision when faces are ambiguous).
- **Technique** captures *how* it was made — golden hour lighting, silhouette composition. Two tags from the same facet but different sub-axes, because technique has multiple meaningful ones (lighting, style, composition, effect).
- **Color** records the *dominant palette* — useful when you later want to assemble a warm-toned or monochrome gallery across the catalog.
- **Project** would tie it to an assignment or body of work (a photo book, a 365 project) if applicable. Not every photo has one.

A search like `event:wedding-jane-2025 person:Jane color:warm` intersects three independent axes. You can recombine them however the question of the moment demands — `technique:golden` across every event, or `color:monochrome subject:nature|landscape` across every location. The catalog becomes a dense multidimensional space you can slice any way you want, rather than a tree you have to walk in a fixed order.

That's the whole point of faceted tagging: invest up front in the vocabulary structure, and every photo you've ever tagged becomes available to arbitrary future queries you didn't think to ask when you tagged it.

---

## Quick-Start Checklist

If you're starting fresh or resetting your tagging approach:

1. Choose a language (English recommended)
2. Choose a case convention (lowercase, proper nouns capitalized)
3. Define your top-level facets — start with the five essentials (subject, location, person, technique, project) and add `event` and/or `color` if you'll use them
4. Edit `vocabulary.yaml` in your catalog root to reflect your planned hierarchy
5. Write 50-100 tags for your most common subjects
6. Save them as your label vocabulary for auto-tagging (`labels = "my-labels.txt"` in `maki.toml`)
7. Set a confidence threshold (`threshold = 0.3` is a reasonable start)
8. Tag your next import using the new vocabulary
9. Run auto-tagging on a test batch and review the suggestions
10. Expand `vocabulary.yaml` as new subjects appear
11. Schedule a quarterly review of your tag list and vocabulary file

---

Next: [Visual Discovery](12-visual-discovery.md) -- face recognition, similarity search, and strolling: finding photos by what they look like.

Previous: [Organizing & Culling](10-organizing-and-culling.md) |
[Back to Manual](../index.md)
