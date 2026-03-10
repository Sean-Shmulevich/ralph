use std::time::Duration;

/// Detects rate limit errors (HTTP 429, usage limit) in agent output.
///
/// If detected, tries to parse a retry duration from the output.
/// Returns `Some(Duration)` if a rate limit is detected, `None` otherwise.
pub fn detect_rate_limit(output: &str) -> Option<Duration> {
    if output.contains("429") || output.to_lowercase().contains("usage limit") || output.to_lowercase().contains("rate limit") {
        // Try to find explicit "Retry-After" or "resets_in_seconds"
        if let Some(idx) = output.find("Retry-After: ") {
            let remaining = &output[idx + "Retry-After: ".len()..];
            if let Some(newline_idx) = remaining.find(|c: char| !c.is_numeric()) {
                let secs_str = &remaining[..newline_idx];
                if let Ok(secs) = secs_str.parse::<u64>() {
                    return Some(Duration::from_secs(secs));
                }
            }
        } else if let Some(idx) = output.find("resets_in_seconds") {
            let remaining = &output[idx + "resets_in_seconds".len()..];
            if let Some(start_num) = remaining.find(|c: char| c.is_numeric()) {
                 let num_str = &remaining[start_num..];
                 if let Some(end_num) = num_str.find(|c: char| !c.is_numeric()) {
                     let secs_str = &num_str[..end_num];
                     if let Ok(secs) = secs_str.parse::<u64>() {
                        return Some(Duration::from_secs(secs));
                     }
                 } else {
                     if let Ok(secs) = num_str.parse::<u64>() {
                        return Some(Duration::from_secs(secs));
                     }
                 }
            }
        }
        Some(Duration::from_secs(60))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_rate_limit_429() {
        let out = "HTTP 429 Too Many Requests: Please try again later.";
        assert_eq!(detect_rate_limit(out), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_detect_rate_limit_usage() {
        let out = "error: usage limit exceeded for today.";
        assert_eq!(detect_rate_limit(out), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_detect_rate_limit_none() {
        let out = "All tests passed successfully!";
        assert_eq!(detect_rate_limit(out), None);
    }
}
