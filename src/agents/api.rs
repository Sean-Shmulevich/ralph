use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use super::{Agent, AgentProcess};

/// API-based agent that calls the Anthropic Messages API directly via curl.
///
/// Works with:
/// - The real Anthropic API (`https://api.anthropic.com`)
/// - opencode-claude-max-proxy (`http://localhost:3456`)
/// - Any Anthropic-compatible endpoint
///
/// Uses streaming SSE so Ralph can still detect stalls and completion tokens
/// from the curl stdout, just like CLI agents.
pub struct ApiAgent {
    base_url: String,
    api_key: String,
    model: String,
}

impl ApiAgent {
    pub fn new(base_url: Option<String>, api_key: Option<String>, model: Option<String>) -> Result<Self> {
        let api_key = api_key
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .context(
                "API agent requires ANTHROPIC_API_KEY env var or --api-key flag.\n\
                 For Claude Max proxy: set any value (e.g. ANTHROPIC_API_KEY=max)"
            )?;

        let base_url = base_url
            .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());

        let model = model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

        Ok(Self {
            base_url,
            api_key,
            model,
        })
    }
}

impl Agent for ApiAgent {
    fn is_available(&self) -> bool {
        // curl is available on basically every system
        std::process::Command::new("which")
            .arg("curl")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn spawn(&self, prompt: &str, workdir: &Path) -> Result<AgentProcess> {
        // Build the Anthropic Messages API request body.
        // We use streaming so Ralph can read incremental output and detect stalls.
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 16384,
            "stream": true,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });

        let body_str = serde_json::to_string(&body)
            .context("Failed to serialize API request body")?;

        // Use a shell script that:
        // 1. Calls curl with streaming SSE
        // 2. Pipes through a simple awk/sed to extract text deltas from SSE events
        // 3. Outputs plain text that Ralph can read like any other agent
        //
        // The SSE events look like:
        //   event: content_block_delta
        //   data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}
        //
        // We extract the "text" field from text_delta events and print it.
        let url = format!("{}/v1/messages", self.base_url);

        let script = format!(
            r#"curl -sN \
  -H "Content-Type: application/json" \
  -H "x-api-key: {api_key}" \
  -H "anthropic-version: 2023-06-01" \
  -d '{body}' \
  "{url}" | while IFS= read -r line; do
    case "$line" in
      data:*)
        json="${{line#data: }}"
        # Extract text from text_delta events using grep+sed (no jq dependency)
        text=$(printf '%s' "$json" | grep -o '"text":"[^"]*"' | head -1 | sed 's/"text":"//;s/"$//')
        if [ -n "$text" ]; then
          # Unescape basic JSON escapes
          printf '%b' "$text"
        fi
        # Check for error
        if printf '%s' "$json" | grep -q '"type":"error"'; then
          printf '%s' "$json" | grep -o '"message":"[^"]*"' | sed 's/"message":"//;s/"$//' >&2
        fi
        ;;
    esac
  done
  echo"#,
            api_key = self.api_key,
            body = body_str.replace('\'', "'\\''"),
            url = url,
        );

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&script)
            .current_dir(workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .context("Failed to spawn curl for API agent")?;

        Ok(AgentProcess { child })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_agent_requires_key() {
        // Temporarily clear the env var
        let old = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let result = ApiAgent::new(None, None, None);
        assert!(result.is_err());

        // Restore
        if let Some(key) = old {
            std::env::set_var("ANTHROPIC_API_KEY", key);
        }
    }

    #[test]
    fn api_agent_with_explicit_key() {
        let agent = ApiAgent::new(
            Some("http://localhost:3456".to_string()),
            Some("test-key".to_string()),
            Some("claude-sonnet-4-20250514".to_string()),
        );
        assert!(agent.is_ok());
        assert!(agent.unwrap().is_available());
    }

    #[test]
    fn api_agent_defaults() {
        let agent = ApiAgent::new(None, Some("key".to_string()), None).unwrap();
        assert_eq!(agent.base_url, "https://api.anthropic.com");
        assert_eq!(agent.model, "claude-sonnet-4-20250514");
    }
}
