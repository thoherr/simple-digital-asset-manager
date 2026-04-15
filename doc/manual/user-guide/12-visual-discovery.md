# Visual Discovery

Metadata — tags, ratings, dates, filenames — tells you what you know about an image. But sometimes you need to find photos by what they **look like**: *who is in this shot*, *which other frames show the same scene*, *what other photos does this one remind me of*. Visual discovery is MAKI's answer to that class of question.

Three features share a common foundation (neural-network embeddings that map an image to a vector of numbers, where visually similar images end up near each other):

- **Face recognition** — "who is in this photo?" Detects faces, maps each to an identity vector, and lets you search and group by person.
- **Similarity search** — "what other photos look like this one?" Powers the `similar:` filter, burst detection, and variant discovery.
- **Stroll** — "show me photos visually adjacent to this one, let me wander." A spatial navigation mode that uses similarity to build a graph of your catalog.

This chapter covers all three as workflows rather than controls. For the mechanics (commands, flags, config keys) see [Organizing Assets](04-organize.md), [Browse & Search](05-browse-and-search.md), and [Web UI](06-web-ui.md).

---

## Why Visual Discovery Matters

Tags describe what you can put into words. Visual discovery finds what you recognize without having to name it.

Consider: you remember a sunset photo with a silhouetted figure, but you never tagged "silhouette" — and "sunset" alone returns 800 candidates. Open any silhouette you can find, hit the *similar* button, and the top 30 results are probably the right neighbourhood. You found what you needed without ever putting the feeling into words.

Three scenarios where this pays off:

1. **Finding forgotten photos.** Your catalog is larger than your memory. Visual similarity surfaces images that metadata alone would never connect.
2. **Organising bursts and variants.** RAW+JPEG already get grouped, but twenty near-identical frames from a concert need visual grouping — the filenames alone don't tell you which frames are duplicates.
3. **Connecting people across decades.** A face recognised in a 2003 shot and a 2024 shot are the same person despite everything else having changed.

None of these replace tags. They complement them: use tags for concepts you can name, visual discovery for what you can only recognise.

---

## Face Recognition

MAKI uses two ONNX models in sequence: **YuNet** to find faces in an image (bounding boxes + five landmark points for each face), and **ArcFace ResNet-100** to produce a 512-dimensional identity embedding for each face after aligning it to a canonical pose. Two faces of the same person end up near each other in this 512-dim space; faces of different people end up far apart.

### The Short Version

```
maki faces detect --query "*" --apply          # find faces on every asset (first run)
maki faces cluster --apply                     # group similar faces into people
# → Open /people in the web UI
# → Rename the big clusters: Alice, Bob, Carol
# → Use the "Merge suggestions" panel to fold splinter clusters
```

That gets you 80% of the value. The rest of this section is about when and why to deviate from that flow.

### Detect vs. Cluster vs. Assign: Picking a Workflow

Face recognition has three distinct steps:

1. **Detect** — scan image pixels, find where the faces are, extract an identity embedding. Happens once per asset; `--force` re-runs it (e.g. after upgrading the recognition model).
2. **Cluster** — look at all unassigned face embeddings and group the similar ones into "people." Each cluster becomes an unnamed person ("Unknown") that you can rename later.
3. **Assign** — attach a specific face to a specific person (named or unnamed). Can be automatic (via clustering) or manual (via the asset detail page or a batch merge).

The interesting design choice is *when* to cluster versus assign per-asset. Three scenarios:

**Bulk first-pass.** You just imported 5 000 photos from a family archive. Run `faces detect --query "*" --apply`, then `faces cluster --apply` from the web UI. You'll end up with a handful of big clusters per regular subject (mother, grandmother, siblings) plus lots of singletons for one-off appearances. Name the big ones; ignore the rest until they matter.

**Targeted subject.** You're preparing a gift collection for a friend — you need every photo that includes them. Detect faces on that scope, cluster within the scope (`faces cluster --query "tag:family" --apply`), rename the result. Much faster than clustering the whole catalog.

**One-off group photo.** You just detected faces on a single old group photo. Clustering won't help here — it only creates groups of two or more similar faces, and each face in the photo is a singleton until more photos of that person exist. Just assign each face manually on the detail page: the face-assign combobox has a type-to-filter dropdown that always offers "+ Create new person" as its last option. (*This is the case we get asked about most often — clustering needs at least two similar faces to form a group, by design.*)

### The Clustering Threshold

Clustering is governed by `face_cluster_threshold` — the minimum centroid similarity for two clusters to merge (0.0 to 1.0, higher = stricter). The default is 0.35, tuned for typical portrait-heavy catalogs using MAKI's aligned FP32 pipeline. Your mileage will vary:

- **0.3 and below** — aggressive; big clusters, risk of mixing similar-looking people (close family members, kids of the same age)
- **0.35 (default)** — a usable middle ground
- **0.45 and above** — tight; many singletons and small splinters, very unlikely to merge different people

If clustering is producing nonsense (one giant cluster with everyone in it, or hundreds of tiny clusters that should be merged), the threshold is probably wrong for your data. But before you guess, look at the actual similarity distribution.

### Diagnosing Bad Clusters

`maki faces similarity` prints percentile statistics and a histogram of pairwise similarities for a scope. A healthy distribution is **bimodal**:

```
Histogram (10 buckets, -0.34 – 1.00):
  -0.343 – -0.209     138
  -0.209 – -0.075    4718  ██████████
  -0.075 –  0.060   18237  ████████████████████████████████████████
   0.060 –  0.194    8710  ███████████████████████          ← different-people hump
   0.194 –  0.328     958  ███
   0.328 –  0.462    2036  ████                              ← valley between humps
   0.462 –  0.597    3350  ███████
   0.597 –  0.731    4032  ████████
   0.731 –  0.865    3287  ███████                           ← same-person hump
   0.865 –  1.000     590  █
```

The valley around 0.3–0.4 is where the threshold belongs. If instead you see a single narrow hump (everything at 0.85+, or everything between 0.9 and 1.0), the embeddings are not discriminating and no threshold will help — that points to a pipeline problem: bad crops, wrong model, a preprocessing mismatch.

`maki faces dump-aligned --query "..." --limit 30` saves the 112×112 aligned crops that actually get fed to the recognition model. Eyes should be roughly level, faces centered, nothing mirrored or squashed. If they look wrong, recognition cannot work no matter what you do downstream.

### Merge Suggestions

Clustering almost always produces **splinter clusters**: a main cluster of 90 faces for Alice, plus a 5-face satellite of Alice taken under different lighting or at a much different age. The splinter's centroid is close enough to the main one that you'd call them the same person at a glance, but not close enough to have merged automatically.

The `/people` page surfaces these. The "Merge suggestions" panel above the grid lists candidate pairs with their match scores. Each suggestion shows both sides with an arrow indicating default merge direction (target → source), a swap button, and merge / dismiss actions. If neither side is named, the larger cluster becomes the target; if one is named, the named one wins so you keep your naming work.

Aim to keep this panel empty. Each morning's cleanup is usually five clicks.

### Confidence Filtering

Not every detected face is worth clustering. Blurry faces, profile shots, and faces occluded by hair, sunglasses, or other people produce noisy embeddings that pollute clusters. The `--min-confidence` flag on `cluster` drops low-quality detections before they can cause damage:

```
maki faces cluster --min-confidence 0.85 --apply
```

The default is 0.7. Raising it to 0.8 or 0.85 usually produces cleaner clusters at the cost of leaving some real faces unassigned — they remain as unassigned faces on their assets, still visible and assignable manually. This is almost always the right trade.

### Maintenance

Face recognition is not a one-time operation. New imports bring new faces; new faces sometimes fit existing clusters and sometimes form new ones. A reasonable rhythm:

**Weekly (or after any big import):**
```
maki faces detect --query "*" --apply    # rescans only assets with no faces yet
maki faces cluster --apply               # clusters any newly-unassigned faces
```

A note on what "already detected" means: without `--force`, detection skips any asset that has at least one face record in the catalog. If you delete a face manually (e.g. a bad detection you didn't want), that asset is no longer "done" from the catalog's point of view and will get re-scanned on the next run — which will recreate the same bad detection. Use `--force` on a targeted scope if you want to re-detect anyway, or accept that the bad face will keep coming back until you assign it to a person (assigned faces are still "faces" for this check). Assets where detection ran and found zero faces are currently re-scanned on every run; this is a known limitation on very large catalogs and will likely become a proper "scanned but no face" marker in a future release.

**Occasionally, when clustering quality feels off:**
- Run `faces similarity` on a scope to check the distribution
- Re-cluster with a different `--threshold` or `--min-confidence`
- Review the merge-suggestions panel and fold remaining splinters
- Use `faces clean --apply` to sweep up orphan unassigned faces after pruning

**After upgrading the recognition model** (e.g. a MAKI release with pipeline changes): run `maki faces status` to see how many faces are from the old model, then `faces detect --force` on those scopes to re-embed with the new model. Old embeddings live in a different vector space and are automatically skipped by clustering until replaced.

---

## Similarity Search

The same SigLIP (or SigLIP2) embeddings that power the text-to-image `text:` search also power image-to-image similarity. Every asset with a SigLIP embedding can become the starting point for a "what looks like this?" query.

### The Use Cases

**Finding burst shots and variants.** Ten frames of the same concert moment, shot at 5 fps, with slightly different expressions. They share no tags, they have no filename convention to group them. But their embeddings cluster tightly. Open one, click the "Similar" button on the detail page, and the others are in the top results. Select them all and click "Stack" to collapse them behind a pick.

**Rediscovering forgotten work.** You remember a particular kind of light, a particular mood, a particular composition — but you can't name it. Open any photo that has the feeling you're after, browse by similarity, and the catalog gives you everything that reminds *it* of that photo.

**Quality-controlling a stack.** You've stacked 30 frames from a sequence, designated the pick. Is the pick actually the best one? Browse by similarity from the pick — if you immediately see a visibly better frame, reorder the stack.

### The Mechanics

From the CLI:

```
maki search "similar:72a0bb4b"                  # top 20 most similar
maki search "similar:72a0bb4b:50"               # top 50
maki search "similar:72a0bb4b min_sim:85"       # only >=85% similar
maki search "similar:72a0bb4b tag:portrait"     # similar AND portrait-tagged
```

In the web UI: click **Similar** on the asset detail page, or press `b` in the browse grid to go "browse similar" from the focused card.

### Why `min_sim:` Matters

Raw similarity search returns the top N no matter how dissimilar. With a small catalog that's fine. With 100 000 assets, the top 20 similar results might all be visually related; with 10 assets, you'd get back images that have nothing to do with the source.

`min_sim:85` floors the results at 85% similarity. Use it to cut off the long tail when you're doing exact-match work (finding burst duplicates, stacking variants). Skip it when you're exploring ("show me anything even loosely related").

### Prerequisites

Similarity needs SigLIP embeddings. Either:
- `maki import --embed` to embed at import time (cheap if you do it in batches)
- `maki embed --query "..." --apply` to backfill existing assets

Embeddings are keyed per `(asset_id, model_id)`, so switching models only generates the missing ones — no `--force` needed.

---

## Stroll

Stroll is MAKI's most underappreciated feature. It's what you use when you don't know exactly what you're looking for, or when you want your catalog to show you something you'd forgotten you had.

### The Spatial Metaphor

A regular browse grid is a list: sorted by date, rating, filename. Stroll is a graph: you stand on one asset, and its visual neighbours arrange themselves around it. Click a neighbour to step onto it, and *its* neighbours appear. You wander through your catalog the way you'd wander through a museum — following what draws your eye, with no predetermined path.

### Three Modes, Three Moods

- **Nearest** — the N most visually similar assets, same result every time. Use for finding burst duplicates, stacking candidates, or verifying that a similarity search is stable. This is the default.

- **Discover** — N randomly-sampled assets from a larger pool of similar ones (pool size configurable via `[serve] stroll_discover_pool`, default 80). Each visit produces a different neighbour set. Use for serendipity: breaking out of a tight visual cluster, reminding yourself of work you hadn't thought about in a year, or just browsing for pleasure.

- **Explore** — shows assets ranked N+1 through 2N by similarity, skipping the nearest neighbours entirely. Use when "nearest" keeps returning variants of the same shot and you want to see what else is *near-ish* in the catalog.

Switch modes with the dropdown at the top of the stroll page. The current mode persists per-session; next time you open stroll you're in the same mode you left it in.

### When to Stroll vs. Search

Use **search** when you know the thing's name (a tag, a date, a person, a file pattern). Search gets you precise answers to precise questions.

Use **similarity** when you have one example and want its siblings. Precise question, imprecise filter.

Use **stroll** when you don't have a specific question. You're just looking — for inspiration, for rediscovery, for the moment when your catalog shows you something that stops you in your tracks.

Stroll is not a navigational tool. It's a looking tool.

### Stroll + Filters

The stroll page inherits whatever filters were active on the browse page when you started strolling. Start from a browse filtered to `tag:portrait rating:4+` and your stroll only wanders among 4-star portraits. Start from `date:2015` and you're strolling through one year of your work. This turns stroll from "random walk of the whole catalog" into "focused walk of the subset I'm currently interested in."

---

## Common Problems

### "I ran cluster and nothing happened"

You detected faces on one photo, then clicked *Cluster unassigned faces* hoping MAKI would create new people for the unknowns. It won't: clustering needs at least two similar faces to form a group. If each face is a singleton (one new person appearing in one photo), the cluster command correctly skips all of them.

Use the face-assign combobox on the asset detail page instead. Type a name — even a placeholder like "man with beard" — and click *+ Create new person*. Later photos of the same person will attach to that person when you assign them, and clustering will start forming real groups once there are multiples.

### "Clustering produced one giant cluster"

Threshold too low. Either raise `--threshold` (try 0.5 or 0.6), or raise `--min-confidence` to 0.85 so only high-quality detections participate. Run `faces similarity` on the scope to see the actual distribution before you guess.

### "Clustering produced lots of tiny splinters"

Threshold too high, or the pipeline's embedding quality is poor. First check `faces dump-aligned` — do the crops look healthy? If yes, lower `--threshold` gradually (0.35 → 0.3 → 0.25) and re-cluster. If the crops look bad (mirrored, rotated, too small), check the detection settings and that you're on the current recognition model (`faces status`).

### "Merge suggestions keep suggesting the same pair I dismissed"

Dismissals are per-session (stored in `sessionStorage`). Close the tab and they're gone. If a pair is genuinely not the same person but keeps getting suggested, the only permanent fix is to make one of them more distinct — assign more faces to the correct one so its centroid shifts away from the other.

### "Similarity search returns nonsense"

Two possibilities. First, the asset may not have an embedding — `maki show <id>` shows whether an embedding is present. If not, run `maki embed --asset <id> --apply`. Second, the model may not be suited to the content: SigLIP works well on natural photography, less well on screenshots, diagrams, or heavily-abstract work.

### "Stroll always shows me the same 12 neighbours"

You're in *Nearest* mode. Switch to *Discover* for randomized sampling, or *Explore* to skip the nearest set. Or lower the neighbour count (slider at the top) if you're seeing too much context.

---

## Cheatsheet

Paste this above your desk for the first few weeks.

**Face workflow:**
```
maki faces detect --query "*" --apply       # detect (scans new assets only)
maki faces cluster --apply                  # group into people
# → /people: rename the big clusters
# → /people: clear the merge-suggestions panel
maki faces similarity --query "<scope>"     # diagnose cluster quality
maki faces dump-aligned --query "<scope>"   # visually verify alignment
maki faces status                           # check for stale embeddings
maki faces clean --apply                    # sweep up unassigned orphans
```

**Similarity recipes:**
```
maki search "similar:<id>"                  # top 20 similar
maki search "similar:<id>:50 min_sim:85"    # top 50, only >=85% match
maki search "similar:<id> tag:portrait"     # similar AND tagged
```

**Stroll:**
- `/stroll` or press `s` on a browse card
- Nearest / Discover / Explore — try all three
- Stroll inherits your browse filters

---

For the reference pages behind this chapter, see:

- [Face detection commands](../reference/03-organize-commands.md#maki-faces-detect) — full CLI flags for detect/cluster/similarity/etc.
- [`similar:` and `person:` filters](../reference/06-search-filters.md#person) — search filter reference
- [Web UI stroll page](06-web-ui.md#stroll-page) — the visual controls
- [AI configuration](../reference/08-configuration.md) — models, thresholds, execution providers
