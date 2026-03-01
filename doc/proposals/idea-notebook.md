# Idea Notebook: Musings about possible future features

This is a unstructured collection of ideas that came to mind while working on and with my dam system.

## Data storage

### Metadata of standalone JPEGs (esp.date, probably others) should be editable

The metadata of JPEGs should also be updatable. But for "originals", i.e. imported JPEGs like old slides scans etc.,
we do not want to loose our asset id, so we never write back to the JPEG, and normally we do not have a xmp sidecar
(this should work already). Have to chack how capture one e.g. handles this. But the date field has to be updateable,
probable this information would then only go to our own yaml sidecar file (which is our source of truth)?

### XMP sync / resolution for multi volume duplicates

Maybe we could come up with ideas how to synchronize recipe files across volumes. this is especially critical for
the use case we have a ssd work hard drive (volume role working) vs. the storage drive (role archive). A merge of the
data is probably ways to complicated, maybe a "last one wins" approach is sufficient?

## UX / UI

### Text field with chips

Instead of the current, single selection drop down in the upper search area, we could have a text field with (one or
more) chips entry (selectable like tags), which should behave as logical or for the query
for file formats (maybe also types, volumes and collections; check usability and consistency).

### Complex queries

Currently we only have simple queries, and all of them are anded in the query.
Some sort of logical query language could be useful (e.g. "find all vacation images, but not from france"
or "give me all images with tags "alice" or "bob").

### Asset folder link

The details page should contain a link to the asset and/or asset folder at the locations. maybe just make the location
clickable, if the volume is online (folder part opens folder of asset, asset file name opens asset directly).
Optional/alternative open terminal window at location.

## CLI

### Scripting example

Since the CLI can provide information in machine readable JSON format, one or more small example scripts
(e.g. in Python, Ruby or even as Bash-Script) for how to use it may be helpful. Maybe something like "find all paths
with pictures of Alices birthday and copy the two highest rated images with Bob to a sd card" (where the names would
of cource have been tagged). Or maybe we come up with another, more useful example.

