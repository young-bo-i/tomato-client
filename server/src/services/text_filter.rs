use regex::Regex;
use std::sync::LazyLock;

static RE_BRACKETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[《》【】\[\]()（）「」『』\{\}<>〈〉]").unwrap()
});
static RE_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[,，。.!！?？;；:：~～、…—\-]").unwrap()
});
static RE_VALID_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{Han}a-zA-Z0-9]{4,6}$").unwrap()
});

/// Text filter pipeline matching original TitleWrapperConvert logic.
/// Returns the filtered word if valid (4-6 chars), or None if invalid.
pub fn filter_title(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }

    // Step 1: Remove brackets and extract inner text
    let text = RE_BRACKETS.replace_all(text, "").to_string();

    // Step 2: Remove punctuation
    let text = RE_PUNCTUATION.replace_all(&text, "").to_string();

    // Step 3: Trim whitespace
    let text = text.trim().to_string();

    // Step 4: Validate length 4-6 characters
    let char_count = text.chars().count();
    if char_count < 4 || char_count > 6 {
        // Try to extract first 4-6 chars
        let truncated: String = text.chars().take(6).collect();
        let trunc_count = truncated.chars().count();
        if trunc_count < 4 {
            return None;
        }
        if RE_VALID_WORD.is_match(&truncated) {
            return Some(truncated);
        }
        return None;
    }

    if RE_VALID_WORD.is_match(&text) {
        Some(text)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_normal() {
        assert_eq!(filter_title("测试小说名"), Some("测试小说名".into()));
    }

    #[test]
    fn test_filter_with_brackets() {
        assert_eq!(filter_title("《测试小说名》"), Some("测试小说名".into()));
    }

    #[test]
    fn test_filter_too_short() {
        assert_eq!(filter_title("短"), None);
    }

    #[test]
    fn test_filter_with_punctuation() {
        assert_eq!(filter_title("测试，小说"), Some("测试小说".into()));
    }
}
