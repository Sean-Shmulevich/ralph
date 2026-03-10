use std::time::Duration;

pub fn detect_rate_limit(output: &str) -> Option<Duration> {
    if output.contains("429 Too Many Requests") || output.to_lowercase().contains("rate limit") || output.to_lowercase().contains("usage limit") {
        return Some(Duration::from_secs(60)); // Default 60s for now
    }
    None
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
