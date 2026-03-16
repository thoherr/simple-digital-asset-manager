# Organizing and Culling

You shoot thousands of images but only show dozens. Every growing collection eventually faces the same challenge: separating the work you want to see from the work you don't -- without losing anything permanently. This chapter covers practical strategies for culling and the default filter feature that ties them together.

---

## Three Populations

In any large catalog, your assets fall into three groups:

1. **Picks** -- curated keepers you actively browse, rate, and share.
2. **Backlog** -- unreviewed imports you haven't looked at yet.
3. **Set aside** -- images you've consciously decided to exclude from everyday browsing (rejects, duplicates, test shots, outtakes).

The goal of a culling workflow is to move assets between these populations efficiently, without deleting anything prematurely.

---

## Rating vs. Curation

Star ratings and inclusion/exclusion serve different purposes. Conflating them leads to awkward workarounds.

**Rating** answers "how good is this image among my keepers?" It is a quality scale within your working set. A 3-star landscape and a 5-star portrait are both part of your active catalog -- they just differ in quality or prominence.

**Curation** answers "should this image appear in my browsing views at all?" It is a visibility gate. A blurry test shot and a duplicate JPEG are not bad images that deserve 1 star -- they are images you don't want to see while browsing.

Keep these concerns separate. Use ratings for quality within your keepers. Use tags, labels, or the absence of a rating for curation.

---

## Workflow Approaches

There is no single correct approach. Choose one that matches how you work, and stay consistent within a project.

### Tag-based culling

Add a `rest` tag to images you want to hide. Set a default filter of `-tag:rest` so they disappear from browsing. Everything else -- rated or not -- stays visible.

```toml
[browse]
default_filter = "-tag:rest"
```

**Strengths:** Simple to understand. Unreviewed imports remain visible by default, so you naturally encounter them. Opt-in exclusion means you only have to act on what you want to hide.

**When to use it:** Most users, especially if you import sporadically and want new work to show up immediately.

### Rating-based culling

Rate your keepers 1--5. Leave rejects and backlog unrated. Set a default filter of `rating:1+` so only rated images appear.

```toml
[browse]
default_filter = "rating:1+"
```

**Strengths:** Clean browsing experience -- only reviewed work appears. Ratings carry meaning across your entire catalog.

**Trade-off:** Unreviewed imports disappear until you rate them. You must do an explicit review pass before new work shows up in the browse grid.

### Color-label workflow

Use color labels for workflow state rather than aesthetic preference. For example: Red = reject, Green = approved, Yellow = needs review.

```toml
[browse]
default_filter = "-label:Red"
```

**Strengths:** Visual and quick in the web UI (keyboard shortcuts r/o/y/g/b for labels). Leaves ratings free for quality scoring. Works well in teams where labels signal handoff state.

### Combined approaches

You can combine filters. For example, show everything that is either rated or not yet tagged as `rest`:

```toml
[browse]
default_filter = "rating:1+ OR -tag:rest"
```

Or hide Red-labeled rejects and rest-tagged shots:

```toml
[browse]
default_filter = "-label:Red -tag:rest"
```

---

## The Default Filter

The `[browse] default_filter` option in `maki.toml` applies a search filter automatically to all browse, search, and stroll views.

### What it does

When set, every browse page load, search query, and stroll session starts with this filter pre-applied. Assets that don't match the filter are hidden from view -- but they remain in the catalog and are fully accessible to CLI commands.

### How to configure it

Add a `[browse]` section to your `maki.toml`:

```toml
[browse]
default_filter = "-tag:rest"
```

The value is any valid search filter string -- the same syntax you use with `maki search`. See the [Search Filters Reference](../reference/06-search-filters.md) for the full syntax.

### Toggling in the web UI

The web UI shows a checkbox in the filter bar when a default filter is configured. Unchecking it temporarily disables the filter for the current session, letting you see everything. Re-checking it restores the filter. This makes it easy to peek at hidden assets without editing `maki.toml`.

### Scope

The default filter applies to **browsing views only**: the browse grid, search results, and the stroll page. It does **not** affect operational commands like `maki export`, `maki describe`, `maki verify`, or `maki search` on the CLI. Those commands always operate on the full catalog unless you explicitly pass a query.

### Examples for common workflows

```toml
# Hide images tagged "rest" (recommended starting point)
[browse]
default_filter = "-tag:rest"

# Show only rated images
[browse]
default_filter = "rating:1+"

# Hide Red-labeled rejects
[browse]
default_filter = "-label:Red"

# Show only images, hide videos and documents
[browse]
default_filter = "type:image"

# Combine: hide rest-tagged and Red-labeled
[browse]
default_filter = "-tag:rest -label:Red"
```

---

## Practical Workflow Examples

### Wedding photographer

1. Import the shoot: `maki import /Volumes/Cards/DCIM --log`
2. Quick scan in the web UI. Tag obvious rejects (out of focus, closed eyes, test flashes) as `rest` using keyboard shortcuts or batch select.
3. Rate keepers: 3 = good, 4 = strong, 5 = hero shots.
4. Deliver 4--5 star images to client: `maki export "rating:4+" /Volumes/Delivery/`
5. Build a portfolio collection from the best: `maki search -q "rating:5" | xargs maki col add "Wedding Portfolio"`

Default filter: `-tag:rest`

### Travel and personal

1. Import after a trip: `maki import ~/Photos/Iceland/ --log`
2. Browse at leisure. Rate favorites as you go -- no pressure to review everything immediately.
3. Revisit unrated images when you feel like it. Tag anything truly unwanted as `rest`.
4. Over time, the unrated backlog shrinks naturally.

Default filter: `-tag:rest`

### Stock and archive

1. Import with descriptive tags: `maki import /Volumes/Stock/ --add-tag stock --log`
2. Use `maki describe` to generate AI descriptions for searchability.
3. Organize into collections by theme: "Architecture", "Food", "Travel".
4. Rate for commercial potential: 5 = lead image, 4 = strong, 3 = filler.
5. Default filter hides low-rated filler from the browse grid.

Default filter: `rating:3+`

### Art and portfolio

1. Import selectively -- only work you consider keeping.
2. Rate carefully for portfolio tiers:
   - 5 = hero image (exhibition, cover)
   - 4 = portfolio (website gallery)
   - 3 = extended portfolio (social media)
   - 2 = decent (personal archive)
   - 1 = keeper (not for sharing)
3. Build saved searches for each tier: `maki ss save "Portfolio" "rating:4+" --sort date_desc`
4. Use collections for specific series or exhibitions.

Default filter: `rating:2+` (hide the bottom tier from daily browsing)

---

## Tips

**Start simple.** A single `rest` tag and `-tag:rest` default filter covers most needs. You can add ratings, labels, and more structure later without changing what you've already done.

**The `rest` tag approach is recommended for most users** because unreviewed images stay visible by default. You only act on what you want to hide, which is usually the smaller set.

**Bulk-tag the remainder after a culling session.** After rating your keepers from a shoot, tag everything unrated as `rest` in one command:

```
maki search -q "path:Capture/2026-03-15 -rating:1+" | xargs maki tag rest
```

This clears the backlog for that shoot without deleting anything.

**Ratings are personal and subjective.** Be consistent within a project, but don't worry about cross-project consistency. A 4-star wedding photo and a 4-star landscape don't need to be comparable.

**Use saved searches for recurring views.** Rather than remembering filter syntax, save your common queries:

```
maki ss save "Portfolio candidates" "rating:4+ -tag:rest" --sort date_desc
maki ss save "Unreviewed this month" "date:2026-03 -rating:1+ -tag:rest" --sort date_desc
maki ss save "Needs keywords" "-tag:rest rating:1+ tag:untagged" --sort date_desc
```

**Don't delete -- set aside.** The `rest` tag is reversible. Deletion is not. You can always reconsider an image later by toggling off the default filter in the web UI.

---

Previous: [Interactive Shell](09-shell.md) |
[Back to Manual](../index.md)
