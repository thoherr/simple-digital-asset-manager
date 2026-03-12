use std::path::{Path, PathBuf};

use anyhow::Result;

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

/// Run an interactive REPL session.
fn run_interactive(
    catalog_root: &Path,
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) {
    use rustyline::error::ReadlineError;

    let catalog_name = catalog_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("dam");

    let mut rl = rustyline::DefaultEditor::new().unwrap();

    // Load history (ignore errors — file may not exist yet)
    let history_path = catalog_root.join(".dam").join("shell_history");
    if history_path.exists() {
        let _ = rl.load_history(&history_path);
    }

    let mut last_ids: Vec<String> = Vec::new();

    eprintln!("dam shell v{} — type 'help' or 'quit' to exit", env!("CARGO_PKG_VERSION"));

    loop {
        let prompt = format!("{catalog_name}> ");
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                let _ = rl.add_history_entry(trimmed);

                match handle_line(trimmed, &last_ids, executor) {
                    LineResult::Ok(ids) => {
                        if !ids.is_empty() {
                            last_ids = ids;
                        }
                    }
                    LineResult::Err(msg) => eprintln!("Error: {msg:#}"),
                    LineResult::Quit => break,
                    LineResult::Blocked(cmd) => {
                        eprintln!("'{cmd}' cannot be used inside the shell.");
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

    let mut last_ids: Vec<String> = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match handle_line(trimmed, &last_ids, executor) {
            LineResult::Ok(ids) => {
                if !ids.is_empty() {
                    last_ids = ids;
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
        }
    }
}

enum LineResult {
    Ok(Vec<String>),
    Err(anyhow::Error),
    Quit,
    Blocked(String),
}

/// Process a single shell line.
fn handle_line(
    line: &str,
    last_ids: &[String],
    executor: &impl Fn(Vec<String>) -> Result<Vec<String>>,
) -> LineResult {
    // Built-in commands
    match line {
        "quit" | "exit" => return LineResult::Quit,
        "help" => {
            print_shell_help();
            return LineResult::Ok(Vec::new());
        }
        _ => {}
    }

    // Expand _ to last result IDs
    let expanded = expand_underscore(line, last_ids);

    // Shell-split the line into tokens
    let tokens = match shell_split(&expanded) {
        Some(t) => t,
        None => return LineResult::Err(anyhow::anyhow!("Unmatched quote in command")),
    };

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
fn shell_split(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

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
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
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

Special syntax:
  _              Expands to asset IDs from the last search/command
  # comment      Lines starting with # are ignored

Shell commands:
  help           Show this help
  quit / exit    End the session (also Ctrl-D)

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
    fn test_expand_underscore_in_word() {
        let ids = vec!["abc123".to_string()];
        // _foo should NOT be expanded
        assert_eq!(expand_underscore("search _foo", &ids), "search _foo");
        // foo_ should NOT be expanded
        assert_eq!(expand_underscore("search foo_bar", &ids), "search foo_bar");
    }
}
