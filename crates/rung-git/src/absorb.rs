//! Git operations for the absorb command.
//!
//! Provides diff parsing, blame queries, and fixup commit creation.

use crate::Repository;
use crate::error::{Error, Result};
use git2::Oid;

/// A hunk of changes from a staged diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// File path relative to repository root.
    pub file_path: String,
    /// Starting line in the original file (1-indexed, before changes).
    pub old_start: u32,
    /// Number of lines in the original file.
    pub old_lines: u32,
    /// Starting line in the new file (1-indexed, after changes).
    pub new_start: u32,
    /// Number of lines in the new file.
    pub new_lines: u32,
    /// The actual diff content (context + changes).
    pub content: String,
    /// Whether this hunk is from a newly created file.
    pub is_new_file: bool,
}

/// Result of a blame query for a line range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameResult {
    /// The commit that last modified this line range.
    pub commit: Oid,
    /// Short commit message (first line).
    pub message: String,
}

impl Repository {
    /// Get the staged diff as a list of hunks.
    ///
    /// Parses `git diff --cached` output to extract individual hunks
    /// with file paths and line ranges.
    ///
    /// # Errors
    /// Returns error if git diff fails or output cannot be parsed.
    pub fn staged_diff_hunks(&self) -> Result<Vec<Hunk>> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["diff", "--cached", "-U0", "--no-color"])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::Git2(git2::Error::from_str(&e.to_string())))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git2(git2::Error::from_str(&stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_diff_hunks(&stdout))
    }

    /// Query git blame for a specific line range in a file.
    ///
    /// Returns the commits that last modified lines in the given range.
    /// Uses `git blame -L <start>,<end>` for targeted queries.
    ///
    /// # Errors
    /// Returns error if blame fails or commit cannot be found.
    pub fn blame_lines(&self, file_path: &str, start: u32, end: u32) -> Result<Vec<BlameResult>> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        // Use -l for full commit hashes, -s for suppressing author/date
        let line_range = format!("{start},{end}");
        let output = std::process::Command::new("git")
            .args(["blame", "-l", "-s", "-L", &line_range, "--", file_path])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::Git2(git2::Error::from_str(&e.to_string())))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git2(git2::Error::from_str(&stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_blame_output(&stdout)
    }

    /// Create a fixup commit targeting the specified commit.
    ///
    /// Equivalent to `git commit --fixup=<target>`.
    /// Staged changes must exist before calling this.
    ///
    /// # Errors
    /// Returns error if commit creation fails.
    pub fn create_fixup_commit(&self, target: Oid) -> Result<Oid> {
        let workdir = self.workdir().ok_or(Error::NotARepository)?;

        let output = std::process::Command::new("git")
            .args(["commit", "--fixup", &target.to_string()])
            .current_dir(workdir)
            .output()
            .map_err(|e| Error::Git2(git2::Error::from_str(&e.to_string())))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git2(git2::Error::from_str(&stderr)));
        }

        // Get the commit we just created
        let head = self.inner().head()?.peel_to_commit()?;
        Ok(head.id())
    }

    /// Parse blame output into `BlameResult` items.
    fn parse_blame_output(&self, output: &str) -> Result<Vec<BlameResult>> {
        let mut results = Vec::new();
        let mut seen_commits = std::collections::HashSet::new();

        for line in output.lines() {
            // Format: <40-char-sha> <line-num> <content>
            // Boundary commits start with '^': ^<39-char-sha> <line-num> <content>
            // With -s flag, no author/date info

            // Detect and skip boundary commits (start with ^) first
            let (sha, is_boundary) = if line.starts_with('^') {
                // Boundary commit: ^<39-char SHA> (40 chars total including ^)
                if line.len() < 41 {
                    continue;
                }
                (&line[1..41], true)
            } else {
                if line.len() < 40 {
                    continue;
                }
                (&line[..40], false)
            };

            // Skip boundary commits - they're outside the blame range
            if is_boundary {
                continue;
            }

            // Skip if we've already seen this commit
            if seen_commits.contains(sha) {
                continue;
            }
            seen_commits.insert(sha.to_string());

            let oid = Oid::from_str(sha)
                .map_err(|e| Error::Git2(git2::Error::from_str(&e.to_string())))?;

            // Get commit message
            let commit = self.find_commit(oid)?;
            let message = commit
                .message()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();

            results.push(BlameResult {
                commit: oid,
                message,
            });
        }

        Ok(results)
    }

    /// Check if a commit is an ancestor of another commit.
    ///
    /// Returns true if `ancestor` is reachable from `descendant`.
    ///
    /// # Errors
    /// Returns error if the check fails.
    pub fn is_ancestor(&self, ancestor: Oid, descendant: Oid) -> Result<bool> {
        Ok(self.inner().graph_descendant_of(descendant, ancestor)?)
    }
}

/// Parse unified diff output into hunks.
fn parse_diff_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;
    let mut hunk_content = String::new();
    let mut current_hunk: Option<(u32, u32, u32, u32)> = None;
    let mut is_new_file = false;

    for line in diff.lines() {
        // New file header: diff --git a/path b/path
        if line.starts_with("diff --git ") {
            // Save previous hunk if exists
            if let (Some(file), Some((old_start, old_lines, new_start, new_lines))) =
                (&current_file, current_hunk)
            {
                hunks.push(Hunk {
                    file_path: file.clone(),
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    content: hunk_content.clone(),
                    is_new_file,
                });
            }
            hunk_content.clear();
            current_hunk = None;
            is_new_file = false;

            // Parse file path from "diff --git a/path b/path"
            // Use robust parsing to handle paths containing " b/"
            current_file = parse_diff_git_path(line);
            continue;
        }

        // Detect new file mode
        if line.starts_with("new file mode") {
            is_new_file = true;
            continue;
        }

        // Hunk header: @@ -old_start,old_lines +new_start,new_lines @@
        if line.starts_with("@@ ") {
            // Save previous hunk if exists
            if let (Some(file), Some((old_start, old_lines, new_start, new_lines))) =
                (&current_file, current_hunk)
            {
                hunks.push(Hunk {
                    file_path: file.clone(),
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    content: hunk_content.clone(),
                    is_new_file,
                });
            }
            hunk_content.clear();

            // Parse hunk header
            if let Some((old, new)) = parse_hunk_header(line) {
                current_hunk = Some((old.0, old.1, new.0, new.1));
            }
            continue;
        }

        // Skip other headers
        if line.starts_with("---")
            || line.starts_with("+++")
            || line.starts_with("index ")
            || line.starts_with("deleted file")
        {
            continue;
        }

        // Accumulate hunk content
        if current_hunk.is_some() {
            hunk_content.push_str(line);
            hunk_content.push('\n');
        }
    }

    // Don't forget the last hunk
    if let (Some(file), Some((old_start, old_lines, new_start, new_lines))) =
        (&current_file, current_hunk)
    {
        hunks.push(Hunk {
            file_path: file.clone(),
            old_start,
            old_lines,
            new_start,
            new_lines,
            content: hunk_content,
            is_new_file,
        });
    }

    hunks
}

/// Parse file path from a "diff --git a/path b/path" line.
///
/// Handles edge cases like paths containing " b/" by using the fact that
/// git's diff output format has symmetric a-path and b-path (except for renames).
fn parse_diff_git_path(line: &str) -> Option<String> {
    // Format: "diff --git a/<path> b/<path>"
    let line = line.strip_prefix("diff --git ")?;

    // For non-rename cases, a-path and b-path are identical.
    // The line format is "a/<path> b/<path>".
    let after_a = line.strip_prefix("a/")?;

    // Since paths are symmetric: "<path> b/<path>"
    // Total length = path_len + 3 (" b/") + path_len
    // So: path_len = (total_len - 3) / 2
    let total_len = after_a.len();
    if total_len < 4 {
        // Too short to contain " b/" + at least 1 char
        return None;
    }

    // Calculate expected path length assuming symmetric paths
    let path_len = (total_len - 3) / 2;

    // Verify the separator is at the expected position
    let expected_sep = &after_a[path_len..path_len + 3];
    if expected_sep == " b/" {
        let a_path = &after_a[..path_len];
        let b_path = &after_a[path_len + 3..];

        // Verify paths match (for non-renames)
        if a_path == b_path {
            return Some(b_path.to_string());
        }
        // For renames, paths differ; return b-path (destination)
        return Some(b_path.to_string());
    }

    // Fallback for unusual cases: try simple split
    line.split(" b/").nth(1).map(String::from)
}

/// Parse a hunk header line like "@@ -1,3 +1,4 @@" or "@@ -1 +1,2 @@"
fn parse_hunk_header(line: &str) -> Option<((u32, u32), (u32, u32))> {
    // Strip @@ markers
    let line = line.trim_start_matches("@@ ").trim_end_matches(" @@");
    let line = line.split(" @@").next()?; // Handle trailing context

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let old = parse_range(parts[0].trim_start_matches('-'))?;
    let new = parse_range(parts[1].trim_start_matches('+'))?;

    Some((old, new))
}

/// Parse a range like "1,3" or "1" into (start, count).
fn parse_range(s: &str) -> Option<(u32, u32)> {
    if let Some((start, count)) = s.split_once(',') {
        Some((start.parse().ok()?, count.parse().ok()?))
    } else {
        // Single line: "1" means start=1, count=1
        Some((s.parse().ok()?, 1))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -1,3 +1,4 @@"), Some(((1, 3), (1, 4))));
        assert_eq!(
            parse_hunk_header("@@ -10,5 +12,7 @@ fn foo()"),
            Some(((10, 5), (12, 7)))
        );
        assert_eq!(parse_hunk_header("@@ -1 +1,2 @@"), Some(((1, 1), (1, 2))));
        assert_eq!(parse_hunk_header("@@ -0,0 +1,5 @@"), Some(((0, 0), (1, 5))));
    }

    #[test]
    fn test_parse_range() {
        assert_eq!(parse_range("1,3"), Some((1, 3)));
        assert_eq!(parse_range("10"), Some((10, 1)));
        assert_eq!(parse_range("0,0"), Some((0, 0)));
    }

    #[test]
    fn test_parse_diff_git_path() {
        // Simple path
        assert_eq!(
            parse_diff_git_path("diff --git a/src/main.rs b/src/main.rs"),
            Some("src/main.rs".to_string())
        );

        // Path with spaces
        assert_eq!(
            parse_diff_git_path("diff --git a/path with spaces/file.rs b/path with spaces/file.rs"),
            Some("path with spaces/file.rs".to_string())
        );

        // Path containing " b/" substring (edge case)
        assert_eq!(
            parse_diff_git_path("diff --git a/a b/c/file.rs b/a b/c/file.rs"),
            Some("a b/c/file.rs".to_string())
        );

        // Nested directories
        assert_eq!(
            parse_diff_git_path("diff --git a/deep/nested/path/file.rs b/deep/nested/path/file.rs"),
            Some("deep/nested/path/file.rs".to_string())
        );
    }

    #[test]
    fn test_parse_diff_hunks_single_file() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc123..def456 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,3 +10,4 @@ fn main() {
     println!("hello");
+    println!("world");
 }
"#;

        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "src/main.rs");
        assert_eq!(hunks[0].old_start, 10);
        assert_eq!(hunks[0].old_lines, 3);
        assert_eq!(hunks[0].new_start, 10);
        assert_eq!(hunks[0].new_lines, 4);
        assert!(!hunks[0].is_new_file);
    }

    #[test]
    fn test_parse_diff_hunks_multiple_hunks() {
        let diff = r"diff --git a/file.txt b/file.txt
index abc..def 100644
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,3 @@
 line1
+added
 line2
@@ -10,1 +11,2 @@
 line10
+another
";

        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[1].old_start, 10);
        assert!(!hunks[0].is_new_file);
        assert!(!hunks[1].is_new_file);
    }

    #[test]
    fn test_parse_diff_hunks_new_file() {
        let diff = r"diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,3 @@
+line1
+line2
+line3
";

        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "new.txt");
        assert_eq!(hunks[0].old_start, 0);
        assert_eq!(hunks[0].old_lines, 0);
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(hunks[0].new_lines, 3);
        assert!(hunks[0].is_new_file);
    }
}
