//! Property-based input strategies for hardening the command-output parsers.
//!
//! SysTUI parses the stdout of system tools (`ss`, `df`, `ip`, `systemctl`,
//! `journalctl`, package managers, ...). That output is trusted today, but a
//! truncated stream, a busybox variant, a localized build or a hostile host can
//! produce shapes the happy-path fixtures never cover. These strategies generate
//! such shapes so parser tests can assert the only invariant that must always
//! hold: **parsing never panics** — it returns empty/partial data instead.
//!
//! Two complementary generators:
//! - [`arbitrary_text`] — anything, including control bytes and newlines, to catch
//!   slicing/`unwrap` panics on genuinely malformed input;
//! - [`command_output`] — table-shaped lines of whitespace/colon-separated tokens
//!   (with empty fields, huge integers and negative numbers mixed in) to drive the
//!   field-splitting and number-parsing paths where indexing panics actually live.

use proptest::prelude::*;

/// Fully arbitrary text up to `max` bytes, including newlines and control chars.
pub fn arbitrary_text(max: usize) -> impl Strategy<Value = String> {
    // `(?s)` makes `.` match newlines too; the char class adds tabs and NULs that
    // a plain `.` regex would exclude, so truncated/binary streams are covered.
    prop::string::string_regex(&format!(
        "(?s)[\\x00-\\x{:x}\\n\\t]{{0,{max}}}",
        0x10ffff_u32
    ))
    .expect("valid regex")
}

/// A single token that often appears in command output: a word, a (possibly
/// out-of-range) integer, a path-like blob, an empty string, or random unicode.
fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z][a-zA-Z0-9._-]{0,16}",
        // Numbers including ones that overflow u64/usize and negatives, to stress
        // every `.parse()` the field readers do.
        "-?[0-9]{1,25}",
        "/[a-z/]{0,20}",
        Just(String::new()),
        "[\\x21-\\x7e]{0,8}",
    ]
}

/// Table-shaped command output: 0..40 lines, each a run of tokens joined by a mix
/// of single/multiple spaces, tabs and colons — the separators the parsers split on.
pub fn command_output() -> impl Strategy<Value = String> {
    let sep = prop_oneof![Just(" "), Just("  "), Just("\t"), Just(":"), Just(" : ")];
    let line = prop::collection::vec((token(), sep), 0..8).prop_map(|parts| {
        parts
            .into_iter()
            .map(|(tok, s)| format!("{tok}{s}"))
            .collect::<String>()
    });
    prop::collection::vec(line, 0..40).prop_map(|lines| lines.join("\n"))
}

/// Convenience: either generator, so one `proptest!` arm covers both raw and
/// table-shaped adversarial input.
pub fn messy_output() -> impl Strategy<Value = String> {
    prop_oneof![command_output(), arbitrary_text(2000)]
}
