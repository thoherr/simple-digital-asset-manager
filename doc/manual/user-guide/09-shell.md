# Interactive Shell

dam includes an interactive shell that keeps catalog state alive between commands, giving near-instant response times and enabling multi-step workflows with named variables, session defaults, tab completion, and script files. Instead of paying the startup cost of locating the catalog, loading configuration, and opening SQLite on every invocation, the shell does this once and reuses it for the entire session.

This chapter covers interactive use, variables, scripting, and practical workflows. For one-shot CLI scripting with bash or Python, see [Scripting](08-scripting.md).

---

## Starting the Shell

### Interactive mode

Launch the shell from within a catalog directory:

```bash
dam shell
```

Output:

```
dam shell v3.2.2 — type 'help' or 'quit' to exit
photos>
```

The prompt shows the catalog name (the directory basename). Type any dam command without the `dam` prefix.

### Run a script file

Execute a `.dam` script file and exit:

```bash
dam shell post-import.dam
```

### Run a single command

Run one command and exit, useful in shell scripts or aliases:

```bash
dam shell -c 'search "rating:5 tag:landscape"'
```

The `-c` mode shares the same parsing and variable support as interactive mode, but exits after processing the command string.

---

## Basic Usage

Inside the shell, type any dam subcommand directly. There is no need to type `dam` before each command:

```
photos> stats
photos> search "tag:portrait rating:4+"
  42 assets found
photos> show abc12345
photos> edit --rating 5 abc12345
```

### Blank lines and comments

Blank lines are silently skipped. Lines starting with `#` are treated as comments:

```
photos> # This is a comment
photos>
photos> stats
```

### History

The shell saves command history to `.dam/shell_history` inside your catalog directory. Use the up/down arrow keys to recall previous commands. History persists across sessions.

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| Up/Down | Navigate command history |
| Ctrl-C | Cancel the current line (does not exit) |
| Ctrl-D | Exit the shell (same as `quit`) |
| Ctrl-A | Move cursor to start of line |
| Ctrl-E | Move cursor to end of line |
| Ctrl-K | Delete from cursor to end of line |
| Ctrl-W | Delete word before cursor |
| Tab | Trigger tab completion |

---

## Variables

The shell supports named variables that hold lists of asset IDs. Variables let you capture command results and reuse them in subsequent commands without re-running queries.

### The implicit `_` variable

Every command that produces asset IDs (such as `search`, `duplicates`, or `show`) automatically stores its results in the implicit `_` variable. Use `_` in the next command to refer to those results:

```
photos> search "tag:landscape rating:4+"
  12 assets found
photos> edit --rating 5 _
  12 assets edited
photos> tag --add "portfolio" _
  12 assets tagged
```

The `_` variable is updated after every command that returns assets. It only expands when it appears as a standalone token -- underscores inside words like `my_tag` or `_foo` are left alone.

### Named variables

Store results in a named variable with the `$name = command` syntax:

```
photos> $picks = search "rating:5 date:2024"
  38 assets --> $picks
photos> $untagged = search "tags:none type:image"
  142 assets --> $untagged
```

Use variables in any command by referencing `$name`:

```
photos> tag --add "needs-review" $untagged
  142 assets tagged
photos> export $picks /tmp/best
  38 assets exported
```

You can also copy the last result into a named variable:

```
photos> search "rating:5"
  38 assets found
photos> $best = _
  38 assets --> $best
```

### Variable rules

- Names start with `$`, followed by letters, digits, or underscores (`$picks`, `$session_2024`, `$batch1`)
- Variables hold asset ID lists -- they are not re-evaluated queries
- Variables persist for the session and are cleared on `reload`
- Expanding an undefined variable leaves it as-is in the command
- **Position does not matter.** When a variable expands, its asset IDs are always placed at the end of the argument list (the trailing positional slot where commands expect asset IDs). This means `tag --add portfolio $picks` and `tag $picks --add portfolio` produce the same result. You never need to worry about where you place the variable in the command.

### Listing and removing variables

Use `vars` to see all defined variables and their counts:

```
photos> vars
  Session defaults: --log
  _ = 38 assets
  $picks = 38 assets
  $untagged = 142 assets
```

Remove a variable with `unset`:

```
photos> unset $untagged
  Removed $untagged
```

---

## Session Defaults

Use `set` to add flags that are automatically injected into every subsequent command. This is useful when you want `--log` or `--json` output for an entire session without typing it each time.

### Setting defaults

```
photos> set --log
  Session default: --log
  Active defaults: --log
photos> set --json
  Session default: --json
  Active defaults: --json --log
```

The settable flags are: `--json`, `--log`, `--debug`, and `--time`.

After setting `--log`, every command will include progress output automatically:

```
photos> set --log
photos> generate-previews "rating:5"
  [1/38] abc12345 -- generated preview
  [2/38] def67890 -- generated preview
  ...
```

### Removing defaults

```
photos> unset --log
  Removed session default: --log
```

Session defaults are also cleared by `reload`.

**Tip:** If a session default conflicts with what a particular command needs, you do not need to unset it first. The shell will not inject a flag that already appears in the command line.

---

## Tab Completion

The shell provides context-aware tab completion powered by rustyline. Press Tab at any point to see suggestions.

### What gets completed

| Context | Completes | Example |
|---------|-----------|---------|
| First word | Subcommand names and built-in commands | `sea` -> `search` |
| `--` prefix | Common flags | `--j` -> `--json` |
| `$` prefix | Defined variable names | `$pi` -> `$picks` |
| `tag:` prefix | Tag names from the catalog | `tag:land` -> `tag:landscape` |
| `volume:` prefix | Volume labels from the catalog | `volume:Ph` -> `volume:Photos` |
| After `--volume` | Volume labels | `--volume Ph` -> `--volume Photos` |

Volume labels containing spaces are automatically quoted in completions:

```
photos> search volume:<Tab>
volume:Photos  volume:"External SSD"  volume:Archive
```

Completion data is loaded from the catalog when the shell starts. Use `reload` to refresh it after adding new tags or volumes.

---

## Script Files

A `.dam` script file is a sequence of commands, one per line. Scripts support the same syntax as interactive mode: variables, comments, blank lines, and `source`.

### Basic script format

```bash
# post-import.dam -- run after each shoot day
import /Volumes/Cards/DCIM --log
auto-group
generate-previews --log
auto-tag --apply --log
describe --query "description:none" --apply --log
stats
```

Execute it:

```bash
dam shell post-import.dam
```

### Variables in scripts

Variables work across lines, making multi-step workflows clean:

```bash
# curate-and-export.dam
$new = search "imported:today type:image"
auto-group $new
generate-previews $new --log
$picks = search "imported:today rating:4+"
tag --add "selects" $picks
export $picks /tmp/selects
stats
```

### The `--strict` flag

By default, scripts continue after errors. Use `--strict` to stop on the first failure:

```bash
dam shell --strict critical-workflow.dam
```

In `--strict` mode, the shell exits with code 1 on any error. Without `--strict`, errors are printed with file and line number but execution continues:

```
critical-workflow.dam:3: Error: volume "Offline" is not mounted
```

### The `source` command

Use `source` inside the shell (or another script) to run a script file inline, sharing the current session state:

```
photos> source post-import.dam
```

Variables defined in the sourced script are available after it finishes. Paths are resolved relative to the catalog root directory, or you can use absolute paths:

```
photos> source /path/to/shared/cleanup.dam
```

---

## Session Management

### `reload`

Re-reads `dam.toml` configuration, refreshes tab completion data (tags and volumes), and clears all variables and session defaults:

```
photos> reload
  Reloaded config, cleared variables and session defaults.
```

Use this after making external changes -- editing `dam.toml`, importing from another terminal, or adding volumes.

### `help`

Prints a quick reference of shell syntax, variable usage, session defaults, and built-in commands:

```
photos> help
```

### `quit` / `exit`

End the session. Ctrl-D also exits. Command history is saved automatically on exit.

---

## Quote Handling

The shell uses smart quote handling that lets search filter syntax pass through naturally. The rule is simple:

- **Quotes at the start of a token** are grouping quotes -- they are stripped, and their content is treated as a single argument (standard shell behavior).
- **Quotes that appear mid-token** are preserved as part of the value (syntax quotes).

This means search filters with quoted values work without extra escaping:

```
photos> search text:"woman with glasses" tag:portrait
```

The shell splits this into two tokens: `text:"woman with glasses"` and `tag:portrait`. The quotes around `woman with glasses` are preserved because they start mid-token (after `text:`), so the search engine receives them intact.

Compare with grouping quotes, which wrap the entire token from the start:

```
photos> search "tag:landscape rating:4+"
```

Here the outer quotes are stripped (they start at the beginning of the token), producing two arguments: `tag:landscape` and `rating:4+`. This is equivalent to typing them without quotes.

You can mix both styles:

```
photos> search "type:image" camera:"NIKON Z 9" rating:4+
```

This produces three tokens: `type:image` (grouping quotes stripped), `camera:"NIKON Z 9"` (mid-token quotes preserved), and `rating:4+`.

**Tip:** If you are used to the standard CLI where you type `dam search 'text:"query"'`, inside the shell you can drop the outer quotes entirely and just type `search text:"query"`.

---

## Tilde Expansion

The shell expands `~` to your home directory (`$HOME` on Unix, `%USERPROFILE%` on Windows) in any token:

```
photos> export $picks ~/Desktop/delivery
  38 assets exported to /Users/alice/Desktop/delivery
photos> export "rating:5" ~/exports/portfolio --zip
  Export complete: 12 files archived
```

Only `~` at the start of a token is expanded — `~` appearing elsewhere (e.g. inside a search query) is left as-is.

---

## Built-in Commands

Most commands in the shell are dam subcommands run directly. A few commands are **built-ins** handled specially by the shell:

| Command | Purpose |
|---------|---------|
| `export` | Export assets to a directory or ZIP archive (supports `$var` expansion for multi-ID export) |
| `preview` | Show asset previews in the terminal (supports `$var` and `_`) |
| `help` | Shell help and syntax reference |
| `reload` | Re-read config, clear variables and defaults |
| `set` / `unset` | Manage session defaults and variables |
| `source` | Run a script file inline |
| `vars` | List defined variables |
| `quit` / `exit` | End the session |

### Shell `export`

The `export` command is a built-in that supports variable expansion and all standard export flags:

```
photos> export <query|$var> <target> [--layout flat|mirror] [--all-variants]
        [--include-sidecars] [--dry-run] [--overwrite] [--symlink] [--zip]
```

When a variable expands to multiple asset IDs, all assets are exported in a single operation:

```
photos> $picks = search "rating:5 tag:landscape"
  24 assets --> $picks
photos> export $picks ~/Desktop/landscapes
  24 assets exported
photos> export $picks ~/Desktop/landscapes.zip --zip
  Export complete: 24 files archived
```

---

## Practical Examples

### Curate and export workflow

Browse a recent shoot, refine selections, and export:

```
photos> $candidates = search "date:2024-06 type:image rating:3+"
  287 assets --> $candidates
photos> $portraits = search "date:2024-06 tag:portrait rating:4+"
  42 assets --> $portraits
photos> tag --add "june-selects" $portraits
photos> export $portraits /tmp/june-portraits
  42 assets exported
```

### Post-import script

Save this as `post-import.dam` and run after every import:

```bash
# post-import.dam
$new = search "imported:today"
auto-group $new
generate-previews $new --log
auto-tag --apply --query "tags:none type:image" --log
describe --query "description:none" --apply --log
stats
```

Run it:

```bash
dam shell post-import.dam
```

Or source it during an interactive session:

```
photos> import /Volumes/Cards/DCIM --log
photos> source post-import.dam
```

### Audit workflow

Investigate and fix grouping issues:

```
photos> set --log
photos> $scattered = search "scattered:2 variants:3+"
  15 assets --> $scattered
photos> show $scattered
  ... review each asset ...
photos> split abc12345 --variants def678,ghi901
photos> auto-group $scattered
```

### Batch tagging with variables

Tag assets from multiple searches without re-querying:

```
photos> $landscapes = search "tag:landscape rating:4+"
  85 assets --> $landscapes
photos> $portraits = search "tag:portrait rating:4+"
  42 assets --> $portraits
photos> tag --add "portfolio" $landscapes
photos> tag --add "portfolio" $portraits
photos> $portfolio = search "tag:portfolio"
  127 assets --> $portfolio
photos> export $portfolio /tmp/portfolio
```

### Quick one-liners with `-c`

Use `-c` mode for cron jobs or shell aliases:

```bash
# Daily stats check
dam shell -c 'stats'

# Export today's picks
dam shell -c '$picks = search "imported:today rating:4+"; export $picks /tmp/daily'

# Verify with logging
dam shell -c 'set --log; verify'
```

---

## Blocked Commands

Four commands are blocked inside the shell because they conflict with the shell session:

| Command | Reason |
|---------|--------|
| `init` | Creates a new catalog; the shell is already inside one |
| `migrate` | Schema migration should be run standalone |
| `serve` | Starts a long-running web server that conflicts with the shell loop |
| `shell` | No nesting; use `source` to run script files instead |

Attempting to run a blocked command prints a clear message:

```
photos> serve
'serve' cannot be used inside the shell.
```

---

## Tips

**The shell prompt tells you what is in memory.** When named variables are defined, the prompt shows their counts in brackets:

```
photos [picks=38 untagged=142]> tag --add "review" $untagged
```

**Variables are snapshots, not live queries.** If you run `$picks = search "rating:5"` and then rate more assets, `$picks` still holds the original results. Run the assignment again to refresh.

**Use `set --time` to profile commands.** This is handy when exploring performance differences between queries during an interactive session.

**Scripts are composable.** A master script can `source` other scripts, building complex workflows from reusable pieces:

```bash
# nightly.dam
source post-import.dam
source cleanup.dam
source verify.dam
```

**Ctrl-C is safe.** It cancels the current input line but does not exit the shell or lose your variables. Use Ctrl-D or `quit` to exit.

---

## Related Topics

- [Scripting](08-scripting.md) -- bash and Python scripting with `dam --json` and `jq`
- [Browsing & Searching](05-browse-and-search.md) -- search syntax and output format options
- [CLI Conventions](../reference/00-cli-conventions.md) -- global flags, exit codes, and output conventions
- [Search Filters Reference](../reference/06-search-filters.md) -- all available search filters
- [Configuration Reference](../reference/08-configuration.md) -- `dam.toml` settings including `[import]` and `[serve]` sections
