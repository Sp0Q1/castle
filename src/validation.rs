//! Server-side validation of user-supplied text.
//!
//! This is deliberately *validation*, not sanitization. Findings and comments
//! are penetration-test content: a report legitimately quotes payloads like
//! `"><script>alert(1)</script>`, and rewriting that on the way in would corrupt
//! the deliverable. Stored text therefore round-trips verbatim, and the XSS
//! defense lives entirely at render time (rehype-sanitize on the markdown AST,
//! CSP, mermaid `securityLevel: "strict"`). Anything that renders markdown
//! *outside* the browser — a future PDF/HTML export or HTML mail — must add its
//! own output sanitization; this module does not provide it.
//!
//! What is enforced here is what output encoding cannot fix:
//!
//! * length caps, so a single request cannot exhaust memory/disk or produce a
//!   document no client can render;
//! * rejection of NUL and other stray control characters, which Postgres
//!   refuses in `text` columns (an unhandled 500) and which can hide content
//!   from a reviewer reading the report.

use loco_rs::prelude::*;

/// One-line fields (finding title, project name).
pub const MAX_TITLE: usize = 300;
/// Short classifiers (finding type, role names).
pub const MAX_LABEL: usize = 120;
/// A markdown report section. Large on purpose — technical descriptions carry
/// request/response transcripts — but not unbounded.
pub const MAX_SECTION: usize = 64 * 1024;
/// A discussion comment.
pub const MAX_COMMENT: usize = 16 * 1024;
/// Project description (plain summary, not a report section).
pub const MAX_DESCRIPTION: usize = 4 * 1024;
/// RFC 5321 caps an address at 320 characters.
pub const MAX_EMAIL: usize = 320;

/// Control characters are rejected except the three that legitimately occur in
/// markdown. NUL in particular is not storable in a Postgres `text` column, so
/// letting it through turns into a 500 rather than a 400.
fn has_forbidden_control_char(value: &str) -> bool {
    value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// Validates a field that may be empty (but not oversized or malformed).
///
/// Length is counted in `char`s so the limit means the same thing for callers
/// writing non-ASCII as for ASCII.
pub fn text(field: &str, value: &str, max: usize) -> Result<()> {
    if value.chars().count() > max {
        return Err(Error::BadRequest(format!(
            "{field} must be at most {max} characters"
        )));
    }
    if has_forbidden_control_char(value) {
        return Err(Error::BadRequest(format!(
            "{field} contains unsupported control characters"
        )));
    }
    Ok(())
}

/// Validates a field that must also carry content — whitespace only is rejected.
pub fn required_text(field: &str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::BadRequest(format!("{field} cannot be empty")));
    }
    text(field, value, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_input() {
        let long = "a".repeat(MAX_TITLE + 1);
        assert!(text("title", &long, MAX_TITLE).is_err());
        assert!(text("title", &"a".repeat(MAX_TITLE), MAX_TITLE).is_ok());
    }

    #[test]
    fn counts_characters_not_bytes() {
        // 4 bytes each, so a byte-based check would reject this at 2 chars.
        assert!(text("title", "😀😀", 2).is_ok());
    }

    #[test]
    fn rejects_nul_and_control_chars_but_keeps_markdown_whitespace() {
        assert!(text("body", "a\0b", MAX_SECTION).is_err());
        assert!(text("body", "a\u{7}b", MAX_SECTION).is_err());
        assert!(text("body", "line\nline\r\n\tindented", MAX_SECTION).is_ok());
    }

    #[test]
    fn preserves_payloads_verbatim() {
        // A finding quoting an XSS payload must validate cleanly: we never
        // rewrite report content.
        let payload = r#""><script>alert(document.cookie)</script>"#;
        assert!(required_text("description", payload, MAX_SECTION).is_ok());
    }

    #[test]
    fn requires_content_when_required() {
        assert!(required_text("title", "   \n ", MAX_TITLE).is_err());
        assert!(required_text("title", "SQL Injection", MAX_TITLE).is_ok());
    }
}
