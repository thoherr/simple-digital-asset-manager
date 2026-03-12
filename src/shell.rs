use std::collections::HashMap;
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

/// Run the interactive shell or execute a script file.
pub fn run(
    catalog_root: &Path,
    script: Option<PathBuf>,
    executor: impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    if let Some(path) = script {
        run_script(&path, &executor);
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
    "export",
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
const BUILTINS: &[&str] = &["exit", "help", "quit", "unset", "vars"];

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

    /// Refresh completion data from the catalog (used after mutations).
    #[allow(dead_code)]
    fn refresh(&mut self, catalog_root: &Path) {
        let (tags, volumes) = load_completion_data(catalog_root);
        self.tags = tags;
        self.volumes = volumes;
    }
}

/// Load tag names and volume labels from the catalog for completion.
fn load_completion_data(catalog_root: &Path) -> (Vec<String>, Vec<String>) {
    let db_path = catalog_root.join(".dam").join("catalog.db");
    let tags;
    let volumes;

    if let Ok(catalog) = crate::catalog::Catalog::open_fast(&db_path) {
        tags = catalog
            .list_all_tags()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _count)| name)
            .collect();
        volumes = catalog
            .list_volumes()
            .unwrap_or_default()
            .into_iter()
            .map(|(_id, label)| label)
            .collect();
    } else {
        tags = Vec::new();
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
// Variable state
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
        .unwrap_or("dam");

    let helper = ShellHelper::new(catalog_root);
    let mut rl = rustyline::Editor::new().unwrap();
    rl.set_helper(Some(helper));

    // Load history (ignore errors — file may not exist yet)
    let history_path = catalog_root.join(".dam").join("shell_history");
    if history_path.exists() {
        let _ = rl.load_history(&history_path);
    }

    let mut vars = Variables::new();

    eprintln!("dam shell v{} — type 'help' or 'quit' to exit", env!("CARGO_PKG_VERSION"));

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

                match handle_line(trimmed, &mut vars, executor) {
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
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading script {}: {e}", path.display());
            return;
        }
    };

    let mut vars = Variables::new();

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match handle_line(trimmed, &mut vars, executor) {
            LineResult::Ok(ids) => {
                if !ids.is_empty() {
                    vars.last_ids = ids;
                }
            }
            LineResult::Err(msg) => {
                eprintln!("{}:{}: Error: {msg:#}", path.display(), line_num + 1);
            }
            LineResult::Quit => break,
            LineResult::Blocked(cmd) => {
                eprintln!(
                    "{}:{}: '{}' cannot be used in scripts.",
                    path.display(),
                    line_num + 1,
                    cmd
                );
            }
            LineResult::Handled => {}
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
}

/// Process a single shell line.
fn handle_line(
    line: &str,
    vars: &mut Variables,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) -> LineResult {
    // Check for variable assignment: $name = <command...>
    if let Some(rest) = parse_variable_assignment(line) {
        let (var_name, command_part) = rest;
        if command_part.is_empty() {
            // $name = _ (assign last result to named variable)
            // Just assigning _ is handled via expand_variables
            return LineResult::Err(anyhow::anyhow!("No command after variable assignment"));
        }

        // Check if it's just `$name = _` — copy last result
        let expanded = expand_variables(&command_part, vars);
        let tokens = match shell_split(&expanded) {
            Some(t) => t,
            None => return LineResult::Err(anyhow::anyhow!("Unmatched quote in command")),
        };

        // If the expanded result is just IDs (from _ expansion), store directly
        if tokens.len() == 1 && tokens[0].contains(' ') {
            // This was just _ expansion — store the IDs
            vars.named.insert(var_name, vars.last_ids.clone());
            let count = vars.last_ids.len();
            eprintln!("  {count} assets → ${}", vars.named.keys().last().unwrap_or(&String::new()));
            return LineResult::Handled;
        }

        // Otherwise execute the command and store result
        return match execute_tokens(tokens, executor) {
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
            print_vars(vars);
            return LineResult::Handled;
        }
        _ => {}
    }

    // unset $name
    if let Some(rest) = line.strip_prefix("unset ") {
        let name = rest.trim();
        if let Some(name) = name.strip_prefix('$') {
            if vars.named.remove(name).is_some() {
                eprintln!("  Removed ${name}");
            } else {
                eprintln!("  Variable ${name} not defined");
            }
        } else {
            eprintln!("  Usage: unset $name");
        }
        return LineResult::Handled;
    }

    // Expand variables ($name and _)
    let expanded = expand_variables(line, vars);

    // Shell-split the line into tokens
    let tokens = match shell_split(&expanded) {
        Some(t) => t,
        None => return LineResult::Err(anyhow::anyhow!("Unmatched quote in command")),
    };

    execute_tokens(tokens, executor)
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

/// Expand `$name` variables and `_` in the command line.
fn expand_variables(line: &str, vars: &Variables) -> String {
    // First expand $name variables
    let mut result = String::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            // Collect variable name
            let mut name = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_alphanumeric() || nc == '_' {
                    name.push(nc);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Some(ids) = vars.named.get(&name) {
                result.push_str(&ids.join(" "));
            } else if !name.is_empty() {
                // Unknown variable — keep as-is
                result.push('$');
                result.push_str(&name);
            } else {
                result.push('$');
            }
        } else {
            result.push(c);
        }
    }

    // Then expand standalone _ to last IDs
    expand_underscore(&result, &vars.last_ids)
}

/// Execute a parsed token list as a dam command.
fn execute_tokens(
    tokens: Vec<String>,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) -> LineResult {
    if tokens.is_empty() {
        return LineResult::Ok(Vec::new());
    }

    // Block commands that don't make sense in the shell
    let cmd = tokens[0].to_lowercase();
    if matches!(cmd.as_str(), "init" | "migrate" | "serve" | "shell") {
        return LineResult::Blocked(cmd);
    }

    // Prepend "dam" as argv[0] for clap parsing
    let mut args = vec!["dam".to_string()];
    args.extend(tokens);

    match executor(args) {
        Ok(ids) => LineResult::Ok(ids),
        Err(e) => LineResult::Err(e),
    }
}

/// Print all defined variables and their asset counts.
fn print_vars(vars: &Variables) {
    if vars.named.is_empty() && vars.last_ids.is_empty() {
        eprintln!("  No variables defined.");
        return;
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
fn expand_underscore(line: &str, last_ids: &[String]) -> String {
    if !line.contains('_') || last_ids.is_empty() {
        return line.to_string();
    }

    // Only expand standalone _ (not inside words like _foo or foo_bar)
    let mut result = String::new();
    let mut chars = line.chars().peekable();
    let mut prev_is_word = false;

    while let Some(c) = chars.next() {
        if c == '_' && !prev_is_word {
            // Check if next char is also a word char
            let next_is_word = chars.peek().map_or(false, |n| n.is_alphanumeric() || *n == '_');
            if !next_is_word {
                // Standalone _ — expand
                result.push_str(&last_ids.join(" "));
                prev_is_word = true;
                continue;
            }
        }
        prev_is_word = c.is_alphanumeric() || c == '_' || c == '-';
        result.push(c);
    }
    result
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
dam shell — interactive asset management shell

Enter any dam command without the 'dam' prefix:
  search \"tag:landscape rating:4+\"
  edit --rating 5 abc12345
  stats

Variables:
  $name = <command>  Store command results in a named variable
  $name              Expands to stored asset IDs in any command
  _                  Expands to asset IDs from the last command
  vars               List all defined variables
  unset $name        Remove a variable

Examples:
  $picks = search \"rating:5 date:2024\"
  tag --add \"portfolio\" $picks
  export --target /tmp/best $picks

Other syntax:
  # comment          Lines starting with # are ignored

Shell commands:
  help               Show this help
  vars               List defined variables with asset counts
  unset $name        Remove a named variable
  quit / exit        End the session (also Ctrl-D)

Tab completion:
  Subcommand names, --flags, $variables, tag:names, volume:labels

Blocked commands (use outside the shell):
  init, migrate, serve, shell");
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_expand_underscore_standalone() {
        let ids = vec!["abc123".to_string(), "def456".to_string()];
        assert_eq!(expand_underscore("edit --rating 5 _", &ids), "edit --rating 5 abc123 def456");
    }

    #[test]
    fn test_expand_underscore_no_ids() {
        let ids: Vec<String> = vec![];
        assert_eq!(expand_underscore("edit --rating 5 _", &ids), "edit --rating 5 _");
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
    fn test_expand_underscore_in_word() {
        let ids = vec!["abc123".to_string()];
        assert_eq!(expand_underscore("search _foo", &ids), "search _foo");
        assert_eq!(expand_underscore("search foo_bar", &ids), "search foo_bar");
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
        // Not a variable assignment
        assert_eq!(parse_variable_assignment("search tag:landscape"), None);
        // Invalid variable name
        assert_eq!(parse_variable_assignment("$ = search"), None);
        assert_eq!(parse_variable_assignment("$foo-bar = search"), None);
    }

    #[test]
    fn test_expand_variables_named() {
        let mut vars = Variables::new();
        vars.named.insert("picks".to_string(), vec!["id1".to_string(), "id2".to_string()]);
        vars.last_ids = vec!["id3".to_string()];

        assert_eq!(
            expand_variables("tag --add portfolio $picks", &vars),
            "tag --add portfolio id1 id2"
        );
    }

    #[test]
    fn test_expand_variables_underscore() {
        let mut vars = Variables::new();
        vars.last_ids = vec!["id1".to_string(), "id2".to_string()];

        assert_eq!(
            expand_variables("edit --rating 5 _", &vars),
            "edit --rating 5 id1 id2"
        );
    }

    #[test]
    fn test_expand_variables_unknown() {
        let vars = Variables::new();
        assert_eq!(
            expand_variables("tag --add $unknown", &vars),
            "tag --add $unknown"
        );
    }

    #[test]
    fn test_expand_variables_dollar_not_var() {
        let vars = Variables::new();
        // Bare $ not followed by a name is kept as-is
        assert_eq!(expand_variables("echo $ done", &vars), "echo $ done");
    }

    #[test]
    fn test_handle_line_vars_command() {
        let mut vars = Variables::new();
        vars.named.insert("test".to_string(), vec!["id1".to_string()]);
        let executor = |_args: Vec<String>| -> Result<Vec<String>> { Ok(vec![]) };

        match handle_line("vars", &mut vars, &executor) {
            LineResult::Handled => {} // expected
            _ => panic!("Expected Handled for vars command"),
        }
    }

    #[test]
    fn test_handle_line_unset() {
        let mut vars = Variables::new();
        vars.named.insert("test".to_string(), vec!["id1".to_string()]);
        let executor = |_args: Vec<String>| -> Result<Vec<String>> { Ok(vec![]) };

        match handle_line("unset $test", &mut vars, &executor) {
            LineResult::Handled => {} // expected
            _ => panic!("Expected Handled for unset"),
        }
        assert!(!vars.named.contains_key("test"));
    }

    #[test]
    fn test_handle_line_variable_assignment() {
        let mut vars = Variables::new();
        let executor = |_args: Vec<String>| -> Result<Vec<String>> {
            Ok(vec!["a1".to_string(), "a2".to_string()])
        };

        match handle_line("$picks = search rating:5", &mut vars, &executor) {
            LineResult::Handled => {} // expected
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
}
