# Organize Commands

Commands for curating static collections, managing saved searches (smart albums), and grouping assets into stacks.

---

## dam collection create

### NAME

dam-collection-create -- create a new collection

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection create <NAME> [--description <TEXT>]
```

Alias: `dam col create`

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
dam collection create "Best of 2026"
```

Create a collection with a description:

```bash
dam col create "Wedding Portfolio" --description "Final selects for client delivery"
```

Create with JSON output:

```bash
dam col create "Travel" --json
```

### SEE ALSO

[collection add](#dam-collection-add) -- add assets to a collection.
[collection show](#dam-collection-show) -- view collection contents.
[search](04-retrieve-commands.md#dam-search) -- `collection:` filter for searching within a collection.

---

## dam collection list

### NAME

dam-collection-list -- list all collections

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection list
```

Alias: `dam col list`

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
dam collection list
```

List collections as JSON and extract names:

```bash
dam col list --json | jq '.[].name'
```

Count total collections:

```bash
dam col list --json | jq 'length'
```

### SEE ALSO

[collection create](#dam-collection-create) -- create a new collection.
[collection show](#dam-collection-show) -- view a specific collection's contents.

---

## dam collection show

### NAME

dam-collection-show -- show the contents of a collection

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection show <NAME> [--format <FMT>]
```

Alias: `dam col show`

### DESCRIPTION

Displays the assets belonging to a collection. Output format can be customized using the same format presets and template syntax as `dam search`.

### ARGUMENTS

**NAME** (required)
: The name of the collection to display.

### OPTIONS

**--format \<FMT\>**
: Output format. Presets: `ids`, `short` (default), `full`, `json`. Custom templates use `{placeholder}` syntax (e.g., `'{id}\t{name}'`).

### EXAMPLES

Show a collection's contents:

```bash
dam collection show "Best of 2026"
```

Get just the asset IDs for piping:

```bash
dam col show "Wedding Portfolio" --format ids
```

Show full details including tags:

```bash
dam col show "Travel" --format full
```

Export collection as JSON:

```bash
dam col show "Favorites" --format json | jq '.[].id'
```

### SEE ALSO

[collection add](#dam-collection-add) -- add assets to the collection.
[collection remove](#dam-collection-remove) -- remove assets from the collection.
[search](04-retrieve-commands.md#dam-search) -- `collection:` filter for searching within collections.

---

## dam collection add

### NAME

dam-collection-add -- add assets to a collection

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection add <NAME> <ASSET_IDS...>
```

Alias: `dam col add`

### DESCRIPTION

Adds one or more assets to an existing collection. Asset IDs that are already in the collection are silently ignored (no duplicates are created).

Supports stdin piping for integration with `dam search -q` and shell scripting.

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
dam collection add "Favorites" a1b2c3d4-... e5f67890-...
```

Pipe search results into a collection:

```bash
dam search -q "rating:5 tag:travel" | xargs dam col add "Travel Best"
```

Add all 5-star landscape photos to a collection:

```bash
dam search -q "rating:5 tag:landscape" | xargs dam col add "Portfolio"
```

Add assets from a saved search:

```bash
dam ss run "Recent Imports" --format ids | xargs dam col add "Review Queue"
```

### SEE ALSO

[collection remove](#dam-collection-remove) -- remove assets from a collection.
[collection show](#dam-collection-show) -- view collection contents.
[search](04-retrieve-commands.md#dam-search) -- find assets to add.

---

## dam collection remove

### NAME

dam-collection-remove -- remove assets from a collection

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection remove <NAME> <ASSET_IDS...>
```

Alias: `dam col remove`

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
dam collection remove "Favorites" a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Remove multiple assets:

```bash
dam col remove "Review Queue" a1b2c3d4-... e5f67890-...
```

Remove all assets with a certain label from a collection:

```bash
dam search -q "collection:Portfolio label:Red" --format ids | xargs dam col remove "Portfolio"
```

### SEE ALSO

[collection add](#dam-collection-add) -- add assets to a collection.
[collection delete](#dam-collection-delete) -- delete the entire collection.

---

## dam collection delete

### NAME

dam-collection-delete -- delete a collection

### SYNOPSIS

```
dam [GLOBAL FLAGS] collection delete <NAME>
```

Alias: `dam col delete`

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
dam collection delete "Old Review Queue"
```

Delete using the alias:

```bash
dam col delete "Temporary"
```

Delete with JSON confirmation:

```bash
dam col delete "Drafts" --json
```

### SEE ALSO

[collection create](#dam-collection-create) -- create a new collection.
[collection list](#dam-collection-list) -- list all collections.

---

## dam saved-search save

### NAME

dam-saved-search-save -- save a search query with a name

### SYNOPSIS

```
dam [GLOBAL FLAGS] saved-search save <NAME> <QUERY> [--sort <SORT>]
```

Alias: `dam ss save`

### DESCRIPTION

Saves a search query under a name for later re-use. Saved searches are stored in `searches.toml` at the catalog root and function as smart albums -- the results update dynamically as the catalog changes.

If a saved search with the same name already exists, it is replaced.

Saved searches appear as clickable chips in the web UI browse page and can be executed from the CLI with `dam saved-search run`.

### ARGUMENTS

**NAME** (required)
: A name for the saved search.

**QUERY** (required)
: The search query string, using the same syntax as `dam search`.

### OPTIONS

**--sort \<SORT\>**
: Sort order for results. Values: `date_desc` (default), `date_asc`, `name_asc`, `name_desc`, `size_asc`, `size_desc`.

`--json` outputs the saved search entry.

### EXAMPLES

Save a search for highly-rated landscapes:

```bash
dam saved-search save "Best Landscapes" "tag:landscape rating:4+"
```

Save with a custom sort order:

```bash
dam ss save "Recent Videos" "type:video" --sort date_desc
```

Save a search using quoted filter values:

```bash
dam ss save "Canon Portraits" 'camera:"Canon EOS R5" tag:portrait'
```

Save a path-scoped search:

```bash
dam ss save "February Shoot" "path:Capture/2026-02"
```

### SEE ALSO

[saved-search run](#dam-saved-search-run) -- execute a saved search.
[saved-search list](#dam-saved-search-list) -- list all saved searches.
[saved-search delete](#dam-saved-search-delete) -- delete a saved search.
[search](04-retrieve-commands.md#dam-search) -- query syntax reference.

---

## dam saved-search list

### NAME

dam-saved-search-list -- list all saved searches

### SYNOPSIS

```
dam [GLOBAL FLAGS] saved-search list
```

Alias: `dam ss list`

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
dam saved-search list
```

List as JSON:

```bash
dam ss list --json
```

Count saved searches:

```bash
dam ss list --json | jq 'length'
```

### SEE ALSO

[saved-search save](#dam-saved-search-save) -- create or update a saved search.
[saved-search run](#dam-saved-search-run) -- execute a saved search.

---

## dam saved-search run

### NAME

dam-saved-search-run -- execute a saved search and display results

### SYNOPSIS

```
dam [GLOBAL FLAGS] saved-search run <NAME> [--format <FMT>]
```

Alias: `dam ss run`

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
dam saved-search run "Best Landscapes"
```

Run and get just IDs for piping:

```bash
dam ss run "Recent Videos" --format ids
```

Run a saved search and add results to a collection:

```bash
dam ss run "Best Landscapes" --format ids | xargs dam col add "Portfolio"
```

Run with JSON output:

```bash
dam ss run "Canon Portraits" --format json | jq '.[].id'
```

### SEE ALSO

[saved-search save](#dam-saved-search-save) -- create or update a saved search.
[collection add](03-organize-commands.md#dam-collection-add) -- add search results to a collection.

---

## dam saved-search delete

### NAME

dam-saved-search-delete -- delete a saved search

### SYNOPSIS

```
dam [GLOBAL FLAGS] saved-search delete <NAME>
```

Alias: `dam ss delete`

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
dam saved-search delete "Old Query"
```

Delete using the alias:

```bash
dam ss delete "Temporary Search"
```

Delete with JSON confirmation:

```bash
dam ss delete "Drafts" --json
```

### SEE ALSO

[saved-search save](#dam-saved-search-save) -- create a new saved search.
[saved-search list](#dam-saved-search-list) -- list all saved searches.

---

## dam stack create

### NAME

dam-stack-create -- create a new stack from the given assets

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack create <ASSET_IDS...>
```

Alias: `dam st create`

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
dam stack create a1b2c3d4-... e5f67890-... f1a2b3c4-...
```

Create using the alias with JSON output:

```bash
dam st create a1b2c3d4-... e5f67890-... --json
```

### SEE ALSO

[stack add](#dam-stack-add) -- add assets to an existing stack.
[stack pick](#dam-stack-pick) -- change which asset is the pick.
[stack dissolve](#dam-stack-dissolve) -- dissolve a stack entirely.

---

## dam stack add

### NAME

dam-stack-add -- add assets to an existing stack

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack add <REFERENCE> <ASSET_IDS...>
```

Alias: `dam st add`

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
dam stack add a1b2c3d4-... new-asset-id-...
```

Add multiple assets:

```bash
dam st add a1b2c3d4-... b2c3d4e5-... c3d4e5f6-...
```

### SEE ALSO

[stack create](#dam-stack-create) -- create a new stack.
[stack remove](#dam-stack-remove) -- remove assets from a stack.

---

## dam stack remove

### NAME

dam-stack-remove -- remove assets from their stack

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack remove <ASSET_IDS...>
```

Alias: `dam st remove`

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
dam stack remove a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Remove multiple assets:

```bash
dam st remove a1b2c3d4-... e5f67890-...
```

### SEE ALSO

[stack add](#dam-stack-add) -- add assets to a stack.
[stack dissolve](#dam-stack-dissolve) -- dissolve an entire stack at once.

---

## dam stack pick

### NAME

dam-stack-pick -- set the pick (top) of a stack

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack pick <ASSET_ID>
```

Alias: `dam st pick`

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
dam stack pick a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Set pick using the alias:

```bash
dam st pick a1b2c3d4-...
```

### SEE ALSO

[stack show](#dam-stack-show) -- view current stack members and pick.
[stack create](#dam-stack-create) -- the first asset in create becomes the initial pick.

---

## dam stack dissolve

### NAME

dam-stack-dissolve -- dissolve an entire stack

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack dissolve <ASSET_ID>
```

Alias: `dam st dissolve`

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
dam stack dissolve a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Dissolve using the alias:

```bash
dam st dissolve a1b2c3d4-...
```

### SEE ALSO

[stack remove](#dam-stack-remove) -- remove individual assets instead of dissolving the whole stack.
[stack list](#dam-stack-list) -- list all stacks.

---

## dam stack list

### NAME

dam-stack-list -- list all stacks

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack list
```

Alias: `dam st list`

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
dam stack list
```

List as JSON and count stacks:

```bash
dam st list --json | jq 'length'
```

### SEE ALSO

[stack show](#dam-stack-show) -- view members of a specific stack.
[stack create](#dam-stack-create) -- create a new stack.

---

## dam stack show

### NAME

dam-stack-show -- show members of a stack

### SYNOPSIS

```
dam [GLOBAL FLAGS] stack show <ASSET_ID> [--format <FMT>]
```

Alias: `dam st show`

### DESCRIPTION

Displays the ordered member list of the stack containing the given asset. The pick (position 0) is shown first. Output format can be customized using the same format presets and template syntax as `dam search`.

### ARGUMENTS

**ASSET_ID** (required)
: Any asset ID that belongs to a stack.

### OPTIONS

**--format \<FMT\>**
: Output format. Presets: `ids`, `short` (default), `full`, `json`. Custom templates use `{placeholder}` syntax.

### EXAMPLES

Show members of a stack:

```bash
dam stack show a1b2c3d4-...
```

Get just the member IDs:

```bash
dam st show a1b2c3d4-... --format ids
```

Show as JSON:

```bash
dam st show a1b2c3d4-... --json
```

### SEE ALSO

[stack pick](#dam-stack-pick) -- change the pick.
[stack list](#dam-stack-list) -- list all stacks.
[search](04-retrieve-commands.md#dam-search) -- `stacked:true` filter finds all stacked assets.

---

---

## dam faces detect

### NAME

dam-faces-detect -- detect faces in asset images

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces detect [--query <Q>] [--asset <id>] [--volume <label>] [--apply]
```

### DESCRIPTION

Detects faces in asset preview images using the YuNet ONNX model. For each detected face, computes a 512-dimensional ArcFace embedding and generates a 150×150 JPEG crop thumbnail. Requires the `ai` feature (`cargo build --features ai`).

Without `--apply`, runs in report-only mode showing how many faces would be detected. With `--apply`, stores face records in the catalog and generates crop thumbnails.

Requires at least one scope filter (`--query`, `--asset`, or `--volume`) to prevent accidental full-catalog processing. Models must be downloaded first with `dam faces download`.

### OPTIONS

**--query \<Q\>**
: Search query to scope which assets are processed.

**--asset \<id\>**
: Process a single asset by ID.

**--volume \<label\>**
: Process assets on a specific volume.

**--apply**
: Actually store detected faces (default is dry run).

`--json`, `--log`, `--time` for output control.

### EXAMPLES

Detect faces in all images (dry run):

```bash
dam faces detect --query "type:image"
```

Detect and store faces for a single asset:

```bash
dam faces detect --asset a1b2c3d4 --apply
```

Detect faces on a specific volume with logging:

```bash
dam faces detect --volume "Photos" --apply --log
```

### SEE ALSO

[faces cluster](#dam-faces-cluster) -- group detected faces into people.
[faces download](#dam-faces-download) -- download required models.

---

## dam faces cluster

### NAME

dam-faces-cluster -- group similar faces into people

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces cluster [--query <Q>] [--asset <id>] [--volume <label>] [--threshold <F>] [--apply]
```

### DESCRIPTION

Groups similar face embeddings into unnamed person groups using greedy single-linkage clustering. Faces that have already been assigned to a person are skipped.

The threshold controls how similar two faces must be to be grouped together (0.0–1.0, higher = stricter). Default is 0.5, configurable via `[ai] face_cluster_threshold` in `dam.toml`.

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
dam faces cluster
```

Apply clustering with a stricter threshold:

```bash
dam faces cluster --threshold 0.6 --apply
```

Cluster only faces from a specific shoot:

```bash
dam faces cluster --query "path:Capture/2026-03" --apply
```

### SEE ALSO

[faces detect](#dam-faces-detect) -- detect faces first.
[faces name](#dam-faces-name) -- name the resulting person groups.

---

## dam faces people

### NAME

dam-faces-people -- list all people

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces people
```

### DESCRIPTION

Lists all people in the catalog with their names (if assigned) and face counts.

### EXAMPLES

```bash
dam faces people
dam faces people --json
```

### SEE ALSO

[faces name](#dam-faces-name) -- name a person.

---

## dam faces name

### NAME

dam-faces-name -- name a person

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces name <PERSON_ID> <NAME>
```

### DESCRIPTION

Assigns a human-readable name to a person. Person IDs are shown by `dam faces people`.

### ARGUMENTS

**PERSON_ID** (required)
: The person's UUID.

**NAME** (required)
: The name to assign.

### EXAMPLES

```bash
dam faces name 550e8400-... "Alice"
```

---

## dam faces merge

### NAME

dam-faces-merge -- merge two people

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces merge <TARGET_ID> <SOURCE_ID>
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
dam faces merge 550e8400-... 661f9511-...
```

---

## dam faces delete-person

### NAME

dam-faces-delete-person -- delete a person

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces delete-person <PERSON_ID>
```

### DESCRIPTION

Deletes a person record. All faces assigned to this person become unassigned (they are not deleted).

### ARGUMENTS

**PERSON_ID** (required)
: The person's UUID.

### EXAMPLES

```bash
dam faces delete-person 550e8400-...
```

---

## dam faces unassign

### NAME

dam-faces-unassign -- remove a face from its person

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces unassign <FACE_ID>
```

### DESCRIPTION

Removes the person assignment from a single face. The face record is preserved; only the person link is cleared.

### ARGUMENTS

**FACE_ID** (required)
: The face's UUID.

### EXAMPLES

```bash
dam faces unassign a1b2c3d4-...
```

---

## dam faces export

### NAME

dam-faces-export -- export faces and people to YAML and binary files

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces export
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
dam faces export
```

Export with JSON output:

```bash
dam faces export --json
```

### SEE ALSO

[faces detect](#dam-faces-detect) -- detect faces in images.
[rebuild-catalog](05-maintain-commands.md#dam-rebuild-catalog) -- rebuilds catalog from files.

---

## dam faces download

### NAME

dam-faces-download -- download face detection models

### SYNOPSIS

```
dam [GLOBAL FLAGS] faces download
```

### DESCRIPTION

Downloads the YuNet face detection model and ArcFace face recognition model from HuggingFace. Models are cached in the model directory (default `~/.cache/dam/models`, configurable via `[ai] model_dir` in `dam.toml`).

### EXAMPLES

```bash
dam faces download
```

---

Previous: [Ingest Commands](02-ingest-commands.md) -- `import`, `tag`, `edit`, `group`, `auto-group`.
Next: [Retrieve Commands](04-retrieve-commands.md) -- `search`, `show`, `duplicates`, `stats`, `serve`.
