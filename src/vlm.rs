use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Describe mode: what to generate from the VLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescribeMode {
    Describe,
    Tags,
    Both,
}

impl DescribeMode {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "describe" => Ok(Self::Describe),
            "tags" => Ok(Self::Tags),
            "both" => Ok(Self::Both),
            _ => anyhow::bail!("Invalid mode '{s}'. Valid modes: describe, tags, both"),
        }
    }
}

impl std::fmt::Display for DescribeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Describe => write!(f, "describe"),
            Self::Tags => write!(f, "tags"),
            Self::Both => write!(f, "both"),
        }
    }
}

/// Default prompt for describe mode.
pub const DEFAULT_DESCRIBE_PROMPT: &str =
    "Describe this photograph in 1-3 concise sentences. Focus on the subject, setting, lighting, and mood. Be specific about what you see, not what you interpret.";

/// Default prompt for tags mode.
pub const DEFAULT_TAGS_PROMPT: &str =
    "Suggest descriptive tags for this photograph. Return a JSON object with a single key \"tags\" containing an array of short, specific tag strings. Focus on subject, scene type, lighting, mood, colors, and photographic style. Example: {\"tags\": [\"golden hour\", \"silhouette\", \"beach\"]}";

/// Default prompt for both mode.
pub const DEFAULT_BOTH_PROMPT: &str =
    "Analyze this photograph. Return a JSON object with two keys: \"description\" (1-3 concise sentences about the subject, setting, lighting, and mood) and \"tags\" (an array of short, specific tag strings covering subject, scene, lighting, mood, colors, and style). Example: {\"description\": \"A lone tree on a hilltop at sunset.\", \"tags\": [\"golden hour\", \"silhouette\", \"landscape\"]}";

/// Return the default prompt for a given mode.
pub fn default_prompt_for_mode(mode: DescribeMode) -> &'static str {
    match mode {
        DescribeMode::Describe => DEFAULT_DESCRIBE_PROMPT,
        DescribeMode::Tags => DEFAULT_TAGS_PROMPT,
        DescribeMode::Both => DEFAULT_BOTH_PROMPT,
    }
}

/// Output of a VLM call, parsed per mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlmOutput {
    pub description: Option<String>,
    pub tags: Vec<String>,
}

/// Result of a single VLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeResult {
    pub asset_id: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub status: DescribeStatus,
}

/// Status of a single describe operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescribeStatus {
    Described,
    Skipped(String),
    Error(String),
}

/// Aggregate result for batch describe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDescribeResult {
    pub described: usize,
    pub skipped: usize,
    pub failed: usize,
    pub tags_applied: usize,
    pub errors: Vec<String>,
    pub dry_run: bool,
    pub mode: String,
    pub results: Vec<DescribeResult>,
}

/// Call a VLM endpoint with an image and parse output according to mode.
pub fn call_vlm_with_mode(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    max_tokens: u32,
    timeout: u32,
    mode: DescribeMode,
    debug: bool,
) -> Result<VlmOutput> {
    let raw = call_vlm(endpoint, model, image_base64, prompt, max_tokens, timeout, debug)?;
    parse_vlm_output(&raw, mode)
}

/// Parse raw VLM text into structured output based on mode.
pub fn parse_vlm_output(raw: &str, mode: DescribeMode) -> Result<VlmOutput> {
    match mode {
        DescribeMode::Describe => Ok(VlmOutput {
            description: Some(raw.to_string()),
            tags: Vec::new(),
        }),
        DescribeMode::Tags => {
            let tags = extract_tags_from_json(raw)?;
            Ok(VlmOutput {
                description: None,
                tags,
            })
        }
        DescribeMode::Both => {
            // Try to parse as JSON with both fields
            if let Some(parsed) = try_parse_both(raw) {
                return Ok(parsed);
            }
            // Fallback: only treat as description if it doesn't look like JSON
            let trimmed = raw.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                anyhow::bail!("Could not parse JSON response from VLM in 'both' mode. Try increasing --max-tokens or using --mode describe instead.");
            }
            Ok(VlmOutput {
                description: Some(raw.to_string()),
                tags: Vec::new(),
            })
        }
    }
}

/// Extract tags array from a JSON response.
/// Handles: `{"tags": [...]}`, `["tag1", ...]`, or markdown-wrapped JSON.
fn extract_tags_from_json(raw: &str) -> Result<Vec<String>> {
    let cleaned = strip_markdown_json(raw);

    // Try {"tags": [...]}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&cleaned) {
        if let Some(tags) = extract_string_array(&v, "tags") {
            return Ok(tags);
        }
        // Try bare array
        if let Some(arr) = v.as_array() {
            let tags: Vec<String> = arr.iter().filter_map(|t| t.as_str().map(String::from)).collect();
            if !tags.is_empty() {
                return Ok(tags);
            }
        }
    }

    anyhow::bail!("Could not parse tags from VLM response: {}", &raw[..raw.len().min(200)])
}

/// Try to parse a "both" mode response: {"description": "...", "tags": [...]}
/// Handles truncated JSON from hitting max_tokens.
fn try_parse_both(raw: &str) -> Option<VlmOutput> {
    let cleaned = strip_markdown_json(&raw);

    // Try clean JSON parse first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&cleaned) {
        let description = v.get("description").and_then(|d| d.as_str()).map(|s| s.trim().to_string());
        let tags = extract_string_array(&v, "tags").unwrap_or_default();
        if description.is_some() || !tags.is_empty() {
            return Some(VlmOutput { description, tags });
        }
    }

    // Truncated JSON — extract what we can via string matching
    let description = extract_json_string_field(&cleaned, "description");
    let tags = extract_json_string_array_partial(&cleaned, "tags");

    if description.is_some() || !tags.is_empty() {
        Some(VlmOutput { description, tags })
    } else {
        None
    }
}

/// Extract a string field value from possibly-truncated JSON.
/// Looks for `"key": "value"` or `"key":"value"`.
fn extract_json_string_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let key_pos = json.find(&pattern)?;
    let after_key = &json[key_pos + pattern.len()..];
    // Skip whitespace and colon
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_ws = after_colon.trim_start();
    // Expect opening quote
    let after_quote = after_ws.strip_prefix('"')?;
    // Find closing quote (handle escaped quotes)
    let mut chars = after_quote.chars();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                match escaped {
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    'n' => value.push('\n'),
                    _ => { value.push('\\'); value.push(escaped); }
                }
            }
        } else if c == '"' {
            return Some(value);
        } else {
            value.push(c);
        }
    }
    // Truncated — return what we have if non-empty
    if !value.is_empty() { Some(value) } else { None }
}

/// Extract string array elements from possibly-truncated JSON.
/// Looks for `"key": ["a", "b", ...]` and collects complete strings.
fn extract_json_string_array_partial(json: &str, key: &str) -> Vec<String> {
    let pattern = format!("\"{}\"", key);
    let key_pos = match json.find(&pattern) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let after_key = &json[key_pos + pattern.len()..];
    let after_colon = match after_key.trim_start().strip_prefix(':') {
        Some(s) => s,
        None => return Vec::new(),
    };
    let after_ws = after_colon.trim_start();
    let after_bracket = match after_ws.strip_prefix('[') {
        Some(s) => s,
        None => return Vec::new(),
    };

    // Extract complete quoted strings
    let mut tags = Vec::new();
    let mut rest = after_bracket;
    loop {
        rest = rest.trim_start();
        if rest.starts_with(']') {
            break;
        }
        if rest.starts_with(',') {
            rest = &rest[1..];
            continue;
        }
        if rest.starts_with('"') {
            rest = &rest[1..];
            if let Some(end) = rest.find('"') {
                let tag = rest[..end].trim().to_string();
                if !tag.is_empty() {
                    tags.push(tag);
                }
                rest = &rest[end + 1..];
            } else {
                break; // Truncated inside a string
            }
        } else {
            break; // Unexpected content
        }
    }
    tags
}

/// Extract a string array from a JSON value by key.
fn extract_string_array(v: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    v.get(key)
        .and_then(|arr| arr.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .filter(|tags: &Vec<String>| !tags.is_empty())
}

/// Strip markdown code fences from JSON responses.
/// Many VLMs wrap JSON in ```json ... ``` blocks.
fn strip_markdown_json(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Call a VLM endpoint with an image.
///
/// Tries OpenAI-compatible `/v1/chat/completions` first, falls back to
/// Ollama's native `/api/generate` on 404.
pub fn call_vlm(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    max_tokens: u32,
    timeout: u32,
    debug: bool,
) -> Result<String> {
    // Try OpenAI-compatible endpoint first
    match call_openai_compatible(endpoint, model, image_base64, prompt, max_tokens, timeout, debug)
    {
        Ok(text) => return Ok(text),
        Err(e) => {
            let err_str = format!("{e}");
            if err_str.contains("404") || err_str.contains("not found") {
                if debug {
                    eprintln!("  [debug] /v1/chat/completions returned 404, falling back to /api/generate");
                }
                // Fall back to Ollama native API
                return call_ollama_native(
                    endpoint,
                    model,
                    image_base64,
                    prompt,
                    max_tokens,
                    timeout,
                    debug,
                );
            }
            return Err(e);
        }
    }
}

/// Call the OpenAI-compatible /v1/chat/completions endpoint.
fn call_openai_compatible(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    max_tokens: u32,
    timeout: u32,
    debug: bool,
) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:image/jpeg;base64,{image_base64}")
                    }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }],
        "max_tokens": max_tokens,
        "temperature": 0.3,
        "stream": false
    });

    let url = format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'));
    let response = curl_post(&url, &body, timeout, debug)?;

    // Parse OpenAI response format
    let resp: serde_json::Value =
        serde_json::from_str(&response).context("Failed to parse VLM response as JSON")?;

    // Check for error response
    if let Some(err) = resp.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("VLM error: {msg}");
    }

    let text = resp
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Unexpected VLM response format: no choices[0].message.content"))?;

    Ok(text.trim().to_string())
}

/// Call Ollama's native /api/generate endpoint.
fn call_ollama_native(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    _max_tokens: u32,
    timeout: u32,
    debug: bool,
) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "images": [image_base64],
        "stream": false
    });

    let url = format!("{}/api/generate", endpoint.trim_end_matches('/'));
    let response = curl_post(&url, &body, timeout, debug)?;

    let resp: serde_json::Value =
        serde_json::from_str(&response).context("Failed to parse Ollama response as JSON")?;

    if let Some(err) = resp.get("error") {
        let msg = err.as_str().unwrap_or("unknown error");
        anyhow::bail!("Ollama error: {msg}");
    }

    let text = resp
        .get("response")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Unexpected Ollama response format: no 'response' field"))?;

    Ok(text.trim().to_string())
}

/// Send a POST request via curl with JSON body on stdin.
fn curl_post(
    url: &str,
    body: &serde_json::Value,
    timeout: u32,
    debug: bool,
) -> Result<String> {
    let body_str = serde_json::to_string(body)?;

    if debug {
        eprintln!("  [debug] POST {url} (body: {} bytes)", body_str.len());
    }

    let mut child = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            url,
            "-H",
            "Content-Type: application/json",
            "-d",
            "@-",
            "--max-time",
            &timeout.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run curl. Is curl installed?")?;

    // Write body to stdin
    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(body_str.as_bytes())
            .context("Failed to write to curl stdin")?;
    }
    // Drop stdin to signal EOF
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .context("Failed to wait for curl")?;

    if debug {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            eprintln!("  [debug] curl stderr: {stderr}");
        }
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Detect connection refused (common: Ollama not running)
        if stderr.contains("Connection refused") || stderr.contains("couldn't connect") {
            anyhow::bail!(
                "VLM server not reachable at {url}. Start Ollama with `ollama serve` or check your endpoint configuration."
            );
        }
        // Detect timeout
        if stderr.contains("timed out") || stderr.contains("Operation timeout") {
            anyhow::bail!("VLM request timed out after {timeout}s");
        }
        anyhow::bail!("curl failed (exit {}): {}{}", output.status, stderr, stdout);
    }

    let response = String::from_utf8(output.stdout)
        .context("VLM response is not valid UTF-8")?;

    // Detect HTTP error status from curl output
    if response.starts_with("<!DOCTYPE") || response.starts_with("<html") {
        anyhow::bail!("VLM endpoint returned HTML (404 or error page)");
    }

    Ok(response)
}

/// Check if a VLM endpoint is reachable.
pub fn check_endpoint(endpoint: &str, timeout: u32, debug: bool) -> Result<String> {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));

    if debug {
        eprintln!("  [debug] GET {url}");
    }

    let output = Command::new("curl")
        .args(["-sS", "--max-time", &timeout.to_string(), &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to run curl")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Connection refused") || stderr.contains("couldn't connect") {
            anyhow::bail!(
                "VLM server not reachable at {}. Start Ollama with `ollama serve`.",
                endpoint
            );
        }
        anyhow::bail!("curl failed: {}", stderr);
    }

    let response = String::from_utf8_lossy(&output.stdout);

    // Try to parse as Ollama /api/tags response
    if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&response) {
        if let Some(models) = resp.get("models").and_then(|m| m.as_array()) {
            let names: Vec<&str> = models
                .iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .collect();
            return Ok(format!(
                "Connected to {}. {} model(s) available: {}",
                endpoint,
                names.len(),
                names.join(", ")
            ));
        }
    }

    Ok(format!("Connected to {endpoint}. Server is responding."))
}

/// Read an image file and return its base64 encoding.
pub fn encode_image_base64(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open image: {}", path.display()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    // Use a simple base64 encoder (no extra dependency)
    Ok(base64_encode(&buf))
}

/// Simple base64 encoder (avoids adding a base64 crate dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_base64_encode_hello() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn test_base64_encode_hello_world() {
        assert_eq!(base64_encode(b"Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }

    #[test]
    fn test_base64_encode_three_bytes() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn test_base64_encode_one_byte() {
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn test_base64_encode_two_bytes() {
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn test_default_describe_prompt() {
        assert!(DEFAULT_DESCRIBE_PROMPT.contains("photograph"));
    }

    #[test]
    fn test_describe_mode_from_str() {
        assert_eq!(DescribeMode::from_str("describe").unwrap(), DescribeMode::Describe);
        assert_eq!(DescribeMode::from_str("tags").unwrap(), DescribeMode::Tags);
        assert_eq!(DescribeMode::from_str("both").unwrap(), DescribeMode::Both);
        assert_eq!(DescribeMode::from_str("TAGS").unwrap(), DescribeMode::Tags);
        assert!(DescribeMode::from_str("invalid").is_err());
    }

    #[test]
    fn test_parse_describe_mode() {
        let output = parse_vlm_output("A sunset over the ocean.", DescribeMode::Describe).unwrap();
        assert_eq!(output.description.unwrap(), "A sunset over the ocean.");
        assert!(output.tags.is_empty());
    }

    #[test]
    fn test_parse_tags_json_object() {
        let raw = r#"{"tags": ["golden hour", "beach", "sunset"]}"#;
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert!(output.description.is_none());
        assert_eq!(output.tags, vec!["golden hour", "beach", "sunset"]);
    }

    #[test]
    fn test_parse_tags_bare_array() {
        let raw = r#"["golden hour", "beach", "sunset"]"#;
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert_eq!(output.tags, vec!["golden hour", "beach", "sunset"]);
    }

    #[test]
    fn test_parse_tags_markdown_wrapped() {
        let raw = "```json\n{\"tags\": [\"landscape\", \"mountains\"]}\n```";
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert_eq!(output.tags, vec!["landscape", "mountains"]);
    }

    #[test]
    fn test_parse_both_mode() {
        let raw = r#"{"description": "A sunset at the beach.", "tags": ["sunset", "beach"]}"#;
        let output = parse_vlm_output(raw, DescribeMode::Both).unwrap();
        assert_eq!(output.description.unwrap(), "A sunset at the beach.");
        assert_eq!(output.tags, vec!["sunset", "beach"]);
    }

    #[test]
    fn test_parse_both_fallback_to_description() {
        let raw = "Just a plain text response without JSON.";
        let output = parse_vlm_output(raw, DescribeMode::Both).unwrap();
        assert_eq!(output.description.unwrap(), raw);
        assert!(output.tags.is_empty());
    }

    #[test]
    fn test_strip_markdown_json() {
        assert_eq!(strip_markdown_json("```json\n{}\n```"), "{}");
        assert_eq!(strip_markdown_json("```\n{}\n```"), "{}");
        assert_eq!(strip_markdown_json("{}"), "{}");
    }

    #[test]
    fn test_parse_tags_invalid_json() {
        let raw = "Here are some tags: sunset, beach, ocean";
        assert!(parse_vlm_output(raw, DescribeMode::Tags).is_err());
    }

    #[test]
    fn test_parse_both_truncated_json() {
        let raw = r#"{"description": "A butterfly on a flower.", "tags": ["butterfly", "flower", "nature", "wildfl"#;
        let output = parse_vlm_output(raw, DescribeMode::Both).unwrap();
        assert_eq!(output.description.unwrap(), "A butterfly on a flower.");
        // "wildfl" is truncated inside quotes, so only complete strings are extracted
        assert_eq!(output.tags, vec!["butterfly", "flower", "nature"]);
    }

    #[test]
    fn test_parse_both_truncated_description_only() {
        let raw = r#"{"description": "A sunset over the oce"#;
        let output = parse_vlm_output(raw, DescribeMode::Both).unwrap();
        assert_eq!(output.description.unwrap(), "A sunset over the oce");
        assert!(output.tags.is_empty());
    }

    #[test]
    fn test_parse_both_unparseable_json_errors() {
        let raw = r#"{"weird_field": 123"#;
        assert!(parse_vlm_output(raw, DescribeMode::Both).is_err());
    }

    #[test]
    fn test_extract_json_string_field() {
        assert_eq!(
            extract_json_string_field(r#"{"description": "hello world", "tags": []}"#, "description"),
            Some("hello world".to_string())
        );
        assert_eq!(
            extract_json_string_field(r#"{"description": "trunca"#, "description"),
            Some("trunca".to_string())
        );
        assert_eq!(
            extract_json_string_field(r#"{"tags": ["a"]}"#, "description"),
            None
        );
    }

    #[test]
    fn test_extract_json_string_array_partial() {
        let tags = extract_json_string_array_partial(
            r#"{"tags": ["a", "b", "c"]}"#, "tags"
        );
        assert_eq!(tags, vec!["a", "b", "c"]);

        let tags = extract_json_string_array_partial(
            r#"{"tags": ["a", "b", "trunc"#, "tags"
        );
        assert_eq!(tags, vec!["a", "b"]);

        let tags = extract_json_string_array_partial(
            r#"{"description": "hi"}"#, "tags"
        );
        assert!(tags.is_empty());
    }

    #[test]
    fn test_both_mode_dedup_tags() {
        // Tags with duplicates — dedup happens at the application layer, not parsing
        let raw = r#"{"description": "A flower.", "tags": ["pink", "flower", "pink", "flower"]}"#;
        let output = parse_vlm_output(raw, DescribeMode::Both).unwrap();
        assert_eq!(output.tags, vec!["pink", "flower", "pink", "flower"]);
    }
}
