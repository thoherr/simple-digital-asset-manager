# Idea Notebook: Musings about possible future features

This is a unstructured collection of ideas that came to mind while working on and with my dam system.

## UX / UI

### Text field with chips

Instead of the current, single selection drop down in the upper search area, we could have a text field with (one or
more) chips entry (selectable like tags), which should behave as logical or for the query
for file formats (maybe also types, volumes and collections; check usability and consistency).

### Complex queries

Currently we only have simple queries, and all of them are anded in the query.
Some sort of logical query language could be useful (e.g. "find all vacation images, but not from france"
or "give me all images with tags "alice" or "bob").

## CLI

### Scripting example

Since the CLI can provide information in machine readable JSON format, one or more small example scripts
(e.g. in Python, Ruby or even as Bash-Script) for how to use it may be helpful. Maybe something like "find all paths
with pictures of Alices birthday and copy the two highest rated images with Bob to a sd card" (where the names would
of cource have been tagged). Or maybe we come up with another, more useful example.

