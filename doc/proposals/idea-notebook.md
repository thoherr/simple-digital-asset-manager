# Idea Notebook: Musings about possible future features

This is a unstructured collection of ideas that came to mind while working on and with my dam system.

## Data storage

### XMP sync / resolution for multi volume duplicates

Maybe we could come up with ideas how to synchronize recipe files across volumes. this is especially critical for
the use case we have a ssd work hard drive (volume role working) vs. the storage drive (role archive). A merge of the
data is probably ways to complicated, maybe a "last one wins" approach is sufficient?

## UX / UI

### Text field with chips

Instead of the current, single selection drop down in the upper search area, we could have a text field with (one or
more) chips entry (selectable like tags), which should behave as logical or for the query
for file formats (maybe also types, volumes and collections; check usability and consistency).

### ~~Complex queries~~ **DONE** (v1.8.6)

Currently we only have simple queries, and all of them are anded in the query.
Some sort of logical query language could be useful (e.g. "find all vacation images, but not from france"
or "give me all images with tags "alice" or "bob").

### ~~Asset folder link~~ — **DONE** (v1.8.2)

Implemented as reveal-in-file-manager (📂) and open-terminal (`>_`) buttons on the asset detail page, next to each file location on online volumes. Supports macOS (Finder/Terminal.app), Linux (xdg-open/terminal emulators), and Windows (Explorer/cmd). Backed by `POST /api/open-location` and `POST /api/open-terminal` endpoints.

## CLI

### ~~Scripting example~~ - **DONE** (v1.8.5)

Since the CLI can provide information in machine readable JSON format, one or more small example scripts
(e.g. in Python, Ruby or even as Bash-Script) for how to use it may be helpful. Maybe something like "find all paths
with pictures of Alices birthday and copy the two highest rated images with Bob to a sd card" (where the names would
of cource have been tagged). Or maybe we come up with another, more useful example.

### delete command

Would be ver useful if we have orphaned files or other import bugs. Currently this is solved by quite complex and long
running commands, whereas we often know that we just have to delete (and maybe re-import) a certain asset.

## Documentation

### ~~Cross References in the PDF manual do not work (they link to the *.md file) - BUG~~ **DONE** (v1.8.5)
 