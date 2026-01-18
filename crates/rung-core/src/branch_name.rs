//! Branch name validation and newtype.
//!
//! Provides a [`BranchName`] type that enforces git branch name rules
//! and prevents security issues like path traversal and shell injection.
//!
//! Also provides [`slugify`] to convert arbitrary text into a valid branch name.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

/// A validated git branch name.
///
/// This newtype ensures branch names are valid according to git's rules
/// and don't contain dangerous characters that could enable:
/// - Path traversal attacks (`../`)
/// - Shell injection (`$`, `;`, `|`, etc.)
///
/// # Examples
///
/// ```
/// use rung_core::BranchName;
///
/// // Valid branch names
/// let name = BranchName::new("feature/auth").unwrap();
/// let name = BranchName::new("fix-bug-123").unwrap();
///
/// // Invalid branch names
/// assert!(BranchName::new("../etc/passwd").is_err());
/// assert!(BranchName::new("name;rm -rf").is_err());
/// assert!(BranchName::new("branch..name").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Create a new validated branch name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBranchName`] if the name violates git's
    /// branch naming rules or contains dangerous characters.
    pub fn new(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        validate_branch_name(&name)?;
        Ok(Self(name))
    }

    /// Create a branch name by slugifying a commit message.
    ///
    /// Takes the first line of the message, converts to lowercase,
    /// and replaces non-alphanumeric characters with hyphens.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBranchName`] if the slugified result is invalid
    /// (e.g., empty message).
    ///
    /// # Examples
    ///
    /// ```
    /// use rung_core::BranchName;
    ///
    /// let name = BranchName::from_message("feat: add authentication").unwrap();
    /// assert_eq!(name.as_str(), "feat-add-authentication");
    /// ```
    pub fn from_message(message: &str) -> Result<Self, Error> {
        let slugified = slugify(message);

        if slugified.is_empty() {
            return Err(Error::InvalidBranchName {
                name: message.to_string(),
                reason: "message contains no alphanumeric characters".to_string(),
            });
        }

        Self::new(slugified)
    }

    /// Get the branch name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the `BranchName` and return the inner `String`.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for BranchName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for BranchName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<str> for BranchName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for BranchName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for BranchName {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl Serialize for BranchName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BranchName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// Validate a branch name against git rules and security constraints.
fn validate_branch_name(name: &str) -> Result<(), Error> {
    // Empty name
    if name.is_empty() {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot be empty".to_string(),
        });
    }

    // Single @ is not allowed
    if name == "@" {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot be '@'".to_string(),
        });
    }

    // Cannot start with a dot
    if name.starts_with('.') {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot start with '.'".to_string(),
        });
    }

    // Cannot end with a dot
    if name.ends_with('.') {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot end with '.'".to_string(),
        });
    }

    // Cannot end with .lock (git's rule is case-sensitive)
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if name.ends_with(".lock") {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot end with '.lock'".to_string(),
        });
    }

    // Cannot start or end with a slash
    if name.starts_with('/') || name.ends_with('/') {
        return Err(Error::InvalidBranchName {
            name: name.to_string(),
            reason: "branch name cannot start or end with '/'".to_string(),
        });
    }

    // Check for invalid patterns and characters
    for (i, c) in name.chars().enumerate() {
        // Control characters (0x00-0x1f, 0x7f)
        if c.is_ascii_control() {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: "branch name cannot contain control characters".to_string(),
            });
        }

        // Git-forbidden characters: space ~ ^ : ? * [
        if matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[') {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: format!("branch name cannot contain '{c}'"),
            });
        }

        // Shell metacharacters for security: $ ; | & > < ` \ " ' ( ) { } !
        if matches!(
            c,
            '$' | ';'
                | '|'
                | '&'
                | '>'
                | '<'
                | '`'
                | '\\'
                | '"'
                | '\''
                | '('
                | ')'
                | '{'
                | '}'
                | '!'
        ) {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: format!("branch name cannot contain shell metacharacter '{c}'"),
            });
        }

        // Check for consecutive dots (..)
        if c == '.' && name.chars().nth(i + 1) == Some('.') {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: "branch name cannot contain '..'".to_string(),
            });
        }

        // Check for consecutive slashes (//)
        if c == '/' && name.chars().nth(i + 1) == Some('/') {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: "branch name cannot contain '//'".to_string(),
            });
        }

        // Check for @{ sequence
        if c == '@' && name.chars().nth(i + 1) == Some('{') {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: "branch name cannot contain '@{'".to_string(),
            });
        }

        // Check for slash followed by dot (/.component)
        if c == '/' && name.chars().nth(i + 1) == Some('.') {
            return Err(Error::InvalidBranchName {
                name: name.to_string(),
                reason: "branch name component cannot start with '.'".to_string(),
            });
        }
    }

    Ok(())
}

/// Maximum length for generated branch names.
const MAX_BRANCH_NAME_LENGTH: usize = 50;

/// Convert arbitrary text into a git-safe branch name.
///
/// Takes the first line and slugifies it:
/// - Converts to lowercase
/// - Replaces non-alphanumeric characters with hyphens
/// - Removes consecutive/leading/trailing hyphens
/// - Truncates to 50 characters at word boundaries
///
/// # Examples
///
/// ```
/// use rung_core::slugify;
///
/// assert_eq!(slugify("feat: add authentication"), "feat-add-authentication");
/// assert_eq!(slugify("Fix login bug"), "fix-login-bug");
/// assert_eq!(slugify("feat(auth): add OAuth support"), "feat-auth-add-oauth-support");
///
/// // Long messages are truncated at word boundaries
/// let long_msg = "feat: implement very long feature name that exceeds the maximum length";
/// let result = slugify(long_msg);
/// assert!(result.len() <= 50);
/// assert!(!result.ends_with('-'));
/// ```
#[must_use]
pub fn slugify(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text);

    let slug: String = first_line
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    // Truncate at word boundary if too long (character-safe for UTF-8)
    if slug.chars().count() <= MAX_BRANCH_NAME_LENGTH {
        return slug;
    }

    // Find last hyphen within the character limit
    // Track both character count and byte index for proper UTF-8 slicing
    let mut last_hyphen_byte_pos = None;
    for (char_count, (byte_pos, c)) in slug.char_indices().enumerate() {
        if char_count >= MAX_BRANCH_NAME_LENGTH {
            break;
        }
        if c == '-' {
            last_hyphen_byte_pos = Some(byte_pos);
        }
    }

    last_hyphen_byte_pos.map_or_else(
        || slug.chars().take(MAX_BRANCH_NAME_LENGTH).collect(),
        |pos| slug[..pos].to_string(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_branch_names() {
        // Simple names
        assert!(BranchName::new("main").is_ok());
        assert!(BranchName::new("master").is_ok());
        assert!(BranchName::new("develop").is_ok());

        // With slashes (hierarchical)
        assert!(BranchName::new("feature/auth").is_ok());
        assert!(BranchName::new("feature/user/login").is_ok());
        assert!(BranchName::new("fix/bug-123").is_ok());

        // With dashes and underscores
        assert!(BranchName::new("my-feature").is_ok());
        assert!(BranchName::new("my_feature").is_ok());
        assert!(BranchName::new("feature-123-fix").is_ok());

        // With numbers
        assert!(BranchName::new("v1.0.0").is_ok());
        assert!(BranchName::new("release-2024-01").is_ok());

        // With @ (not followed by {)
        assert!(BranchName::new("user@feature").is_ok());
    }

    #[test]
    fn test_empty_name() {
        let err = BranchName::new("").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_single_at() {
        let err = BranchName::new("@").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_starts_with_dot() {
        let err = BranchName::new(".hidden").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_ends_with_dot() {
        let err = BranchName::new("branch.").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_ends_with_lock() {
        let err = BranchName::new("branch.lock").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_consecutive_dots() {
        let err = BranchName::new("branch..name").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Path traversal attempt
        let err = BranchName::new("../etc/passwd").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_slash_rules() {
        // Starts with slash
        let err = BranchName::new("/branch").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Ends with slash
        let err = BranchName::new("branch/").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Consecutive slashes
        let err = BranchName::new("feature//auth").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Slash followed by dot
        let err = BranchName::new("feature/.hidden").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_git_forbidden_characters() {
        for c in [' ', '~', '^', ':', '?', '*', '['] {
            let name = format!("branch{c}name");
            let err = BranchName::new(&name).unwrap_err();
            assert!(matches!(err, Error::InvalidBranchName { .. }), "char: {c}");
        }
    }

    #[test]
    fn test_shell_metacharacters() {
        for c in [
            '$', ';', '|', '&', '>', '<', '`', '\\', '"', '\'', '(', ')', '{', '}', '!',
        ] {
            let name = format!("branch{c}name");
            let err = BranchName::new(&name).unwrap_err();
            assert!(matches!(err, Error::InvalidBranchName { .. }), "char: {c}");
        }
    }

    #[test]
    fn test_shell_injection_attempts() {
        // Command substitution
        let err = BranchName::new("branch$(whoami)").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Command chaining
        let err = BranchName::new("branch;rm -rf /").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        // Pipe
        let err = BranchName::new("branch|cat /etc/passwd").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_at_brace_sequence() {
        let err = BranchName::new("branch@{1}").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_control_characters() {
        let err = BranchName::new("branch\x00name").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        let err = BranchName::new("branch\tname").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));

        let err = BranchName::new("branch\nname").unwrap_err();
        assert!(matches!(err, Error::InvalidBranchName { .. }));
    }

    #[test]
    fn test_display_and_deref() {
        let name = BranchName::new("feature/auth").unwrap();
        assert_eq!(format!("{name}"), "feature/auth");
        assert_eq!(name.as_str(), "feature/auth");
        assert_eq!(&*name, "feature/auth");
    }

    #[test]
    fn test_serialize_deserialize() {
        let name = BranchName::new("feature/auth").unwrap();

        // Serialize
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"feature/auth\"");

        // Deserialize valid
        let parsed: BranchName = serde_json::from_str("\"feature/test\"").unwrap();
        assert_eq!(parsed.as_str(), "feature/test");

        // Deserialize invalid should fail
        let result: Result<BranchName, _> = serde_json::from_str("\"..invalid\"");
        assert!(result.is_err());
    }

    // === slugify tests ===

    #[test]
    fn test_slugify_basic() {
        assert_eq!(
            slugify("feat: add authentication"),
            "feat-add-authentication"
        );
        assert_eq!(slugify("Fix login bug"), "fix-login-bug");
        assert_eq!(
            slugify("feat(auth): add OAuth support"),
            "feat-auth-add-oauth-support"
        );
    }

    #[test]
    fn test_slugify_whitespace_only() {
        assert_eq!(slugify("   "), "");
        assert_eq!(slugify("\t\n"), "");
    }

    #[test]
    fn test_slugify_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn test_slugify_emoji_only() {
        assert_eq!(slugify("🔥🚀"), "");
        assert_eq!(slugify("✨ ⭐ 💫"), "");
    }

    #[test]
    fn test_slugify_multiline() {
        // Only first line should be used
        assert_eq!(
            slugify("feat: add auth\n\nThis is a longer description"),
            "feat-add-auth"
        );
        assert_eq!(slugify("fix: bug\nSecond line\nThird line"), "fix-bug");
    }

    #[test]
    fn test_slugify_truncation() {
        // 50 char limit at word boundary
        let long_msg =
            "feat: implement very long feature name that exceeds the maximum length allowed";
        let result = slugify(long_msg);
        assert!(result.chars().count() <= 50);
        assert!(!result.ends_with('-'));
        // Should truncate at word boundary
        assert_eq!(result, "feat-implement-very-long-feature-name-that");
    }

    #[test]
    fn test_slugify_truncation_no_hyphen() {
        // Very long single word should just truncate at char limit
        let long_word = "a".repeat(60);
        let result = slugify(&long_word);
        assert_eq!(result.chars().count(), 50);
    }

    #[test]
    fn test_slugify_unicode() {
        // Accented characters (alphanumeric in Unicode)
        assert_eq!(slugify("café feature"), "café-feature");
        assert_eq!(slugify("naïve implementation"), "naïve-implementation");

        // CJK characters
        assert_eq!(slugify("新功能 feature"), "新功能-feature");

        // Mixed unicode and ASCII
        assert_eq!(slugify("über cool änderung"), "über-cool-änderung");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("fix: bug #123"), "fix-bug-123");
        assert_eq!(
            slugify("feat(scope): add [feature]"),
            "feat-scope-add-feature"
        );
        assert_eq!(slugify("fix: path/to/file"), "fix-path-to-file");
    }

    // === from_message tests ===

    #[test]
    fn test_from_message_basic() {
        let name = BranchName::from_message("feat: add authentication").unwrap();
        assert_eq!(name.as_str(), "feat-add-authentication");
    }

    #[test]
    fn test_from_message_empty_error() {
        let result = BranchName::from_message("");
        assert!(result.is_err());
        if let Err(Error::InvalidBranchName { reason, .. }) = result {
            assert!(reason.contains("no alphanumeric"));
        }
    }

    #[test]
    fn test_from_message_whitespace_only_error() {
        let result = BranchName::from_message("   \t\n  ");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_message_emoji_only_error() {
        let result = BranchName::from_message("🔥🚀✨");
        assert!(result.is_err());
        if let Err(Error::InvalidBranchName { reason, .. }) = result {
            assert!(reason.contains("no alphanumeric"));
        }
    }

    #[test]
    fn test_from_message_multiline() {
        let name = BranchName::from_message("feat: add auth\n\nDetailed description here").unwrap();
        assert_eq!(name.as_str(), "feat-add-auth");
    }
}
