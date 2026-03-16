# Proposal: Asset Management Shell

**Status: Fully implemented in v3.0.0.** All three phases complete — basic shell with `_` and scripts (Phase 1), named variables and tab completion (Phase 2), session defaults, `source`, `reload`, `-c` mode, and `--strict` (Phase 3). See [User Guide](../../manual/user-guide/09-shell.md) and [Command Reference](../../manual/reference/04-retrieve-commands.md#maki-shell).

## Motivation

Every `maki` invocation repeats the same startup work: locate catalog root, load `maki.toml`, open SQLite with pragmas, check schema version, construct services. For interactive workflows — browsing results, editing tags, checking stats — this overhead adds up.

Beyond performance, one-shot CLI invocations can't compose naturally. A shell workflow like `maki search "rating:5" | xargs -I{} maki export {} /tmp/picks` works but is verbose, re-opens the catalog for each asset, and loses type information (IDs become plain strings).

An asset management shell keeps state alive between commands, gives instant response, and introduces asset-typed variables and script files — turning `maki` from a command-line tool into a composable workflow environment.

## Design

### Entry Point

```
maki shell                         # interactive session
maki shell workflow.maki            # run a script file
maki shell -c 'search "rating:5"' # run a single command
```

Starts an interactive session in the current catalog. Displays a prompt, accepts any `maki` subcommand without the `maki` prefix:

```
maki> search "tag:landscape rating:4+"
  12 assets found
maki> edit --rating 5 abc12345
maki> stats
maki> quit
```

### Cached State

Modeled after the existing `AppState` from `maki serve`:

| State | Lifecycle | Notes |
|-------|-----------|-------|
| `catalog_root` | Session | Found once at startup |
| `CatalogConfig` | Session | Reloaded on explicit `reload` command |
| `DeviceRegistry` | Session, invalidated on volume mutations | Volume add/remove/combine refreshes |
| Preview/AI/VLM config | Session | Derived from `CatalogConfig` |
| Catalog (SQLite) | Per-command | Fresh `Catalog::open_fast()` each command, same as web server |
| Variables | Session | Named result sets, cleared on `reload` |

Per-command catalog opens are cheap (~1ms with pragmas) and avoid stale-connection issues. This matches the proven `maki serve` pattern.

### Command Parsing

Clap supports `try_parse_from(args)` which takes an iterator of strings. The shell loop:

1. Read a line from the user (via rustyline)
2. Check for shell-specific syntax (variable assignment, `#` comments, blank lines)
3. Expand variables (`$name` and `_`) to asset ID lists
4. Shell-split into tokens (handle quotes, escapes)
5. Prepend `"maki"` as argv[0]
6. Call `Cli::try_parse_from(tokens)`
7. Execute the command using the cached state
8. Capture result set (if applicable), update `_`
9. Print result, loop

Parse errors are displayed without exiting. Commands like `init`, `migrate`, and `serve` are rejected inside the shell (they don't make sense in an interactive session).

### Result Sets and Variables

Commands that return asset lists (`search`, `duplicates`, `show`) capture their results as the implicit `_` variable. Named variables store result sets for later use:

```
maki> $picks = search "rating:5 date:2024"
  38 assets → $picks
maki> $untagged = search "tags:none type:image"
  142 assets → $untagged
maki> tag --add "needs-review" $untagged
  142 assets tagged
maki> export --target /tmp/best $picks
  38 assets exported
```

Variables hold asset ID lists. They expand to space-separated IDs when referenced. The implicit `_` always holds the result of the last command that produced asset IDs:

```
maki> search "tag:landscape rating:4+"
  12 assets found
maki> edit --rating 5 _
  12 assets edited
maki> tag --add "portfolio" _
  12 assets tagged
```

Variable rules:
- Names start with `$`, followed by letters/digits/underscores
- `_` is the implicit last-result variable (updated after every command that returns assets)
- Variables persist for the session (cleared on `reload` or `unset $name`)
- `vars` command lists all defined variables with their asset counts
- Variables are plain asset ID lists — no query re-execution, no staleness

### Script Files

A `.maki` script file is a sequence of commands, one per line:

```bash
# nightly-import.maki — run after each shoot day
import /Volumes/Cards/DCIM --log
auto-group
generate-previews --log
auto-tag --apply --log
describe --query "description:none" --apply --log
stats
```

Execute with:

```
maki shell nightly-import.maki
maki shell -c 'search "rating:5"'   # one-liner
```

Script features:
- `#` comments and blank lines are ignored
- Variables work across lines (assign in one, use in another)
- Exit code: 0 if all commands succeed, 1 on first error (with `--strict`), or continue-on-error by default
- Scripts can be composed: `source other-script.maki` runs another script inline, sharing the session state

### Shell-Only Commands

| Command | Description |
|---------|-------------|
| `quit` / `exit` / Ctrl-D | End the session |
| `reload` | Re-read `maki.toml`, refresh cached config, clear variables |
| `help` | Show available commands (delegates to clap `--help`) |
| `set <flag>` | Session-wide defaults: `set --log`, `set --debug`, `set --json` |
| `unset <flag>` | Remove a session default |
| `vars` | List defined variables with asset counts |
| `unset $name` | Remove a variable |
| `source <file>` | Execute a script file in the current session |

### Readline Features

Using `rustyline` crate (~4K lines, stable, MIT):

- **Command history** — persisted to `.maki/shell_history` in the catalog directory
- **Tab completion** — subcommand names, `--flags`, volume labels, tag names, variable names
- **Line editing** — Emacs-style keybindings (Ctrl-A/E/K/W), Vi mode optional
- **Multi-line** — not needed; commands are single-line

### Prompt

Shows the catalog name (directory basename) and variable context:

```
photos> search "rating:5"
  38 assets found
photos> $best = _
photos [best=38]> tag --add "portfolio" $best
```

The bracket section appears when named variables are defined, showing their counts.

### Output Handling

- Normal text output goes to stdout as usual
- `--json` works per-command (e.g., `search --json "tag:landscape"`)
- Session defaults via `set` apply to all subsequent commands
- `--time` works per-command
- In script mode (`maki shell script.maki`), output goes to stdout/stderr normally — scripts are pipeable

### Excluded Commands

These are blocked inside the shell with a clear message:

- `init` — creates a new catalog; meaningless inside an existing one
- `migrate` — schema migration should be run standalone
- `serve` — starts a long-running web server; conflicts with the shell's own loop
- `shell` — no nesting (but `source` runs script files)

## Example Workflows

### Curate and Export

```
maki> $candidates = search "date:2024-06 type:image rating:3+"
  287 assets → $candidates
maki> $portraits = search "date:2024-06 tag:portrait rating:4+"
  42 assets → $portraits
maki> tag --add "june-selects" $portraits
maki> export --target /tmp/june-portraits $portraits
  42 assets exported
```

### Cleanup After Import

```bash
# post-import.maki
$new = search "imported:today"
auto-group $new
generate-previews $new --log
describe --query "description:none" --apply --log
auto-tag --apply --query "tags:none type:image" --log
stats
```

### Audit Mis-grouped Assets

```
maki> $scattered = search "scattered:2 variants:3+"
  15 assets → $scattered
maki> show $scattered
  ... review each asset ...
maki> split abc12345 --variants def678,ghi901
maki> reimport-metadata abc12345
```

## Implementation

### Phase 1 — Basic Shell

- Add `rustyline` dependency
- New `Commands::Shell` variant with optional script file and `-c` flag
- Shell loop: readline -> parse -> dispatch -> print
- Cache `catalog_root` and `CatalogConfig`
- Command history (in-memory)
- Block excluded commands
- `_` implicit result variable
- Script file execution (sequential, no variables yet)

### Phase 2 — Variables & Completion

- Named variables (`$name = command`)
- Variable expansion in command arguments
- `vars`, `unset` commands
- Persist history to `.maki/shell_history`
- Tab completion for subcommand names, flags, variable names
- Tab completion for volume labels and tag names (from catalog queries, cached)

### Phase 3 — Session Management & Scripts

- `set` / `unset` for session-wide defaults
- `source` command for inline script execution
- `reload` command
- Prompt customization with catalog name and variable context
- `--strict` flag for scripts (exit on first error)
- `-c` single-command mode

## Dependencies

| Crate | Purpose | Size |
|-------|---------|------|
| `rustyline` | Line editing, history, completion | ~4K lines, stable |

Alternative: `reedline` (Nushell's editor) — better Unicode, heavier (~30K lines). `rustyline` is the pragmatic choice.

## Complexity

**Phase 1:** Low. The command dispatcher already exists as a single `match` block in `main.rs`. Wrapping it in a loop with `try_parse_from` is mechanical. Script execution is just reading lines from a file instead of stdin.

**Phase 2:** Low-Medium. Variable expansion is string substitution before parsing. Rustyline's `Completer` trait is straightforward; the hard part is keeping completion data fresh after mutations.

**Phase 3:** Medium. Session defaults require threading state through the dispatcher. `source` is recursive script execution sharing the session.

## Trade-offs

**Pros:**
- Near-instant command execution after first startup
- Named result sets enable multi-step workflows without shell piping gymnastics
- Script files make repeatable workflows trivial (`post-import.maki`, `nightly-cleanup.maki`)
- Command history and tab completion for efficient interactive use
- Natural fit for exploratory sessions (search, inspect, edit, repeat)
- Mirrors the `sqlite3` / `psql` / `python` interactive experience

**Cons:**
- New dependency (rustyline)
- Commands must be careful about process-level side effects (changing working directory, signal handlers)
- Catalog changes from external `maki` invocations won't be visible without `reload`
- Variable syntax adds a small learning curve (though `$name` is universally familiar)

## Not In Scope

- **Control flow** (if/else, for/while loops) — users who need this can use bash + `maki --json` + `jq`, or Python. Adding control flow means building a language, which is a different project entirely.
- **TUI / ncurses interface** — the shell is text-based, not a full terminal UI
- **Concurrent command execution** — commands run sequentially
- **Remote shell / network protocol** — local only
- **Expression evaluation** — variables hold asset ID lists, not computed values
