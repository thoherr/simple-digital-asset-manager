# Organize Commands

Commands for curating static collections, managing saved searches (smart albums), grouping assets into stacks, and managing face recognition. Collections are manual albums; saved searches are dynamic queries that always reflect the current catalog state. Stacks collapse related assets in the browse grid.

| Command group | Subcommands |
|---------|-------------|
| [collection](#maki-collection-create) | create, list, show, add, remove, delete |
| [saved-search](#maki-saved-search-save) | save, list, run, delete |
| [stack](#maki-stack-create) | create, add, remove, pick, dissolve, list, show, from-tag |
| [faces](#maki-faces-detect) *(Pro)* | detect, cluster, people, name, merge, delete-person, unassign, export, download, status |

---

## maki collection create

### NAME

maki-collection-create -- create a new collection

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection create <NAME> [--description <TEXT>]
```

Alias: `maki col create`

### DESCRIPTION

Creates a new empty collection. Collections are manually curated lists of asset IDs, similar to static albums in photo management tools. They are backed by SQLite tables for fast queries and a `collections.yaml` file at the catalog root for persistence across `rebuild-catalog`.

Collection names must be unique. Attempting to create a collection with an existing name produces an error.

### ARGUMENTS

**NAME** (required)
: The name for the new collection.

### OPTIONS

**--description \<TEXT\>**
: An optional description for the collection.

`--json` outputs the created collection's details.

### EXAMPLES

Create a simple collection:

```bash
maki collection create "Best of 2026"
```

Create a collection with a description:

```bash
maki col create "Wedding Portfolio" --description "Final selects for client delivery"
```

Create with JSON output:

```bash
maki col create "Travel" --json
```

### SEE ALSO

[collection add](#maki-collection-add) -- add assets to a collection.
[collection show](#maki-collection-show) -- view collection contents.
[search](04-retrieve-commands.md#maki-search) -- `collection:` filter for searching within a collection.

---

## maki collection list

### NAME

maki-collection-list -- list all collections

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection list
```

Alias: `maki col list`

### DESCRIPTION

Lists all collections in the catalog, showing each collection's name, description (if any), and the number of assets it contains.

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs an array of collection objects.

### EXAMPLES

List all collections:

```bash
maki collection list
```

List collections as JSON and extract names:

```bash
maki col list --json | jq '.[].name'
```

Count total collections:

```bash
maki col list --json | jq 'length'
```

### SEE ALSO

[collection create](#maki-collection-create) -- create a new collection.
[collection show](#maki-collection-show) -- view a specific collection's contents.

---

## maki collection show

### NAME

maki-collection-show -- show the contents of a collection

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection show <NAME> [--format <FMT>]
```

Alias: `maki col show`

### DESCRIPTION

Displays the assets belonging to a collection. Output format can be customized using the same format presets and template syntax as `maki search`.

### ARGUMENTS

**NAME** (required)
: The name of the collection to display.

### OPTIONS

**--format \<FMT\>**
: Output format. Presets: `ids`, `short` (default), `full`, `json`. Custom templates use `{placeholder}` syntax (e.g., `'{id}\t{name}'`).

### EXAMPLES

Show a collection's contents:

```bash
maki collection show "Best of 2026"
```

Get just the asset IDs for piping:

```bash
maki col show "Wedding Portfolio" --format ids
```

Show full details including tags:

```bash
maki col show "Travel" --format full
```

Export collection as JSON:

```bash
maki col show "Favorites" --format json | jq '.[].id'
```

### SEE ALSO

[collection add](#maki-collection-add) -- add assets to the collection.
[collection remove](#maki-collection-remove) -- remove assets from the collection.
[search](04-retrieve-commands.md#maki-search) -- `collection:` filter for searching within collections.

---

## maki collection add

### NAME

maki-collection-add -- add assets to a collection

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection add <NAME> <ASSET_IDS...>
```

Alias: `maki col add`

### DESCRIPTION

Adds one or more assets to an existing collection. Asset IDs that are already in the collection are silently ignored (no duplicates are created).

Supports stdin piping for integration with `maki search -q` and shell scripting.

### ARGUMENTS

**NAME** (required)
: The name of the collection to add assets to.

**ASSET_IDS** (required)
: One or more asset IDs to add. Also accepts IDs from stdin.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Add specific assets to a collection:

```bash
maki collection add "Favorites" a1b2c3d4-... e5f67890-...
```

Pipe search results into a collection:

```bash
maki search -q "rating:5 tag:travel" | xargs maki col add "Travel Best"
```

Add all 5-star landscape photos to a collection:

```bash
maki search -q "rating:5 tag:landscape" | xargs maki col add "Portfolio"
```

Add assets from a saved search:

```bash
maki ss run "Recent Imports" --format ids | xargs maki col add "Review Queue"
```

### SEE ALSO

[collection remove](#maki-collection-remove) -- remove assets from a collection.
[collection show](#maki-collection-show) -- view collection contents.
[search](04-retrieve-commands.md#maki-search) -- find assets to add.

---

## maki collection remove

### NAME

maki-collection-remove -- remove assets from a collection

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection remove <NAME> <ASSET_IDS...>
```

Alias: `maki col remove`

### DESCRIPTION

Removes one or more assets from a collection. The assets themselves are not deleted -- only their membership in the collection is removed. Asset IDs not present in the collection are silently ignored.

### ARGUMENTS

**NAME** (required)
: The name of the collection to remove assets from.

**ASSET_IDS** (required)
: One or more asset IDs to remove.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Remove a single asset from a collection:

```bash
maki collection remove "Favorites" a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Remove multiple assets:

```bash
maki col remove "Review Queue" a1b2c3d4-... e5f67890-...
```

Remove all assets with a certain label from a collection:

```bash
maki search -q "collection:Portfolio label:Red" --format ids | xargs maki col remove "Portfolio"
```

### SEE ALSO

[collection add](#maki-collection-add) -- add assets to a collection.
[collection delete](#maki-collection-delete) -- delete the entire collection.

---

## maki collection delete

### NAME

maki-collection-delete -- delete a collection

### SYNOPSIS

```
maki [GLOBAL FLAGS] collection delete <NAME>
```

Alias: `maki col delete`

### DESCRIPTION

Deletes a collection entirely. This removes the collection record and all its membership entries. The assets themselves are not affected -- only the collection is removed.

### ARGUMENTS

**NAME** (required)
: The name of the collection to delete.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Delete a collection:

```bash
maki collection delete "Old Review Queue"
```

Delete using the alias:

```bash
maki col delete "Temporary"
```

Delete with JSON confirmation:

```bash
maki col delete "Drafts" --json
```

### SEE ALSO

[collection create](#maki-collection-create) -- create a new collection.
[collection list](#maki-collection-list) -- list all collections.

---

## maki saved-search save

### NAME

maki-saved-search-save -- save a search query with a name

### SYNOPSIS

```
maki [GLOBAL FLAGS] saved-search save <NAME> <QUERY> [--sort <SORT>] [--favorite]
```

Alias: `maki ss save`

### DESCRIPTION

Saves a search query under a name for later re-use. Saved searches are stored in `searches.toml` at the catalog root and function as smart albums -- the results update dynamically as the catalog changes.

If a saved search with the same name already exists, it is replaced.

Saved searches appear as clickable chips in the web UI browse page and can be executed from the CLI with `maki saved-search run`.

### ARGUMENTS

**NAME** (required)
: A name for the saved search.

**QUERY** (required)
: The search query string, using the same syntax as `maki search`.

### OPTIONS

**--sort \<SORT\>**
: Sort order for results. Values: `date_desc` (default), `date_asc`, `name_asc`, `name_desc`, `size_asc`, `size_desc`.

**--favorite**
: Mark the saved search as a favorite. Favorite searches are shown prominently as chips on the web UI browse page.

`--json` outputs the saved search entry.

### EXAMPLES

Save a search for highly-rated landscapes:

```bash
maki saved-search save "Best Landscapes" "tag:landscape rating:4+"
```

Save with a custom sort order:

```bash
maki ss save "Recent Videos" "type:video" --sort date_desc
```

Save a search using quoted filter values:

```bash
maki ss save "Canon Portraits" 'camera:"Canon EOS R5" tag:portrait'
```

Save a path-scoped search:

```bash
maki ss save "February Shoot" "path:Capture/2026-02"
```

### SEE ALSO

[saved-search run](#maki-saved-search-run) -- execute a saved search.
[saved-search list](#maki-saved-search-list) -- list all saved searches.
[saved-search delete](#maki-saved-search-delete) -- delete a saved search.
[search](04-retrieve-commands.md#maki-search) -- query syntax reference.

---

## maki saved-search list

### NAME

maki-saved-search-list -- list all saved searches

### SYNOPSIS

```
maki [GLOBAL FLAGS] saved-search list
```

Alias: `maki ss list`

### DESCRIPTION

Lists all saved searches stored in the catalog, showing each search's name, query, and sort order.

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs an array of saved search objects.

### EXAMPLES

List all saved searches:

```bash
maki saved-search list
```

List as JSON:

```bash
maki ss list --json
```

Count saved searches:

```bash
maki ss list --json | jq 'length'
```

### SEE ALSO

[saved-search save](#maki-saved-search-save) -- create or update a saved search.
[saved-search run](#maki-saved-search-run) -- execute a saved search.

---

## maki saved-search run

### NAME

maki-saved-search-run -- execute a saved search and display results

### SYNOPSIS

```
maki [GLOBAL FLAGS] saved-search run <NAME> [--format <FMT>]
```

Alias: `maki ss run`

### DESCRIPTION

Executes a previously saved search by name and displays the results. The stored query is run against the current state of the catalog, so results reflect any changes since the search was saved.

The sort order saved with the search is applied. Output format can be overridden with `--format`.

### ARGUMENTS

**NAME** (required)
: The name of the saved search to execute.

### OPTIONS

**--format \<FMT\>**
: Output format. Presets: `ids`, `short` (default), `full`, `json`. Custom templates use `{placeholder}` syntax.

### EXAMPLES

Run a saved search:

```bash
maki saved-search run "Best Landscapes"
```

Run and get just IDs for piping:

```bash
maki ss run "Recent Videos" --format ids
```

Run a saved search and add results to a collection:

```bash
maki ss run "Best Landscapes" --format ids | xargs maki col add "Portfolio"
```

Run with JSON output:

```bash
maki ss run "Canon Portraits" --format json | jq '.[].id'
```

### SEE ALSO

[saved-search save](#maki-saved-search-save) -- create or update a saved search.
[collection add](03-organize-commands.md#maki-collection-add) -- add search results to a collection.

---

## maki saved-search delete

### NAME

maki-saved-search-delete -- delete a saved search

### SYNOPSIS

```
maki [GLOBAL FLAGS] saved-search delete <NAME>
```

Alias: `maki ss delete`

### DESCRIPTION

Deletes a saved search by name. The search is removed from `searches.toml`. This does not affect any assets or collections.

### ARGUMENTS

**NAME** (required)
: The name of the saved search to delete.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Delete a saved search:

```bash
maki saved-search delete "Old Query"
```

Delete using the alias:

```bash
maki ss delete "Temporary Search"
```

Delete with JSON confirmation:

```bash
maki ss delete "Drafts" --json
```

### SEE ALSO

[saved-search save](#maki-saved-search-save) -- create a new saved search.
[saved-search list](#maki-saved-search-list) -- list all saved searches.

---

## maki stack create

### NAME

maki-stack-create -- create a new stack from the given assets

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack create <ASSET_IDS...>
```

Alias: `maki st create`

### DESCRIPTION

Creates a new stack from two or more assets. Stacks are lightweight anonymous groups for visually related images -- burst shots, bracketing sequences, similar scenes. The first asset ID becomes the stack's "pick" (displayed in the browse grid).

Each asset can belong to at most one stack. Attempting to stack an asset that is already in a stack produces an error.

Stacks are persisted in `stacks.yaml` at the catalog root and restored during `rebuild-catalog`.

### ARGUMENTS

**ASSET_IDS** (required)
: Two or more asset IDs. The first ID becomes the pick (position 0).

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs the created stack's details (id, member_count, asset_ids).

### EXAMPLES

Create a stack from three burst shots (first becomes pick):

```bash
maki stack create a1b2c3d4-... e5f67890-... f1a2b3c4-...
```

Create using the alias with JSON output:

```bash
maki st create a1b2c3d4-... e5f67890-... --json
```

### SEE ALSO

[stack add](#maki-stack-add) -- add assets to an existing stack.
[stack pick](#maki-stack-pick) -- change which asset is the pick.
[stack dissolve](#maki-stack-dissolve) -- dissolve a stack entirely.

---

## maki stack add

### NAME

maki-stack-add -- add assets to an existing stack

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack add <REFERENCE> <ASSET_IDS...>
```

Alias: `maki st add`

### DESCRIPTION

Adds one or more assets to an existing stack. The reference asset identifies which stack to add to -- it must already be a member of a stack. New assets are appended to the end of the stack's member list.

Assets that are already in a stack (any stack) cannot be added and will produce an error.

### ARGUMENTS

**REFERENCE** (required)
: Any asset ID that is already in the target stack.

**ASSET_IDS** (required)
: One or more asset IDs to add to the stack.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"added": N}` with the count of assets added.

### EXAMPLES

Add an asset to the stack containing a1b2c3d4:

```bash
maki stack add a1b2c3d4-... new-asset-id-...
```

Add multiple assets:

```bash
maki st add a1b2c3d4-... b2c3d4e5-... c3d4e5f6-...
```

### SEE ALSO

[stack create](#maki-stack-create) -- create a new stack.
[stack remove](#maki-stack-remove) -- remove assets from a stack.

---

## maki stack remove

### NAME

maki-stack-remove -- remove assets from their stack

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack remove <ASSET_IDS...>
```

Alias: `maki st remove`

### DESCRIPTION

Removes one or more assets from their stacks. Each asset is removed from whatever stack it belongs to. If removing an asset causes a stack to have fewer than 2 members, the stack is automatically dissolved.

Assets not currently in a stack are silently skipped.

### ARGUMENTS

**ASSET_IDS** (required)
: One or more asset IDs to remove from their stacks.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"removed": N}` with the count of assets removed.

### EXAMPLES

Remove a single asset from its stack:

```bash
maki stack remove a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Remove multiple assets:

```bash
maki st remove a1b2c3d4-... e5f67890-...
```

### SEE ALSO

[stack add](#maki-stack-add) -- add assets to a stack.
[stack dissolve](#maki-stack-dissolve) -- dissolve an entire stack at once.

---

## maki stack pick

### NAME

maki-stack-pick -- set the pick (top) of a stack

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack pick <ASSET_ID>
```

Alias: `maki st pick`

### DESCRIPTION

Promotes an asset to position 0 (the "pick") of its stack. The pick is the asset shown in the browse grid when stacks are collapsed. The asset must already be a member of a stack.

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID to make the pick. Must be in a stack.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"pick": "<asset_id>"}`.

### EXAMPLES

Set a specific asset as the stack pick:

```bash
maki stack pick a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Set pick using the alias:

```bash
maki st pick a1b2c3d4-...
```

### SEE ALSO

[stack show](#maki-stack-show) -- view current stack members and pick.
[stack create](#maki-stack-create) -- the first asset in create becomes the initial pick.

---

## maki stack dissolve

### NAME

maki-stack-dissolve -- dissolve an entire stack

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack dissolve <ASSET_ID>
```

Alias: `maki st dissolve`

### DESCRIPTION

Dissolves the stack that contains the given asset. All member assets become unstacked. The assets themselves are not affected -- only the stack grouping is removed.

### ARGUMENTS

**ASSET_ID** (required)
: Any asset ID in the stack to dissolve.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"status": "dissolved"}`.

### EXAMPLES

Dissolve a stack:

```bash
maki stack dissolve a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Dissolve using the alias:

```bash
maki st dissolve a1b2c3d4-...
```

### SEE ALSO

[stack remove](#maki-stack-remove) -- remove individual assets instead of dissolving the whole stack.
[stack list](#maki-stack-list) -- list all stacks.

---

## maki stack list

### NAME

maki-stack-list -- list all stacks

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack list
```

Alias: `maki st list`

### DESCRIPTION

Lists all stacks in the catalog, showing each stack's ID, member count, creation date, and pick asset ID.

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs an array of `StackSummary` objects.

### EXAMPLES

List all stacks:

```bash
maki stack list
```

List as JSON and count stacks:

```bash
maki st list --json | jq 'length'
```

### SEE ALSO

[stack show](#maki-stack-show) -- view members of a specific stack.
[stack create](#maki-stack-create) -- create a new stack.

---

## maki stack show

### NAME

maki-stack-show -- show members of a stack

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack show <ASSET_ID> [--format <FMT>]
```

Alias: `maki st show`

### DESCRIPTION

Displays the ordered member list of the stack containing the given asset. The pick (position 0) is shown first. Output format can be customized using the same format presets and template syntax as `maki search`.

### ARGUMENTS

**ASSET_ID** (required)
: Any asset ID that belongs to a stack.

### OPTIONS

**--format \<FMT\>**
: Output format. Presets: `ids`, `short` (default), `full`, `json`. Custom templates use `{placeholder}` syntax.

### EXAMPLES

Show members of a stack:

```bash
maki stack show a1b2c3d4-...
```

Get just the member IDs:

```bash
maki st show a1b2c3d4-... --format ids
```

Show as JSON:

```bash
maki st show a1b2c3d4-... --json
```

### SEE ALSO

[stack pick](#maki-stack-pick) -- change the pick.
[stack list](#maki-stack-list) -- list all stacks.
[search](04-retrieve-commands.md#maki-search) -- `stacked:true` filter finds all stacked assets.

---

## maki stack from-tag

### NAME

maki-stack-from-tag -- convert matching tags into stacks

### SYNOPSIS

```
maki [GLOBAL FLAGS] stack from-tag <PATTERN> [--remove-tags] [--apply]
```

Alias: `maki st from-tag`

### DESCRIPTION

Finds assets whose tags match a pattern containing a `{}` wildcard, groups them by the wildcard value, and creates a stack from each group. This automates the common workflow of converting tag-based groupings (e.g., from CaptureOne or Lightroom) into maki stacks.

Without `--apply`, runs in **report-only mode** showing what stacks would be created. With `--apply`, creates the stacks. With `--remove-tags`, the matched tag is removed from each asset after stacking.

### ARGUMENTS

**PATTERN** (required)
: A tag pattern with `{}` as a wildcard placeholder. For example, `"Aperture Stack {}"` matches tags like "Aperture Stack 1", "Aperture Stack 2", etc.

### OPTIONS

**--remove-tags**
: Remove the matched tag from each asset after stack creation. Only effective with `--apply`.

**--apply**
: Actually create stacks. Without this flag, the command only reports what it would do.

### EXAMPLES

Preview what stacks would be created from aperture stack tags:

```bash
maki stack from-tag "Aperture Stack {}"
```

Create stacks and remove the grouping tags:

```bash
maki st from-tag "Aperture Stack {}" --remove-tags --apply
```

Create stacks from bracket sequence tags:

```bash
maki stack from-tag "Bracket {}" --apply
```

### SEE ALSO

[stack create](#maki-stack-create) -- manually create a stack from specific assets.
[stack dissolve](#maki-stack-dissolve) -- dissolve stacks if the result is unwanted.
[tag](02-ingest-commands.md#maki-tag) -- manage asset tags.

---

---

## maki faces detect *(Pro)* {#maki-faces-detect}

### NAME

maki-faces-detect -- detect faces in asset images

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces detect [--query <Q>] [--asset <id>] [--volume <label>] [--apply]
```

### DESCRIPTION

Detects faces in asset preview images using the YuNet ONNX model. For each detected face, computes a 512-dimensional ArcFace embedding and generates a 150×150 JPEG crop thumbnail.

Without `--apply`, runs in report-only mode showing how many faces would be detected. With `--apply`, stores face records in the catalog and generates crop thumbnails.

Requires at least one scope filter (`--query`, `--asset`, or `--volume`) to prevent accidental full-catalog processing. Models must be downloaded first with `maki faces download`.

### OPTIONS

**--query \<Q\>**
: Search query to scope which assets are processed.

**--asset \<id\>**
: Process a single asset by ID.

**--volume \<label\>**
: Process assets on a specific volume.

**--min-confidence \<FLOAT\>**
: Minimum detection confidence threshold (0.0–1.0, default 0.5). Faces below this confidence are discarded.

**--apply**
: Actually store detected faces (default is dry run).

**--force**
: Re-detect faces even if faces already exist for an asset. Without this flag, assets with existing face records are skipped.

`--json`, `--log`, `--time` for output control.

### EXAMPLES

Detect faces in all images (dry run):

```bash
maki faces detect --query "type:image"
```

Detect and store faces for a single asset:

```bash
maki faces detect --asset a1b2c3d4 --apply
```

Detect faces on a specific volume with logging:

```bash
maki faces detect --volume "Photos" --apply --log
```

### SEE ALSO

[faces cluster](#maki-faces-cluster) -- group detected faces into people.
[faces download](#maki-faces-download) -- download required models.

---

## maki faces cluster *(Pro)* {#maki-faces-cluster}

### NAME

maki-faces-cluster -- group similar faces into people

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces cluster [--query <Q>] [--asset <id>] [--volume <label>] [--threshold <F>] [--apply]
```

### DESCRIPTION

Groups similar face embeddings into unnamed person groups using greedy single-linkage clustering. Faces that have already been assigned to a person are skipped.

The threshold controls how similar two faces must be to be grouped together (0.0–1.0, higher = stricter). Default is 0.5, configurable via `[ai] face_cluster_threshold` in `maki.toml`.

Without `--apply`, shows a dry-run report of cluster sizes. With `--apply`, creates person records and assigns faces.

Scope filters (`--query`, `--asset`, `--volume`) limit which faces are considered for clustering.

### OPTIONS

**--query \<Q\>**
: Scope clustering to faces on assets matching this query.

**--asset \<id\>**
: Scope clustering to faces on a single asset.

**--volume \<label\>**
: Scope clustering to faces on assets on a specific volume.

**--threshold \<F\>**
: Similarity threshold for clustering (default 0.5).

**--apply**
: Actually create person groups (default is dry run).

`--json`, `--log`, `--time` for output control.

### EXAMPLES

Preview clustering results:

```bash
maki faces cluster
```

Apply clustering with a stricter threshold:

```bash
maki faces cluster --threshold 0.6 --apply
```

Cluster only faces from a specific shoot:

```bash
maki faces cluster --query "path:Capture/2026-03" --apply
```

### SEE ALSO

[faces detect](#maki-faces-detect) -- detect faces first.
[faces name](#maki-faces-name) -- name the resulting person groups.

---

## maki faces people *(Pro)*

### NAME

maki-faces-people -- list all people

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces people
```

### DESCRIPTION

Lists all people in the catalog with their names (if assigned) and face counts.

### EXAMPLES

```bash
maki faces people
maki faces people --json
```

### SEE ALSO

[faces name](#maki-faces-name) -- name a person.

---

## maki faces name *(Pro)* {#maki-faces-name}

### NAME

maki-faces-name -- name a person

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces name <PERSON_ID> <NAME>
```

### DESCRIPTION

Assigns a human-readable name to a person. Person IDs are shown by `maki faces people`.

### ARGUMENTS

**PERSON_ID** (required)
: The person's UUID.

**NAME** (required)
: The name to assign.

### EXAMPLES

```bash
maki faces name 550e8400-... "Alice"
```

---

## maki faces merge *(Pro)*

### NAME

maki-faces-merge -- merge two people

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces merge <TARGET_ID> <SOURCE_ID>
```

### DESCRIPTION

Moves all faces from the source person to the target person, then deletes the source person. Useful for combining duplicate person groups after clustering.

### ARGUMENTS

**TARGET_ID** (required)
: The person to keep.

**SOURCE_ID** (required)
: The person to merge into the target (deleted after merge).

### EXAMPLES

```bash
maki faces merge 550e8400-... 661f9511-...
```

---

## maki faces delete-person *(Pro)*

### NAME

maki-faces-delete-person -- delete a person

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces delete-person <PERSON_ID>
```

### DESCRIPTION

Deletes a person record. All faces assigned to this person become unassigned (they are not deleted).

### ARGUMENTS

**PERSON_ID** (required)
: The person's UUID.

### EXAMPLES

```bash
maki faces delete-person 550e8400-...
```

---

## maki faces unassign *(Pro)*

### NAME

maki-faces-unassign -- remove a face from its person

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces unassign <FACE_ID>
```

### DESCRIPTION

Removes the person assignment from a single face. The face record is preserved; only the person link is cleared.

### ARGUMENTS

**FACE_ID** (required)
: The face's UUID.

### EXAMPLES

```bash
maki faces unassign a1b2c3d4-...
```

---

## maki faces export *(Pro)*

### NAME

maki-faces-export -- export faces and people to YAML and binary files

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces export
```

### DESCRIPTION

Exports all face and people data from the SQLite catalog to file-based storage:

- `faces.yaml` — face records (bounding boxes, confidence, person assignments)
- `people.yaml` — people records (names, representative faces)
- `embeddings/arcface/<prefix>/<face_id>.bin` — ArcFace face recognition embeddings as raw binary files

This is a one-time migration command for catalogs that have existing face data in SQLite but no corresponding YAML/binary files (i.e., data created before v2.2.1). After running this command, `rebuild-catalog` will be able to restore face and people data.

Going forward, all face/people write operations automatically persist to both SQLite and files.

### EXAMPLES

Export all face data:

```bash
maki faces export
```

Export with JSON output:

```bash
maki faces export --json
```

### SEE ALSO

[faces detect](#maki-faces-detect) -- detect faces in images.
[rebuild-catalog](05-maintain-commands.md#maki-rebuild-catalog) -- rebuilds catalog from files.

---

## maki faces download *(Pro)* {#maki-faces-download}

### NAME

maki-faces-download -- download face detection models

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces download
```

### DESCRIPTION

Downloads the YuNet face detection model and ArcFace face recognition model from HuggingFace. Models are cached in the model directory (default `~/.cache/maki/models`, configurable via `[ai] model_dir` in `maki.toml`).

### EXAMPLES

```bash
maki faces download
```

---

## maki faces status *(Pro)*

### NAME

maki-faces-status -- show face detection model status

### SYNOPSIS

```
maki [GLOBAL FLAGS] faces status
```

### DESCRIPTION

Shows the download status of face detection and recognition models. Reports whether the YuNet face detection model and ArcFace face recognition model are downloaded and ready to use.

### EXAMPLES

```bash
maki faces status
```

### SEE ALSO

[faces download](#maki-faces-download) -- download face models.
[faces detect](#maki-faces-detect) -- detect faces in images.

---

Previous: [Ingest Commands](02-ingest-commands.md) -- `import`, `tag`, `edit`, `group`, `auto-group`.
Next: [Retrieve Commands](04-retrieve-commands.md) -- `search`, `show`, `duplicates`, `stats`, `serve`.
