use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::{Context, Helper, Highlighter, Hinter, Validator};

/// Result of running a single command in the shell.
pub struct CommandResult {
    /// Asset IDs produced by the command (e.g. from search).
    pub asset_ids: Vec<String>,
}

/// Options for `run()`.
pub struct RunOptions {
    /// Script file to execute (instead of interactive mode).
    pub script: Option<PathBuf>,
    /// Single command to run and exit.
    pub command: Option<String>,
    /// Exit on first error (scripts and -c mode).
    pub strict: bool,
}

/// Run the interactive shell, a script file, or a single command.
pub fn run(
    catalog_root: &Path,
    opts: RunOptions,
    executor: impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    if let Some(ref cmd) = opts.command {
        run_single_command(cmd, opts.strict, &executor);
    } else if let Some(ref path) = opts.script {
        run_script(path, opts.strict, &executor);
    } else {
        run_interactive(catalog_root, &executor);
    }
}

// ---------------------------------------------------------------------------
// Tab completion
// ---------------------------------------------------------------------------

/// Subcommand names for completion (sorted).
const SUBCOMMANDS: &[&str] = &[
    "auto-group",
    "auto-tag",
    "backup-status",
    "cleanup",
    "collection",
    "contact-sheet",
    "dedup",
    "delete",
    "describe",
    "duplicates",
    "edit",
    "embed",
    "faces",
    "fix-dates",
    "fix-recipes",
    "fix-roles",
    "generate-previews",
    "group",
    "import",
    "rebuild-catalog",
    "refresh",
    "relocate",
    "saved-search",
    "search",
    "show",
    "split",
    "stack",
    "stats",
    "sync",
    "sync-metadata",
    "tag",
    "update-location",
    "verify",
    "volume",
    "writeback",
];

/// Shell built-in commands for completion.
const BUILTINS: &[&str] = &[
    "exit", "export", "help", "preview", "quit", "reload", "set", "source", "unset", "vars",
];

/// Common flags for completion (subset — the most universally useful).
const COMMON_FLAGS: &[&str] = &[
    "--apply",
    "--debug",
    "--dry-run",
    "--force",
    "--format",
    "--ids",
    "--json",
    "--log",
    "--query",
    "--time",
    "--volume",
];

/// Shell helper that provides tab completion.
#[derive(Helper, Hinter, Highlighter, Validator)]
struct ShellHelper {
    /// Cached tag names for completion.
    tags: Vec<String>,
    /// Cached volume labels for completion.
    volumes: Vec<String>,
    /// Reference to the shared variable map (names only, for completion).
    /// Updated each time the prompt is shown.
    variable_names: Vec<String>,
}

impl ShellHelper {
    fn new(catalog_root: &Path) -> Self {
        let (tags, volumes) = load_completion_data(catalog_root);
        Self {
            tags,
            volumes,
            variable_names: Vec::new(),
        }
    }

    /// Refresh completion data from the catalog.
    fn refresh(&mut self, catalog_root: &Path) {
        let (tags, volumes) = load_completion_data(catalog_root);
        self.tags = tags;
        self.volumes = volumes;
    }
}

/// Load tag names and volume labels from the catalog for completion.
fn load_completion_data(catalog_root: &Path) -> (Vec<String>, Vec<String>) {
    let db_path = catalog_root.join(".maki").join("catalog.db");
    let mut tags: Vec<String>;
    let volumes: Vec<String>;

    if let Ok(catalog) = crate::catalog::Catalog::open_fast(&db_path) {
        let mut tag_set: std::collections::HashSet<String> = catalog
            .list_all_tags()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _count)| name)
            .collect();
        // Merge vocabulary tags
        for vt in crate::vocabulary::load_vocabulary(catalog_root) {
            tag_set.insert(vt);
        }
        tags = tag_set.into_iter().collect();
        tags.sort();
        volumes = catalog
            .list_volumes()
            .unwrap_or_default()
            .into_iter()
            .map(|(_id, label)| label)
            .collect();
    } else {
        tags = crate::vocabulary::load_vocabulary(catalog_root);
        volumes = Vec::new();
    }

    (tags, volumes)
}

impl Completer for ShellHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let before = &line[..pos];

        // Find the start of the current word
        let word_start = before.rfind(|c: char| c == ' ' || c == '\t').map_or(0, |i| i + 1);
        let word = &before[word_start..];

        // Variable completion: $...
        if word.starts_with('$') {
            let prefix = &word[1..]; // skip $
            let matches: Vec<String> = self
                .variable_names
                .iter()
                .filter(|name| name.starts_with(prefix))
                .map(|name| format!("${name}"))
                .collect();
            return Ok((word_start, matches));
        }

        // Flag completion: --...
        if word.starts_with("--") {
            let matches: Vec<String> = COMMON_FLAGS
                .iter()
                .filter(|f| f.starts_with(word))
                .map(|f| f.to_string())
                .collect();
            return Ok((word_start, matches));
        }

        // After "tag:" prefix, complete tag names
        if word.starts_with("tag:") {
            let prefix = &word[4..];
            let matches: Vec<String> = self
                .tags
                .iter()
                .filter(|t| t.starts_with(prefix))
                .map(|t| format!("tag:{t}"))
                .collect();
            return Ok((word_start, matches));
        }

        // After "--volume " or "volume:" prefix, complete volume labels
        if word.starts_with("volume:") {
            let prefix = &word[7..];
            let matches: Vec<String> = self
                .volumes
                .iter()
                .filter(|v| v.to_lowercase().starts_with(&prefix.to_lowercase()))
                .map(|v| {
                    if v.contains(' ') {
                        format!("volume:\"{v}\"")
                    } else {
                        format!("volume:{v}")
                    }
                })
                .collect();
            return Ok((word_start, matches));
        }

        // If the previous word is --volume, complete volume labels
        let words: Vec<&str> = before.split_whitespace().collect();
        if words.len() >= 2 && words[words.len() - 2] == "--volume" && !word.starts_with('-') {
            let matches: Vec<String> = self
                .volumes
                .iter()
                .filter(|v| v.to_lowercase().starts_with(&word.to_lowercase()))
                .map(|v| {
                    if v.contains(' ') {
                        format!("\"{v}\"")
                    } else {
                        v.to_string()
                    }
                })
                .collect();
            return Ok((word_start, matches));
        }

        // First word: complete subcommand or built-in
        if word_start == 0 || !before[..word_start].contains(|c: char| !c.is_whitespace()) {
            let mut matches: Vec<String> = SUBCOMMANDS
                .iter()
                .chain(BUILTINS.iter())
                .filter(|cmd| cmd.starts_with(word))
                .map(|cmd| cmd.to_string())
                .collect();
            matches.sort();
            return Ok((word_start, matches));
        }

        Ok((pos, Vec::new()))
    }
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// Session variables: named result sets mapping $name → asset IDs.
struct Variables {
    named: HashMap<String, Vec<String>>,
    last_ids: Vec<String>,
}

impl Variables {
    fn new() -> Self {
        Self {
            named: HashMap::new(),
            last_ids: Vec::new(),
        }
    }

    /// Get sorted list of variable names (for completion and prompt).
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.named.keys().cloned().collect();
        names.sort();
        names
    }

    /// Build the prompt bracket section showing variable counts.
    fn prompt_context(&self) -> String {
        if self.named.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = self
            .names()
            .iter()
            .map(|name| format!("{}={}", name, self.named[name].len()))
            .collect();
        parts.sort();
        format!(" [{}]", parts.join(" "))
    }
}

/// Session-wide default flags that get injected into every command.
struct SessionDefaults {
    flags: HashSet<String>,
}

/// Flags that can be set as session defaults.
const SETTABLE_FLAGS: &[&str] = &["--json", "--log", "--verbose", "--debug", "--time"];

impl SessionDefaults {
    fn new() -> Self {
        Self {
            flags: HashSet::new(),
        }
    }

    fn set(&mut self, flag: &str) -> bool {
        if SETTABLE_FLAGS.contains(&flag) {
            self.flags.insert(flag.to_string());
            true
        } else {
            false
        }
    }

    fn unset_flag(&mut self, flag: &str) -> bool {
        self.flags.remove(flag)
    }

    fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Inject session defaults into a token list (after "maki" argv[0], before other args).
    fn inject(&self, tokens: &mut Vec<String>) {
        if self.flags.is_empty() {
            return;
        }
        // Insert after the first token (the subcommand) — but we insert into
        // the args list after "maki" is prepended, so position 2 (after "maki" + subcommand).
        // Actually, global flags go right after "maki" (before the subcommand) for clap.
        // But clap also accepts them after the subcommand. Let's append to the end
        // to avoid interfering with subcommand position detection.
        for flag in &self.flags {
            if !tokens.contains(flag) {
                tokens.push(flag.clone());
            }
        }
    }

    fn display(&self) -> String {
        if self.flags.is_empty() {
            return String::new();
        }
        let mut sorted: Vec<&String> = self.flags.iter().collect();
        sorted.sort();
        sorted.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(" ")
    }
}

// ---------------------------------------------------------------------------
// Interactive mode
// ---------------------------------------------------------------------------

/// Run an interactive REPL session.
fn run_interactive(
    catalog_root: &Path,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    let catalog_name = catalog_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("maki");

    let helper = ShellHelper::new(catalog_root);
    let mut rl = rustyline::Editor::new().unwrap();
    rl.set_helper(Some(helper));

    // Load history (ignore errors — file may not exist yet)
    let history_path = catalog_root.join(".maki").join("shell_history");
    if history_path.exists() {
        let _ = rl.load_history(&history_path);
    }

    let mut vars = Variables::new();
    let mut defaults = SessionDefaults::new();

    eprintln!("maki shell v{} — type 'help' or 'quit' to exit", env!("CARGO_PKG_VERSION"));

    loop {
        // Update variable names in the helper for completion
        if let Some(h) = rl.helper_mut() {
            h.variable_names = vars.names();
        }

        let context = vars.prompt_context();
        let prompt = format!("{catalog_name}{context}> ");
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                let _ = rl.add_history_entry(trimmed);

                match handle_line(trimmed, &mut vars, &mut defaults, catalog_root, executor) {
                    LineResult::Ok(ids) => {
                        if !ids.is_empty() {
                            vars.last_ids = ids;
                        }
                    }
                    LineResult::Err(msg) => eprintln!("Error: {msg:#}"),
                    LineResult::Quit => break,
                    LineResult::Blocked(cmd) => {
                        eprintln!("'{cmd}' cannot be used inside the shell.");
                    }
                    LineResult::Handled => {}
                    LineResult::Reload => {
                        vars = Variables::new();
                        defaults = SessionDefaults::new();
                        if let Some(h) = rl.helper_mut() {
                            h.refresh(catalog_root);
                            h.variable_names = Vec::new();
                        }
                        eprintln!("  Reloaded config, cleared variables and session defaults.");
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: cancel current line, continue
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit
                break;
            }
            Err(e) => {
                eprintln!("Readline error: {e}");
                break;
            }
        }
    }

    // Save history
    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.save_history(&history_path);
}

// ---------------------------------------------------------------------------
// Script mode
// ---------------------------------------------------------------------------

/// Run a script file.
fn run_script(
    path: &Path,
    strict: bool,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading script {}: {e}", path.display());
            return;
        }
    };

    run_lines(&content, Some(path), strict, executor);
}

/// Run a single command string (maki shell -c '...').
fn run_single_command(
    command: &str,
    strict: bool,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    run_lines(command, None, strict, executor);
}

/// Run lines from a script or -c command, with shared variable state.
fn run_lines(
    content: &str,
    source_path: Option<&Path>,
    strict: bool,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    let mut vars = Variables::new();
    let mut defaults = SessionDefaults::new();
    let catalog_root = crate::config::find_catalog_root().ok();

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let result = handle_line(
            trimmed,
            &mut vars,
            &mut defaults,
            catalog_root.as_deref().unwrap_or(Path::new(".")),
            executor,
        );

        match result {
            LineResult::Ok(ids) => {
                if !ids.is_empty() {
                    vars.last_ids = ids;
                }
            }
            LineResult::Err(msg) => {
                if let Some(path) = source_path {
                    eprintln!("{}:{}: Error: {msg:#}", path.display(), line_num + 1);
                } else {
                    eprintln!("Error: {msg:#}");
                }
                if strict {
                    std::process::exit(1);
                }
            }
            LineResult::Quit => break,
            LineResult::Blocked(cmd) => {
                if let Some(path) = source_path {
                    eprintln!(
                        "{}:{}: '{}' cannot be used in scripts.",
                        path.display(),
                        line_num + 1,
                        cmd
                    );
                } else {
                    eprintln!("'{cmd}' cannot be used in scripts.");
                }
                if strict {
                    std::process::exit(1);
                }
            }
            LineResult::Handled | LineResult::Reload => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Line handling
// ---------------------------------------------------------------------------

enum LineResult {
    Ok(Vec<String>),
    Err(anyhow::Error),
    Quit,
    Blocked(String),
    /// Line was fully handled by a built-in (no IDs to capture).
    Handled,
    /// Reload command — caller must refresh state.
    Reload,
}

/// Process a single shell line.
fn handle_line(
    line: &str,
    vars: &mut Variables,
    defaults: &mut SessionDefaults,
    catalog_root: &Path,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) -> LineResult {
    // Check for variable assignment: $name = <command...>
    if let Some(rest) = parse_variable_assignment(line) {
        let (var_name, command_part) = rest;
        if command_part.is_empty() {
            return LineResult::Err(anyhow::anyhow!("no command after variable assignment"));
        }

        let tokens = match shell_split(&command_part) {
            Some(t) => t,
            None => return LineResult::Err(anyhow::anyhow!("unmatched quote in command")),
        };

        // Check if it's just `$name = _` or `$name = $other` — copy variable contents
        if tokens.len() == 1 {
            if tokens[0] == "_" && !vars.last_ids.is_empty() {
                let count = vars.last_ids.len();
                vars.named.insert(var_name.clone(), vars.last_ids.clone());
                eprintln!("  {count} assets → ${var_name}");
                return LineResult::Handled;
            }
            if tokens[0].starts_with('$') {
                let src_name = &tokens[0][1..];
                if let Some(ids) = vars.named.get(src_name).cloned() {
                    let count = ids.len();
                    vars.named.insert(var_name.clone(), ids);
                    eprintln!("  {count} assets → ${var_name}");
                    return LineResult::Handled;
                }
            }
        }

        // Otherwise execute the command (with token-level variable expansion)
        let expanded = expand_variables_in_tokens(tokens, vars);
        return match execute_with_ids(expanded.command, expanded.asset_ids, defaults, executor) {
            LineResult::Ok(ids) => {
                let count = ids.len();
                if !ids.is_empty() {
                    vars.last_ids = ids.clone();
                }
                vars.named.insert(var_name.clone(), ids);
                eprintln!("  {count} assets → ${var_name}");
                LineResult::Handled
            }
            other => other,
        };
    }

    // Built-in commands
    match line {
        "quit" | "exit" => return LineResult::Quit,
        "help" => {
            print_shell_help();
            return LineResult::Handled;
        }
        "vars" => {
            print_vars(vars, defaults);
            return LineResult::Handled;
        }
        "reload" => {
            return LineResult::Reload;
        }
        _ => {}
    }

    // preview <asset_id|$var|_>
    if line == "preview" || line.starts_with("preview ") {
        let rest = line.strip_prefix("preview").unwrap_or("").trim();
        // Accept (and ignore) the legacy --open flag for backward compatibility
        let args = rest.strip_prefix("--open").map(str::trim).unwrap_or(rest);
        if args.is_empty() {
            eprintln!("  Usage: preview <asset_id>");
            return LineResult::Handled;
        }
        // Expand variables and _ in args
        let tokens = match shell_split(args) {
            Some(t) => t,
            None => {
                eprintln!("  Error: unmatched quotes");
                return LineResult::Handled;
            }
        };
        let expanded = expand_variables_in_tokens(tokens, vars);
        let ids: Vec<String> = if expanded.asset_ids.is_empty() {
            // No variable expansion — treat as literal asset ID(s)
            expanded.command
        } else {
            expanded.asset_ids
        };
        // Open previews in the OS default viewer
        match handle_preview_builtin(catalog_root, &ids) {
            Ok(displayed_ids) => {
                if !displayed_ids.is_empty() {
                    vars.last_ids = displayed_ids;
                }
            }
            Err(e) => eprintln!("  Error: {e:#}"),
        }
        return LineResult::Handled;
    }

    // export <query|$var|_> <target> [--layout flat|mirror] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite] [--symlink] [--zip]
    if line == "export" || line.starts_with("export ") {
        let rest = line.strip_prefix("export").unwrap_or("").trim();
        if rest.is_empty() {
            eprintln!("  Usage: export <query|$var> <target> [--layout flat|mirror] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite] [--symlink] [--zip]");
            return LineResult::Handled;
        }
        let tokens = match shell_split(rest) {
            Some(t) => t,
            None => {
                eprintln!("  Error: unmatched quotes");
                return LineResult::Handled;
            }
        };
        let expanded = expand_variables_in_tokens(tokens, vars);

        // Separate flags from positional args
        let mut flags = Vec::new();
        let mut positionals = Vec::new();
        let mut iter = expanded.command.iter();
        while let Some(tok) = iter.next() {
            if tok == "--layout" {
                flags.push(tok.clone());
                if let Some(val) = iter.next() {
                    flags.push(val.clone());
                }
            } else if tok.starts_with("--") {
                flags.push(tok.clone());
            } else {
                positionals.push(tok.clone());
            }
        }

        // Build the query: if variable expansion produced IDs, use id:xxx; otherwise use positionals[0]
        let (query, target) = if !expanded.asset_ids.is_empty() {
            // Variable expanded — first positional (if any) is the target dir
            let id_query = expanded.asset_ids.iter()
                .map(|id| format!("id:{id}"))
                .collect::<Vec<_>>()
                .join(" ");
            let target = positionals.first().cloned().unwrap_or_default();
            (id_query, target)
        } else if positionals.len() >= 2 {
            // No variable — first positional is query, second is target
            (positionals[0].clone(), positionals[1].clone())
        } else {
            eprintln!("  Usage: export <query|$var> <target> [--layout flat|mirror] [--all-variants] [--include-sidecars] [--dry-run] [--zip]");
            return LineResult::Handled;
        };

        if target.is_empty() {
            eprintln!("  Error: target is required");
            return LineResult::Handled;
        }

        let mut args = vec![
            "maki".to_string(),
            "export".to_string(),
            query,
            target,
        ];
        args.extend(flags);
        defaults.inject(&mut args);
        match executor(args) {
            Ok(ids) => {
                if !ids.is_empty() {
                    vars.last_ids = ids;
                }
            }
            Err(e) => eprintln!("  Error: {e:#}"),
        }
        return LineResult::Handled;
    }

    // set --flag
    if let Some(rest) = line.strip_prefix("set ") {
        let flag = rest.trim();
        if flag.starts_with("--") {
            if defaults.set(flag) {
                eprintln!("  Session default: {flag}");
                if !defaults.is_empty() {
                    eprintln!("  Active defaults: {}", defaults.display());
                }
            } else {
                eprintln!("  Unknown flag '{flag}'. Settable flags: {}", SETTABLE_FLAGS.join(", "));
            }
        } else {
            eprintln!("  Usage: set --flag (e.g. set --log, set --json)");
        }
        return LineResult::Handled;
    }

    // unset $name or unset --flag
    if let Some(rest) = line.strip_prefix("unset ") {
        let name = rest.trim();
        if let Some(var_name) = name.strip_prefix('$') {
            if vars.named.remove(var_name).is_some() {
                eprintln!("  Removed ${var_name}");
            } else {
                eprintln!("  Variable ${var_name} not defined");
            }
        } else if name.starts_with("--") {
            if defaults.unset_flag(name) {
                eprintln!("  Removed session default: {name}");
            } else {
                eprintln!("  '{name}' is not set as a session default");
            }
        } else {
            eprintln!("  Usage: unset $name or unset --flag");
        }
        return LineResult::Handled;
    }

    // source <file>
    if let Some(rest) = line.strip_prefix("source ") {
        let file_path = rest.trim();
        if file_path.is_empty() {
            eprintln!("  Usage: source <file>");
            return LineResult::Handled;
        }
        // Resolve relative to catalog root
        let path = if Path::new(file_path).is_absolute() {
            PathBuf::from(file_path)
        } else {
            catalog_root.join(file_path)
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return LineResult::Err(anyhow::anyhow!(
                    "Cannot read {}: {e}",
                    path.display()
                ));
            }
        };
        // Execute each line in the sourced file, sharing our session state
        for (line_num, src_line) in content.lines().enumerate() {
            let trimmed = src_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            match handle_line(trimmed, vars, defaults, catalog_root, executor) {
                LineResult::Ok(ids) => {
                    if !ids.is_empty() {
                        vars.last_ids = ids;
                    }
                }
                LineResult::Err(msg) => {
                    eprintln!("{}:{}: Error: {msg:#}", path.display(), line_num + 1);
                }
                LineResult::Quit => return LineResult::Quit,
                LineResult::Blocked(cmd) => {
                    eprintln!(
                        "{}:{}: '{}' cannot be used in scripts.",
                        path.display(),
                        line_num + 1,
                        cmd
                    );
                }
                LineResult::Handled | LineResult::Reload => {}
            }
        }
        return LineResult::Handled;
    }

    // Shell-split the line into tokens (before variable expansion)
    let tokens = match shell_split(line) {
        Some(t) => t,
        None => return LineResult::Err(anyhow::anyhow!("unmatched quote in command")),
    };

    // Expand variables: extract $name/_ references and collect their IDs
    // separately, so the shell can dispatch them correctly.
    let expanded = expand_variables_in_tokens(tokens, vars);

    execute_with_ids(expanded.command, expanded.asset_ids, defaults, executor)
}

/// Parse a `$name = <command>` assignment. Returns (name, command_part).
fn parse_variable_assignment(line: &str) -> Option<(String, String)> {
    // Must start with $
    let rest = line.strip_prefix('$')?;

    // Find the = sign
    let eq_pos = rest.find('=')?;

    let name = rest[..eq_pos].trim();
    // Validate variable name: alphanumeric + underscore
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    let command = rest[eq_pos + 1..].trim();
    Some((name.to_string(), command.to_string()))
}

/// Commands that take a single `asset_id: String` positional.
/// When a variable expands to multiple IDs, the shell loops the command
/// over each ID individually instead of appending all IDs at once.
const SINGLE_ASSET_COMMANDS: &[&str] = &[
    "tag", "edit", "show", "split", "update-location",
];

/// Execute a command with expanded variable tokens.
fn execute_with_ids(
    command_tokens: Vec<String>,
    asset_ids: Vec<String>,
    defaults: &SessionDefaults,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) -> LineResult {
    if command_tokens.is_empty() && asset_ids.is_empty() {
        return LineResult::Ok(Vec::new());
    }

    // Block commands that don't make sense in the shell
    let cmd = command_tokens.first().map(|s| s.to_lowercase()).unwrap_or_default();
    if matches!(cmd.as_str(), "init" | "migrate" | "serve" | "shell") {
        return LineResult::Blocked(cmd);
    }

    // If no variable IDs were expanded, run the command as-is
    if asset_ids.is_empty() {
        let mut args = vec!["maki".to_string()];
        args.extend(command_tokens);
        defaults.inject(&mut args);
        return match executor(args) {
            Ok(ids) => LineResult::Ok(ids),
            Err(e) => LineResult::Err(e),
        };
    }

    // Single-asset commands: loop over each ID individually.
    // The asset ID is inserted right after the subcommand name (position 1
    // in command_tokens), because these commands expect the first positional
    // to be the asset ID (e.g. `tag <ASSET_ID> [TAGS]...`).
    if SINGLE_ASSET_COMMANDS.contains(&cmd.as_str()) && asset_ids.len() > 1 {
        let total = asset_ids.len();
        let mut all_ids = Vec::new();
        let mut errors = 0;
        for (i, id) in asset_ids.iter().enumerate() {
            let mut args = vec!["maki".to_string()];
            // Insert: subcommand, then asset ID, then remaining args
            args.push(command_tokens[0].clone()); // subcommand name
            args.push(id.clone());                // asset ID
            args.extend(command_tokens[1..].iter().cloned()); // remaining args (tags, flags, etc.)
            defaults.inject(&mut args);
            match executor(args) {
                Ok(ids) => all_ids.extend(ids),
                Err(e) => {
                    eprintln!("  [{}/{}] {}: {e:#}", i + 1, total, &id[..8.min(id.len())]);
                    errors += 1;
                }
            }
        }
        if errors > 0 {
            eprintln!("  {errors} of {total} failed");
        }
        return LineResult::Ok(all_ids);
    }

    // Single ID for a single-asset command: insert ID after subcommand
    if SINGLE_ASSET_COMMANDS.contains(&cmd.as_str()) && asset_ids.len() == 1 {
        let mut args = vec!["maki".to_string()];
        args.push(command_tokens[0].clone());
        args.push(asset_ids[0].clone());
        args.extend(command_tokens[1..].iter().cloned());
        defaults.inject(&mut args);
        return match executor(args) {
            Ok(ids) => LineResult::Ok(ids),
            Err(e) => LineResult::Err(e),
        };
    }

    // Batch commands: append IDs at end
    let mut args = vec!["maki".to_string()];
    args.extend(command_tokens);
    args.extend(asset_ids);
    defaults.inject(&mut args);
    match executor(args) {
        Ok(ids) => LineResult::Ok(ids),
        Err(e) => LineResult::Err(e),
    }
}

/// Handle the `preview` built-in: display preview images for one or more assets.
fn handle_preview_builtin(
    catalog_root: &Path,
    asset_ids: &[String],
) -> Result<Vec<String>> {
    let config = crate::config::CatalogConfig::load(catalog_root)?;
    let catalog = crate::catalog::Catalog::open(catalog_root)?;
    let preview_gen = crate::preview::PreviewGenerator::new(catalog_root, crate::Verbosity::quiet(), &config.preview);
    let engine = crate::query::QueryEngine::new(catalog_root);

    let mut displayed = Vec::new();
    for raw_id in asset_ids {
        let details = engine.show(raw_id)?;
        let full_id = &details.id;
        let name = details.name.as_deref().unwrap_or(full_id);

        // Find best preview (stored override → algorithmic)
        let best_hash = catalog.get_asset_best_variant_hash(full_id)
            .unwrap_or(None)
            .or_else(|| {
                crate::models::variant::best_preview_index_details(&details.variants)
                    .map(|i| details.variants[i].content_hash.clone())
            });

        let preview_path = best_hash.as_ref().and_then(|h| {
            let smart = preview_gen.smart_preview_path(h);
            if smart.exists() { return Some(smart); }
            let regular = preview_gen.preview_path(h);
            if regular.exists() { return Some(regular); }
            None
        });

        match preview_path {
            Some(path) => {
                crate::preview::open_in_viewer(&path)?;
                eprintln!("  Opened {name}");
                displayed.push(full_id.to_string());
            }
            None => {
                eprintln!("  {name}: no preview available");
            }
        }
    }
    if !displayed.is_empty() {
        eprintln!("  {} asset(s) previewed", displayed.len());
    }
    Ok(displayed)
}

/// Print all defined variables and their asset counts.
fn print_vars(vars: &Variables, defaults: &SessionDefaults) {
    let has_vars = !vars.named.is_empty() || !vars.last_ids.is_empty();
    let has_defaults = !defaults.is_empty();

    if !has_vars && !has_defaults {
        eprintln!("  No variables or session defaults defined.");
        return;
    }

    if has_defaults {
        eprintln!("  Session defaults: {}", defaults.display());
    }

    if !vars.last_ids.is_empty() {
        eprintln!("  _ = {} assets", vars.last_ids.len());
    }

    for name in vars.names() {
        let ids = &vars.named[&name];
        eprintln!("  ${name} = {} assets", ids.len());
    }
}

/// Expand `_` tokens to the list of asset IDs from the last command.
/// Result of expanding variables in a token list.
struct ExpandedTokens {
    /// Command tokens with variable references removed.
    command: Vec<String>,
    /// Asset IDs collected from all variable references.
    asset_ids: Vec<String>,
}

/// Expand variable references in a token list.
///
/// Variables (`$name` and standalone `_`) are removed from the token list.
/// Their asset IDs are collected separately so the shell can dispatch them
/// correctly — either as trailing positional args (for batch commands) or
/// by looping the command over each ID (for single-asset commands).
/// Expand `~` or `~/...` to the user's home directory.
fn expand_tilde(token: &str) -> String {
    if token == "~" || token.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return format!("{}{}", home, &token[1..]);
        }
    }
    token.to_string()
}

fn expand_variables_in_tokens(tokens: Vec<String>, vars: &Variables) -> ExpandedTokens {
    let mut command_tokens = Vec::new();
    let mut asset_ids: Vec<String> = Vec::new();

    for token in &tokens {
        // Standalone _ (not part of a word like _foo or foo_bar)
        if token == "_" {
            if !vars.last_ids.is_empty() {
                asset_ids.extend(vars.last_ids.iter().cloned());
            } else {
                // No last IDs — keep as-is
                command_tokens.push(token.clone());
            }
        } else if token.starts_with('$') && token.len() > 1
            && token[1..].chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            let name = &token[1..];
            if let Some(ids) = vars.named.get(name) {
                asset_ids.extend(ids.iter().cloned());
            } else {
                // Unknown variable — keep as-is
                command_tokens.push(token.clone());
            }
        } else {
            // Expand ~ to home directory in path-like tokens
            command_tokens.push(expand_tilde(token));
        }
    }

    ExpandedTokens { command: command_tokens, asset_ids }
}

/// Split a command line into tokens, respecting quotes.
///
/// Quotes that wrap an entire token are stripped (grouping quotes):
///   `"tag:landscape rating:4+"` → `tag:landscape rating:4+`
///
/// Quotes that appear mid-token are preserved (syntax quotes):
///   `text:"woman with glasses"` → `text:"woman with glasses"`
///
/// This allows search syntax like `text:"query"` to pass through unchanged
/// while still supporting shell-style grouping for multi-word arguments.
fn shell_split(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    // Track whether the opening quote was at token start (for stripping)
    let mut single_at_start = false;
    let mut double_at_start = false;

    for c in line.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }

        match c {
            '\\' if !in_single => {
                escaped = true;
            }
            '\'' if !in_double => {
                if in_single {
                    // Closing single quote
                    in_single = false;
                    if !single_at_start {
                        current.push('\''); // mid-token: preserve closing quote
                    }
                    single_at_start = false;
                } else {
                    // Opening single quote
                    in_single = true;
                    if current.is_empty() {
                        single_at_start = true; // token-start: will strip
                    } else {
                        single_at_start = false;
                        current.push('\''); // mid-token: preserve opening quote
                    }
                }
            }
            '"' if !in_single => {
                if in_double {
                    // Closing double quote
                    in_double = false;
                    if !double_at_start {
                        current.push('"'); // mid-token: preserve closing quote
                    }
                    double_at_start = false;
                } else {
                    // Opening double quote
                    in_double = true;
                    if current.is_empty() {
                        double_at_start = true; // token-start: will strip
                    } else {
                        double_at_start = false;
                        current.push('"'); // mid-token: preserve opening quote
                    }
                }
            }
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if in_single || in_double {
        return None; // Unmatched quote
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Some(tokens)
}

fn print_shell_help() {
    eprintln!("\
maki shell — interactive asset management shell

Enter any maki command without the 'maki' prefix:
  search \"tag:landscape rating:4+\"
  edit --rating 5 abc12345
  stats

Variables:
  $name = <command>  Store command results in a named variable
  $name              Expands to stored asset IDs in any command
  _                  Expands to asset IDs from the last command
  vars               List all variables and session defaults
  unset $name        Remove a variable

Session defaults:
  set --flag         Add a default flag to all commands (--json, --log, --debug, --time)
  unset --flag       Remove a session default

Session management:
  source <file>      Execute a script file in the current session (shares variables)
  reload             Re-read config, refresh completions, clear variables and defaults

Examples:
  $picks = search \"rating:5 date:2024\"
  tag --add \"portfolio\" $picks
  export --target /tmp/best $picks
  set --log
  source post-import.dam

Other syntax:
  # comment          Lines starting with # are ignored

Shell commands:
  preview <id>       Open the asset's preview in the OS default image viewer
  help               Show this help
  quit / exit        End the session (also Ctrl-D)

Tab completion:
  Subcommand names, --flags, $variables, tag:names, volume:labels

Blocked commands (use outside the shell):
  init, migrate, serve, shell");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_executor(ids: Vec<String>) -> impl Fn(Vec<String>) -> Result<Vec<String>> {
        move |_args: Vec<String>| -> Result<Vec<String>> { Ok(ids.clone()) }
    }

    fn noop_executor(_args: Vec<String>) -> Result<Vec<String>> {
        Ok(vec![])
    }

    #[test]
    fn test_shell_split_simple() {
        assert_eq!(
            shell_split("search tag:landscape"),
            Some(vec!["search".to_string(), "tag:landscape".to_string()])
        );
    }

    #[test]
    fn test_shell_split_quotes() {
        assert_eq!(
            shell_split(r#"search "tag:landscape rating:4+""#),
            Some(vec!["search".to_string(), "tag:landscape rating:4+".to_string()])
        );
    }

    #[test]
    fn test_shell_split_single_quotes() {
        assert_eq!(
            shell_split("search 'tag:landscape'"),
            Some(vec!["search".to_string(), "tag:landscape".to_string()])
        );
    }

    #[test]
    fn test_shell_split_unmatched() {
        assert_eq!(shell_split("search \"unmatched"), None);
    }

    #[test]
    fn test_shell_split_empty() {
        assert_eq!(shell_split(""), Some(vec![]));
    }

    #[test]
    fn test_expand_variables_underscore_at_end() {
        let mut vars = Variables::new();
        vars.last_ids = vec!["abc123".to_string(), "def456".to_string()];
        let tokens = vec!["edit".into(), "--rating".into(), "5".into(), "_".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["edit", "--rating", "5"]);
        assert_eq!(expanded.asset_ids, vec!["abc123", "def456"]);
    }

    #[test]
    fn test_expand_variables_underscore_at_start() {
        let mut vars = Variables::new();
        vars.last_ids = vec!["abc123".to_string(), "def456".to_string()];
        let tokens = vec!["tag".into(), "_".into(), "screensaver".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        // _ is removed; "screensaver" stays as a command token
        assert_eq!(expanded.command, vec!["tag", "screensaver"]);
        assert_eq!(expanded.asset_ids, vec!["abc123", "def456"]);
    }

    #[test]
    fn test_expand_variables_position_independent() {
        let mut vars = Variables::new();
        vars.last_ids = vec!["id1".to_string(), "id2".to_string()];

        // _ at start
        let tokens = vec!["tag".into(), "_".into(), "screensaver".into()];
        let exp1 = expand_variables_in_tokens(tokens, &vars);

        // _ at end
        let tokens = vec!["tag".into(), "screensaver".into(), "_".into()];
        let exp2 = expand_variables_in_tokens(tokens, &vars);

        // Both produce the same command + asset_ids
        assert_eq!(exp1.command, exp2.command);
        assert_eq!(exp1.asset_ids, exp2.asset_ids);
    }

    #[test]
    fn test_expand_variables_no_last_ids() {
        let vars = Variables::new();
        let tokens = vec!["edit".into(), "--rating".into(), "5".into(), "_".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        // No last_ids — _ kept as-is in command tokens
        assert_eq!(expanded.command, vec!["edit", "--rating", "5", "_"]);
        assert!(expanded.asset_ids.is_empty());
    }

    #[test]
    fn test_shell_split_mid_token_double_quotes_preserved() {
        assert_eq!(
            shell_split(r#"search text:"woman with glasses""#),
            Some(vec![
                "search".to_string(),
                r#"text:"woman with glasses""#.to_string(),
            ])
        );
    }

    #[test]
    fn test_shell_split_mid_token_single_quotes_preserved() {
        assert_eq!(
            shell_split("search text:'woman with glasses'"),
            Some(vec![
                "search".to_string(),
                "text:'woman with glasses'".to_string(),
            ])
        );
    }

    #[test]
    fn test_shell_split_grouping_with_mid_token_quotes() {
        assert_eq!(
            shell_split(r#""text:'woman with glasses'""#),
            Some(vec!["text:'woman with glasses'".to_string()])
        );
    }

    #[test]
    fn test_expand_variables_underscore_in_word_not_expanded() {
        let mut vars = Variables::new();
        vars.last_ids = vec!["abc123".to_string()];
        // _foo and foo_bar are not standalone _ — they should not be expanded
        let tokens = vec!["search".into(), "_foo".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["search", "_foo"]);
        assert!(expanded.asset_ids.is_empty());

        let tokens = vec!["search".into(), "foo_bar".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["search", "foo_bar"]);
        assert!(expanded.asset_ids.is_empty());
    }

    // --- Phase 2: Variable tests ---

    #[test]
    fn test_parse_variable_assignment() {
        assert_eq!(
            parse_variable_assignment("$picks = search \"rating:5\""),
            Some(("picks".to_string(), "search \"rating:5\"".to_string()))
        );
        assert_eq!(
            parse_variable_assignment("$my_var=search tag:landscape"),
            Some(("my_var".to_string(), "search tag:landscape".to_string()))
        );
        assert_eq!(parse_variable_assignment("search tag:landscape"), None);
        assert_eq!(parse_variable_assignment("$ = search"), None);
        assert_eq!(parse_variable_assignment("$foo-bar = search"), None);
    }

    #[test]
    fn test_expand_variables_named_at_end() {
        let mut vars = Variables::new();
        vars.named.insert("picks".to_string(), vec!["id1".to_string(), "id2".to_string()]);

        let tokens = vec!["delete".into(), "$picks".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["delete"]);
        assert_eq!(expanded.asset_ids, vec!["id1", "id2"]);
    }

    #[test]
    fn test_expand_variables_named_at_start() {
        let mut vars = Variables::new();
        vars.named.insert("picks".to_string(), vec!["id1".to_string(), "id2".to_string()]);

        // $picks at start — IDs are separated from command tokens
        let tokens = vec!["tag".into(), "$picks".into(), "portfolio".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["tag", "portfolio"]);
        assert_eq!(expanded.asset_ids, vec!["id1", "id2"]);
    }

    #[test]
    fn test_expand_variables_unknown_kept() {
        let vars = Variables::new();
        let tokens = vec!["tag".into(), "$unknown".into(), "portfolio".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["tag", "$unknown", "portfolio"]);
        assert!(expanded.asset_ids.is_empty());
    }

    #[test]
    fn test_expand_variables_dollar_not_var() {
        let vars = Variables::new();
        // Bare $ is not a variable reference
        let tokens = vec!["echo".into(), "$".into(), "done".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        assert_eq!(expanded.command, vec!["echo", "$", "done"]);
        assert!(expanded.asset_ids.is_empty());
    }

    #[test]
    fn test_handle_line_vars_command() {
        let mut vars = Variables::new();
        vars.named.insert("test".to_string(), vec!["id1".to_string()]);
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");

        match handle_line("vars", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Handled => {}
            _ => panic!("Expected Handled for vars command"),
        }
    }

    #[test]
    fn test_handle_line_unset_variable() {
        let mut vars = Variables::new();
        vars.named.insert("test".to_string(), vec!["id1".to_string()]);
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");

        match handle_line("unset $test", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Handled => {}
            _ => panic!("Expected Handled for unset"),
        }
        assert!(!vars.named.contains_key("test"));
    }

    #[test]
    fn test_handle_line_variable_assignment() {
        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");
        let executor = dummy_executor(vec!["a1".to_string(), "a2".to_string()]);

        match handle_line("$picks = search rating:5", &mut vars, &mut defaults, root, &executor) {
            LineResult::Handled => {}
            _ => panic!("Expected Handled for variable assignment"),
        }
        assert_eq!(vars.named.get("picks").unwrap(), &vec!["a1".to_string(), "a2".to_string()]);
        assert_eq!(vars.last_ids, vec!["a1".to_string(), "a2".to_string()]);
    }

    #[test]
    fn test_prompt_context_empty() {
        let vars = Variables::new();
        assert_eq!(vars.prompt_context(), "");
    }

    #[test]
    fn test_prompt_context_with_vars() {
        let mut vars = Variables::new();
        vars.named.insert("picks".to_string(), vec!["a".to_string(), "b".to_string()]);
        vars.named.insert("best".to_string(), vec!["c".to_string()]);
        let ctx = vars.prompt_context();
        assert_eq!(ctx, " [best=1 picks=2]");
    }

    // --- Phase 3: Session defaults, set/unset, source, reload ---

    #[test]
    fn test_session_defaults_set_valid() {
        let mut defaults = SessionDefaults::new();
        assert!(defaults.set("--json"));
        assert!(defaults.set("--log"));
        assert!(!defaults.set("--unknown"));
        assert_eq!(defaults.display(), "--json --log");
    }

    #[test]
    fn test_session_defaults_unset() {
        let mut defaults = SessionDefaults::new();
        defaults.set("--json");
        defaults.set("--log");
        assert!(defaults.unset_flag("--json"));
        assert!(!defaults.unset_flag("--unknown"));
        assert_eq!(defaults.display(), "--log");
    }

    #[test]
    fn test_session_defaults_inject() {
        let mut defaults = SessionDefaults::new();
        defaults.set("--json");
        defaults.set("--log");

        let mut args = vec!["maki".to_string(), "search".to_string(), "tag:landscape".to_string()];
        defaults.inject(&mut args);

        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"--log".to_string()));
    }

    #[test]
    fn test_session_defaults_inject_no_duplicate() {
        let mut defaults = SessionDefaults::new();
        defaults.set("--json");

        // Command already has --json
        let mut args = vec!["maki".to_string(), "search".to_string(), "--json".to_string()];
        defaults.inject(&mut args);

        // Should not add a duplicate
        let json_count = args.iter().filter(|a| a.as_str() == "--json").count();
        assert_eq!(json_count, 1);
    }

    #[test]
    fn test_handle_line_set_flag() {
        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");

        match handle_line("set --json", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Handled => {}
            _ => panic!("Expected Handled"),
        }
        assert!(defaults.flags.contains("--json"));
    }

    #[test]
    fn test_handle_line_unset_flag() {
        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();
        defaults.set("--log");
        let root = Path::new(".");

        match handle_line("unset --log", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Handled => {}
            _ => panic!("Expected Handled"),
        }
        assert!(!defaults.flags.contains("--log"));
    }

    #[test]
    fn test_handle_line_reload() {
        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");

        match handle_line("reload", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Reload => {}
            _ => panic!("Expected Reload"),
        }
    }

    #[test]
    fn test_handle_line_source_missing_file() {
        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();
        let root = Path::new(".");

        match handle_line("source nonexistent.dam", &mut vars, &mut defaults, root, &noop_executor) {
            LineResult::Err(_) => {}
            _ => panic!("Expected Err for missing source file"),
        }
    }

    #[test]
    fn test_handle_line_source_runs_script() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("test.dam");
        std::fs::write(&script_path, "# comment\nstats\n").unwrap();

        let mut vars = Variables::new();
        let mut defaults = SessionDefaults::new();

        // source resolves relative to catalog_root
        match handle_line("source test.dam", &mut vars, &mut defaults, dir.path(), &noop_executor) {
            LineResult::Handled => {}
            other => panic!("Expected Handled, got {:?}", line_result_name(&other)),
        }
    }

    #[test]
    fn test_execute_with_defaults() {
        let mut defaults = SessionDefaults::new();
        defaults.set("--json");

        // Track what args the executor receives
        let received = std::cell::RefCell::new(Vec::new());
        let executor = |args: Vec<String>| -> Result<Vec<String>> {
            *received.borrow_mut() = args;
            Ok(vec![])
        };

        let command = vec!["stats".to_string()];
        execute_with_ids(command, vec![], &defaults, &executor);

        let args = received.borrow();
        assert_eq!(args[0], "maki");
        assert_eq!(args[1], "stats");
        assert!(args.contains(&"--json".to_string()));
    }

    #[test]
    fn test_execute_single_asset_command_loops() {
        // Track all executor calls
        let calls = std::cell::RefCell::new(Vec::new());
        let executor = |args: Vec<String>| -> Result<Vec<String>> {
            calls.borrow_mut().push(args);
            Ok(vec![])
        };
        let defaults = SessionDefaults::new();

        // `tag` is a single-asset command — with 3 IDs, it should loop 3 times
        // Each call inserts the asset ID after the subcommand: maki tag <ID> screensaver
        let command = vec!["tag".to_string(), "screensaver".to_string()];
        let ids = vec!["id1".to_string(), "id2".to_string(), "id3".to_string()];
        execute_with_ids(command, ids, &defaults, &executor);

        let calls = calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], vec!["maki", "tag", "id1", "screensaver"]);
        assert_eq!(calls[1], vec!["maki", "tag", "id2", "screensaver"]);
        assert_eq!(calls[2], vec!["maki", "tag", "id3", "screensaver"]);
    }

    #[test]
    fn test_execute_batch_command_appends() {
        let calls = std::cell::RefCell::new(Vec::new());
        let executor = |args: Vec<String>| -> Result<Vec<String>> {
            calls.borrow_mut().push(args);
            Ok(vec![])
        };
        let defaults = SessionDefaults::new();

        // `delete` is a batch command — IDs should be appended in one call
        let command = vec!["delete".to_string(), "--apply".to_string()];
        let ids = vec!["id1".to_string(), "id2".to_string()];
        execute_with_ids(command, ids, &defaults, &executor);

        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["maki", "delete", "--apply", "id1", "id2"]);
    }

    #[test]
    fn test_execute_single_asset_with_one_id() {
        let calls = std::cell::RefCell::new(Vec::new());
        let executor = |args: Vec<String>| -> Result<Vec<String>> {
            calls.borrow_mut().push(args);
            Ok(vec![])
        };
        let defaults = SessionDefaults::new();

        // Single ID for single-asset command: maki tag <ID> screensaver
        let command = vec!["tag".to_string(), "screensaver".to_string()];
        let ids = vec!["id1".to_string()];
        execute_with_ids(command, ids, &defaults, &executor);

        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["maki", "tag", "id1", "screensaver"]);
    }

    /// Helper for debug output in test assertions.
    fn line_result_name(r: &LineResult) -> &'static str {
        match r {
            LineResult::Ok(_) => "Ok",
            LineResult::Err(_) => "Err",
            LineResult::Quit => "Quit",
            LineResult::Blocked(_) => "Blocked",
            LineResult::Handled => "Handled",
            LineResult::Reload => "Reload",
        }
    }

    #[test]
    fn tilde_expansion() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        assert_eq!(expand_tilde("~/Desktop"), format!("{home}/Desktop"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("hello"), "hello");
    }

    #[test]
    fn tilde_in_variable_expansion() {
        let vars = Variables::new();
        let tokens = vec!["export".into(), "~/out".into()];
        let expanded = expand_variables_in_tokens(tokens, &vars);
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        assert_eq!(expanded.command, vec!["export", &format!("{home}/out")]);
    }
}
