use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Resolved VLM parameters for a single request (global + per-model overrides merged).
#[derive(Debug, Clone)]
pub struct VlmParams {
    pub max_tokens: u32,
    pub timeout: u32,
    pub temperature: f32,
    pub max_image_edge: u32,
    pub num_ctx: u32,
    pub top_p: f32,
    pub top_k: u32,
    pub repeat_penalty: f32,
    pub prompt: Option<String>,
}

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
    "Concise 1-3 sentence description of this photograph — subject, setting, lighting, mood. Be specific, no preamble:";

/// Default prompt for tags mode.
pub const DEFAULT_TAGS_PROMPT: &str =
    "Descriptive tags for this photograph as JSON. Focus on subject, scene, lighting, mood, colors, style. Return only: {\"tags\": [\"golden hour\", \"silhouette\", \"beach\"]}";

/// Return the default prompt for a given mode.
///
/// In `Both` mode, returns the describe prompt (tags uses its own prompt internally).
pub fn default_prompt_for_mode(mode: DescribeMode) -> &'static str {
    match mode {
        DescribeMode::Describe | DescribeMode::Both => DEFAULT_DESCRIBE_PROMPT,
        DescribeMode::Tags => DEFAULT_TAGS_PROMPT,
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
///
/// In `Both` mode, makes two separate VLM calls (describe + tags) and merges
/// the results, so each call uses the optimal prompt for its task.
pub fn call_vlm_with_mode(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    params: &VlmParams,
    mode: DescribeMode,
    verbosity: crate::Verbosity,
) -> Result<VlmOutput> {
    match mode {
        DescribeMode::Both => {
            // Two separate calls: describe first, then tags
            let desc_raw = call_vlm(endpoint, model, image_base64, prompt, params, verbosity)?;
            let tags_raw = call_vlm(endpoint, model, image_base64, DEFAULT_TAGS_PROMPT, params, verbosity)?;
            let description = Some(desc_raw.trim().to_string());
            let tags = extract_tags_from_json(&tags_raw).unwrap_or_default();
            Ok(VlmOutput { description, tags })
        }
        DescribeMode::Tags => {
            // Tags mode always uses the tags prompt — custom prompt only affects describe
            let raw = call_vlm(endpoint, model, image_base64, DEFAULT_TAGS_PROMPT, params, verbosity)?;
            parse_vlm_output(&raw, mode)
        }
        DescribeMode::Describe => {
            let raw = call_vlm(endpoint, model, image_base64, prompt, params, verbosity)?;
            parse_vlm_output(&raw, mode)
        }
    }
}

/// Parse raw VLM text into structured output based on mode.
///
/// Note: `Both` mode is handled by `call_vlm_with_mode` (two separate calls),
/// so this function only needs to handle `Describe` and `Tags`.
pub fn parse_vlm_output(raw: &str, mode: DescribeMode) -> Result<VlmOutput> {
    match mode {
        DescribeMode::Describe | DescribeMode::Both => Ok(VlmOutput {
            description: Some(raw.to_string()),
            tags: Vec::new(),
        }),
        DescribeMode::Tags => {
            match extract_tags_from_json(raw) {
                Ok(tags) => Ok(VlmOutput { description: None, tags }),
                Err(_) => {
                    eprintln!(
                        "Note: model returned prose instead of JSON tags — \
                         saving as description instead. Try --mode both or a more capable model."
                    );
                    Ok(VlmOutput {
                        description: Some(raw.trim().to_string()),
                        tags: Vec::new(),
                    })
                }
            }
        }
    }
}

/// Extract tags array from a JSON response.
/// Handles: `{"tags": [...]}`, `["tag1", ...]`, markdown-wrapped JSON,
/// and truncated JSON (extracts complete strings only).
fn extract_tags_from_json(raw: &str) -> Result<Vec<String>> {
    let cleaned = strip_markdown_json(raw);

    // Try clean JSON parse first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&cleaned) {
        if let Some(tags) = extract_string_array(&v, "tags") {
            return Ok(dedup_tags(tags));
        }
        // Try bare array
        if let Some(arr) = v.as_array() {
            let tags: Vec<String> = arr.iter().filter_map(|t| t.as_str().map(String::from)).collect();
            if !tags.is_empty() {
                return Ok(dedup_tags(tags));
            }
        }
    }

    // Truncated JSON — extract complete tag strings
    let tags = extract_json_string_array_partial(&cleaned, "tags");
    if !tags.is_empty() {
        return Ok(dedup_tags(tags));
    }

    anyhow::bail!(
        "Model returned prose instead of JSON tags. \
         Try --mode describe, --mode both, or a more capable model. \
         Response: {}",
        &raw[..raw.len().min(200)]
    )
}

/// Deduplicate tags preserving first occurrence order.
fn dedup_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    tags.into_iter()
        .filter(|t| seen.insert(t.to_lowercase()))
        .collect()
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

/// Strip `<think>...</think>` blocks from model output.
///
/// Reasoning models (qwen3, deepseek-r1, etc.) wrap internal chain-of-thought
/// in `<think>` tags. The actual answer follows after the closing tag.
fn strip_think_tags(text: &str) -> &str {
    let trimmed = text.trim();
    // Fast path: no think tags
    if !trimmed.contains("<think>") {
        return trimmed;
    }
    // Find the last </think> and return everything after it
    if let Some(pos) = trimmed.rfind("</think>") {
        return trimmed[pos + 8..].trim();
    }
    // Unclosed <think> — model spent all tokens thinking, no answer produced
    if trimmed.starts_with("<think>") {
        return "";
    }
    trimmed
}

/// Call a VLM endpoint with an image.
///
/// Tries Ollama's native `/api/generate` first (supports `think: false`),
/// falls back to OpenAI-compatible `/v1/chat/completions` on 404.
pub fn call_vlm(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    params: &VlmParams,
    verbosity: crate::Verbosity,
) -> Result<String> {
    if verbosity.verbose {
        eprintln!("  VLM prompt: {prompt}");
    }

    // Try Ollama native endpoint first (properly supports think: false)
    match call_ollama_native(endpoint, model, image_base64, prompt, params, verbosity) {
        Ok(text) => return Ok(text),
        Err(e) => {
            let err_str = format!("{e}");
            if err_str.contains("404") || err_str.contains("not found") {
                if verbosity.verbose {
                    eprintln!("  VLM: /api/generate returned 404, falling back to /v1/chat/completions");
                }
                // Not Ollama — fall back to OpenAI-compatible API
                return call_openai_compatible(
                    endpoint, model, image_base64, prompt, params, verbosity,
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
    params: &VlmParams,
    verbosity: crate::Verbosity,
) -> Result<String> {
    let mut body = serde_json::json!({
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
        "max_tokens": params.max_tokens,
        "temperature": params.temperature,
        "stream": false,
        "think": false
    });

    // Add optional sampling parameters
    if params.top_p > 0.0 {
        body["top_p"] = serde_json::json!(params.top_p);
    }
    if params.repeat_penalty > 0.0 {
        body["repeat_penalty"] = serde_json::json!(params.repeat_penalty);
    }

    let url = format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'));
    let response = curl_post(&url, &body, params.timeout, verbosity)?;

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

    let raw_text = resp
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Strip <think>...</think> blocks (qwen3 and other reasoning models)
    let trimmed = strip_think_tags(raw_text).trim().to_string();

    if !trimmed.is_empty() {
        return Ok(trimmed);
    }

    // Some reasoning models put the answer in reasoning_content when content is empty
    if let Some(reasoning) = resp
        .pointer("/choices/0/message/reasoning_content")
        .and_then(|v| v.as_str())
    {
        let stripped = strip_think_tags(reasoning).trim().to_string();
        if !stripped.is_empty() {
            return Ok(stripped);
        }
    }

    // Truly empty — report error with diagnostics
    let finish = resp
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    anyhow::bail!(
        "VLM returned empty content (finish_reason: {finish}). \
         The model may have failed to process the image — check if \"{model}\" \
         supports vision and is fully loaded (try `ollama ps` to check loaded models)"
    );
}

/// Call Ollama's native /api/generate endpoint.
fn call_ollama_native(
    endpoint: &str,
    model: &str,
    image_base64: &str,
    prompt: &str,
    params: &VlmParams,
    verbosity: crate::Verbosity,
) -> Result<String> {
    let mut options = serde_json::json!({
        "temperature": params.temperature
    });
    if params.num_ctx > 0 {
        options["num_ctx"] = serde_json::json!(params.num_ctx);
    }
    if params.top_p > 0.0 {
        options["top_p"] = serde_json::json!(params.top_p);
    }
    if params.top_k > 0 {
        options["top_k"] = serde_json::json!(params.top_k);
    }
    if params.repeat_penalty > 0.0 {
        options["repeat_penalty"] = serde_json::json!(params.repeat_penalty);
    }

    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "images": [image_base64],
        "stream": false,
        "options": options,
        "think": false
    });

    let url = format!("{}/api/generate", endpoint.trim_end_matches('/'));
    let response = curl_post(&url, &body, params.timeout, verbosity)?;

    let resp: serde_json::Value =
        serde_json::from_str(&response).context("Failed to parse Ollama response as JSON")?;

    if let Some(err) = resp.get("error") {
        let msg = err.as_str().unwrap_or("unknown error");
        anyhow::bail!("Ollama error: {msg}");
    }

    let raw_text = resp
        .get("response")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let snippet = response.chars().take(300).collect::<String>();
            anyhow::anyhow!("Unexpected Ollama response format: no 'response' field\n  Response: {snippet}")
        })?;

    // Strip <think>...</think> blocks (qwen3 and other reasoning models)
    let trimmed = strip_think_tags(raw_text).trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!(
            "VLM returned empty content. \
             The model may have failed to process the image — check if \"{model}\" \
             supports vision and is fully loaded (try `ollama ps` to check loaded models)"
        );
    }

    Ok(trimmed)
}

/// Send a POST request via curl with JSON body on stdin.
fn curl_post(
    url: &str,
    body: &serde_json::Value,
    timeout: u32,
    verbosity: crate::Verbosity,
) -> Result<String> {
    let body_str = serde_json::to_string(body)?;

    if verbosity.debug {
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

    if verbosity.debug {
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

    if verbosity.verbose {
        eprintln!("  VLM: response {} bytes", response.len());
    }

    // Detect HTTP error status from curl output
    if response.starts_with("<!DOCTYPE") || response.starts_with("<html") {
        anyhow::bail!("VLM endpoint returned HTML (404 or error page)");
    }

    Ok(response)
}

/// Check if a VLM endpoint is reachable.
/// Result of checking a VLM endpoint.
pub struct EndpointStatus {
    /// Human-readable status message.
    pub message: String,
    /// Model names available on the server (empty if server doesn't list models).
    pub available_models: Vec<String>,
}

/// Check VLM endpoint connectivity and list available models.
pub fn check_endpoint_status(endpoint: &str, timeout: u32, verbosity: crate::Verbosity) -> Result<EndpointStatus> {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));

    if verbosity.debug {
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
            let names: Vec<String> = models
                .iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect();
            let msg = format!(
                "Connected to {}. {} model(s) available: {}",
                endpoint,
                names.len(),
                names.join(", ")
            );
            return Ok(EndpointStatus { message: msg, available_models: names });
        }
    }

    Ok(EndpointStatus {
        message: format!("Connected to {endpoint}. Server is responding."),
        available_models: Vec::new(),
    })
}

pub fn check_endpoint(endpoint: &str, timeout: u32, verbosity: crate::Verbosity) -> Result<String> {
    check_endpoint_status(endpoint, timeout, verbosity).map(|s| s.message)
}

/// Check if a model name matches any available model on the server.
/// Matches by exact name, by unique base name (e.g. "qwen3-vl" matches "qwen3-vl:8b"
/// only if there's one match), or by prefix (e.g. "qwen2.5vl:3b" matches "qwen2.5vl:3b-fp16").
pub fn find_matching_model(configured: &str, available: &[String]) -> Option<String> {
    // Exact match
    if available.iter().any(|m| m == configured) {
        return Some(configured.to_string());
    }
    // Configured name as prefix — collect all matches
    let prefix_matches: Vec<&String> = available.iter()
        .filter(|m| m.starts_with(configured))
        .collect();
    if prefix_matches.len() == 1 {
        return Some(prefix_matches[0].clone());
    }
    None
}

/// Read an image file, optionally resize, and return its base64 encoding.
///
/// When `max_edge` is `Some(n)` with `n > 0`, images whose longest edge exceeds `n`
/// are resized (preserving aspect ratio) before encoding. This reduces the data sent
/// to the VLM and — more importantly — the time the vision encoder spends processing
/// the image, which can cause timeouts on memory-constrained machines.
pub fn encode_image_base64(path: &std::path::Path, max_edge: Option<u32>) -> Result<String> {
    use std::io::Read;

    // If max_edge is set, try to resize via the `image` crate
    if let Some(max) = max_edge {
        if max > 0 {
            if let Ok(img) = image::open(path) {
                let (w, h) = (img.width(), img.height());
                if w.max(h) > max {
                    let nw = (w as f64 * max as f64 / w.max(h) as f64).round() as u32;
                    let nh = (h as f64 * max as f64 / w.max(h) as f64).round() as u32;
                    let resized = image::DynamicImage::ImageRgba8(
                        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3),
                    );
                    let mut buf = std::io::Cursor::new(Vec::new());
                    resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
                    return Ok(base64_encode(buf.get_ref()));
                }
            }
            // Fall through to raw read if image crate can't open (e.g. unsupported format)
        }
    }

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open image: {}", path.display()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
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
    fn test_parse_both_is_plain_text() {
        // Both mode now delegates to two calls in call_vlm_with_mode,
        // so parse_vlm_output with Both just treats raw as description
        let raw = "A butterfly on a pink flower.";
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
    fn test_parse_tags_prose_fallback_to_description() {
        // Model returned prose instead of JSON — should save as description, not fail
        let raw = "A young woman with long, flowing blonde hair looks upward with a contemplative expression.";
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert!(output.tags.is_empty());
        assert_eq!(output.description, Some(raw.trim().to_string()));
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
    fn test_parse_tags_truncated_json() {
        // Truncated JSON from hitting max_tokens — should extract complete tags
        let raw = r#"{"tags": ["golden hour", "silhouette", "beach", "flowers", "butterfly", "wildfl"#;
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert_eq!(output.tags, vec!["golden hour", "silhouette", "beach", "flowers", "butterfly"]);
        assert!(output.description.is_none());
    }

    #[test]
    fn test_parse_tags_dedup() {
        // VLMs sometimes repeat tags — dedup in tags mode
        let raw = r#"{"tags": ["wildflowers", "pink", "wildflowers", "Pink", "butterfly"]}"#;
        let output = parse_vlm_output(raw, DescribeMode::Tags).unwrap();
        assert_eq!(output.tags, vec!["wildflowers", "pink", "butterfly"]);
    }

    #[test]
    fn test_dedup_tags() {
        assert_eq!(dedup_tags(vec![]), Vec::<String>::new());
        assert_eq!(
            dedup_tags(vec!["a".into(), "B".into(), "A".into(), "b".into(), "c".into()]),
            vec!["a", "B", "c"]
        );
    }

    #[test]
    fn test_find_matching_model_exact() {
        let available = vec!["qwen2.5vl:3b".into(), "moondream:latest".into()];
        assert_eq!(
            find_matching_model("qwen2.5vl:3b", &available),
            Some("qwen2.5vl:3b".into())
        );
    }

    #[test]
    fn test_find_matching_model_prefix() {
        let available = vec!["qwen2.5vl:3b".into(), "qwen2.5vl:7b".into()];
        // Ambiguous prefix — two matches, no result
        assert_eq!(find_matching_model("qwen2.5vl", &available), None);
    }

    #[test]
    fn test_find_matching_model_unique_prefix() {
        let available = vec!["qwen3-vl:8b".into(), "moondream:latest".into()];
        assert_eq!(
            find_matching_model("qwen3-vl", &available),
            Some("qwen3-vl:8b".into())
        );
    }

    #[test]
    fn test_find_matching_model_starts_with() {
        let available = vec!["qwen2.5vl:3b-fp16".into()];
        assert_eq!(
            find_matching_model("qwen2.5vl:3b", &available),
            Some("qwen2.5vl:3b-fp16".into())
        );
    }

    #[test]
    fn test_find_matching_model_not_found() {
        let available = vec!["moondream:latest".into()];
        assert_eq!(find_matching_model("qwen2.5vl:3b", &available), None);
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        assert_eq!(strip_think_tags("Hello world"), "Hello world");
    }

    #[test]
    fn test_strip_think_tags_with_answer() {
        let input = "<think>\nLet me analyze this image...\n</think>\nA photo of a sunset.";
        assert_eq!(strip_think_tags(input), "A photo of a sunset.");
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "<think>\nStill thinking about this very complex image...";
        assert_eq!(strip_think_tags(input), "");
    }

    #[test]
    fn test_strip_think_tags_multiple() {
        let input = "<think>first</think>middle<think>second</think>The answer.";
        assert_eq!(strip_think_tags(input), "The answer.");
    }

    #[test]
    fn test_strip_think_tags_whitespace() {
        let input = "  <think>reasoning</think>  \n  A description.  ";
        assert_eq!(strip_think_tags(input), "A description.");
    }
}
