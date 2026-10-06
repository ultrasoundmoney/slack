use std::fmt::Display;

/// Maximum top-level `text` length accepted by this crate, before Slack truncates it.
/// Counted after escaping literal text, excluding JSON string encoding.
pub const MESSAGE_MAX_CHARS: usize = 40_000;
const VALUE_MAX_CHARS: usize = 4_000;
const TRUNCATION_MARKER: &str = "… [truncated]";

fn escaped_width(c: char) -> usize {
    match c {
        '&' => 5,
        '<' | '>' => 4,
        _ => 1,
    }
}

#[cfg(test)]
fn escaped_len(text: &str) -> usize {
    text.chars().map(escaped_width).sum()
}

// Inspect only a bounded prefix, even for a multi-megabyte diagnostic. Return
// source text so entity encoding happens exactly once, at the transport boundary.
fn bounded_text(text: &str, limit: usize) -> String {
    let mut width = 0;
    let oversized = text.chars().any(|c| {
        width += escaped_width(c);
        width > limit
    });
    if !oversized {
        return text.to_owned();
    }
    let budget = limit - TRUNCATION_MARKER.chars().count();
    let mut used = 0;
    let prefix: String = text
        .chars()
        .take_while(|&c| {
            used += escaped_width(c);
            used <= budget
        })
        .collect();
    prefix + TRUNCATION_MARKER
}

/// Escape Slack's special parsing characters in a literal fragment.
///
/// This does not escape mrkdwn delimiters such as `*`, `_`, or backticks.
/// Use `SlackMessage::plain` for arbitrary text that must not be formatted.
pub fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Explicitly shorten literal source text, including a visible ellipsis in the budget.
///
/// Counts Unicode scalar values before Slack entity encoding. This preserves UTF-8,
/// but may split a grapheme cluster. It is not safe for truncating formatted mrkdwn.
/// The resulting message is still subject to send-time validation after escaping.
pub fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }
    text.chars()
        .take(max_chars - 1)
        .chain(std::iter::once('…'))
        .collect()
}

/// A message keeps unescaped text, possibly shortened by plain-text construction.
#[derive(Clone)]
pub struct SlackMessage {
    text: String,
    mrkdwn: bool,
}
impl std::fmt::Debug for SlackMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackMessage")
            .field("mrkdwn", &self.mrkdwn)
            .finish_non_exhaustive()
    }
}
impl SlackMessage {
    /// Literal text, automatically bounded to 40,000 escaped characters.
    /// Truncation preserves UTF-8 but may split a grapheme cluster.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: bounded_text(&text.into(), MESSAGE_MAX_CHARS),
            mrkdwn: false,
        }
    }

    /// Send caller-authored Slack mrkdwn unchanged.
    ///
    /// Explicit links and mentions remain active, even though automatic parsing is
    /// disabled. Do not pass arbitrary user/log text here. `escape_text` neutralizes
    /// special angle-bracket syntax, but not emphasis or code delimiters.
    pub fn mrkdwn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            mrkdwn: true,
        }
    }

    /// Possibly truncated source text, before any transport escaping.
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn is_mrkdwn(&self) -> bool {
        self.mrkdwn
    }

    #[cfg(any(test, feature = "async", feature = "blocking"))]
    pub(crate) fn wire_text(&self) -> std::borrow::Cow<'_, str> {
        if self.mrkdwn {
            std::borrow::Cow::Borrowed(&self.text)
        } else {
            std::borrow::Cow::Owned(escape_text(&self.text))
        }
    }
}

/// Plain-text lines with fixed field caps and a final aggregate prefix cap.
#[derive(Default)]
pub struct MessageBuilder {
    lines: Vec<String>,
}
impl MessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Append literal text, capped at 4,000 escaped characters.
    pub fn line(mut self, text: impl Display) -> Self {
        self.lines
            .push(bounded_text(&text.to_string(), VALUE_MAX_CHARS));
        self
    }
    /// Append a literal key/value pair, each capped at 4,000 escaped characters.
    pub fn kv(mut self, key: impl Display, value: impl Display) -> Self {
        self.lines.push(format!(
            "{}: {}",
            bounded_text(&key.to_string(), VALUE_MAX_CHARS),
            bounded_text(&value.to_string(), VALUE_MAX_CHARS)
        ));
        self
    }
    /// Join fields, then cap the message at 40,000 escaped characters.
    /// Aggregate truncation can remove trailing fields or instructions.
    pub fn build(self) -> SlackMessage {
        SlackMessage::plain(self.lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn plain_text_roundtrips_without_punctuation_changes(text in ".{0,2000}") {
            let message = SlackMessage::plain(text.clone());
            prop_assert_eq!(message.text(), &text);
            let wire = message.wire_text();
            prop_assert!(!wire.contains('<') && !wire.contains('>'));
            let decoded = wire.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
            prop_assert_eq!(decoded, text);
        }
        #[test]
        fn opt_in_truncation_obeys_requested_budget(text in ".{0,2000}", limit in 0usize..2000) {
            let output = truncate_text(&text, limit);
            prop_assert!(output.chars().count() <= limit);
            if text.chars().count() <= limit { prop_assert_eq!(output, text); }
            else if limit > 0 {
                prop_assert!(output.ends_with('…'));
                prop_assert_eq!(output.chars().count(), limit);
            }
        }
    }
    #[test]
    fn plain_escapes_special_parsing_but_preserves_logs() {
        let message = SlackMessage::plain("<!channel> <@U123> a_b *x* ~y~ `z` &amp;");
        assert_eq!(
            message.wire_text(),
            "&lt;!channel&gt; &lt;@U123&gt; a_b *x* ~y~ `z` &amp;amp;"
        );
        assert!(!message.is_mrkdwn());
    }
    #[test]
    fn explicit_mrkdwn_is_passed_through() {
        let text = "*heading*\n<https://example.com|link> <@U123> `a_b`";
        let message = SlackMessage::mrkdwn(text);
        assert_eq!(message.wire_text(), text);
        assert!(message.is_mrkdwn());
    }
    #[test]
    fn builder_bounds_large_values_and_preserves_later_context() {
        let value = "é".repeat(5000);
        let message = MessageBuilder::new()
            .line("alert")
            .kv("error", &value)
            .kv("slot", 42)
            .build();
        assert_eq!(
            message.text(),
            format!("alert\nerror: {}\nslot: 42", bounded_text(&value, 4000))
        );
    }
    #[test]
    fn truncation_handles_zero_and_unicode() {
        assert_eq!(truncate_text("abc", 0), "");
        assert_eq!(truncate_text("abc", 1), "…");
        assert_eq!(truncate_text("é🙂界x", 3), "é🙂…");
    }

    #[test]
    fn plain_contains_megabyte_dumps_and_counts_escaping() {
        for text in ["x".repeat(1_048_576), "<&🙂".repeat(100_000)] {
            let message = SlackMessage::plain(&text);
            assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
            assert!(message.text().ends_with(TRUNCATION_MARKER));
            assert!(text.starts_with(message.text().strip_suffix(TRUNCATION_MARKER).unwrap()));
        }
        let exact = "&".repeat(8_000);
        assert_eq!(SlackMessage::plain(&exact).text(), exact);
        assert!(
            SlackMessage::plain(exact + "x")
                .text()
                .ends_with(TRUNCATION_MARKER)
        );
    }

    #[test]
    fn aggregate_overflow_cuts_the_tail_with_a_visible_marker() {
        let mut builder = MessageBuilder::new().line("alert");
        for _ in 0..20 {
            builder = builder.kv("error", "x".repeat(5000));
        }
        let message = builder.line("Check payment before retrying.").build();
        assert_eq!(escaped_len(message.text()), MESSAGE_MAX_CHARS);
        assert!(message.text().ends_with(TRUNCATION_MARKER));
        assert!(!message.text().contains("Check payment before retrying."));
        assert!(message.text().starts_with("alert\nerror: "));
    }

    #[test]
    fn builder_limits_fields_but_plain_allows_longer_text() {
        let text = "&".repeat(1000);
        let bounded = MessageBuilder::new().line(&text).build();
        assert!(escaped_len(bounded.text()) <= VALUE_MAX_CHARS);
        assert!(bounded.text().ends_with(TRUNCATION_MARKER));
        assert_eq!(SlackMessage::plain(&text).text(), text);
        let exact = "&".repeat(800);
        assert_eq!(MessageBuilder::new().line(&exact).build().text(), exact);
        assert_eq!(
            MessageBuilder::default()
                .line("short")
                .kv("slot", 42)
                .build()
                .text(),
            "short\nslot: 42"
        );
        assert_eq!(MessageBuilder::new().build().text(), "");
    }

    #[test]
    fn huge_labels_and_many_small_fields_remain_bounded() {
        let message = MessageBuilder::new()
            .kv("&".repeat(100_000), "value")
            .build();
        assert!(message.text().ends_with("… [truncated]: value"));
        assert!(escaped_len(message.text()) <= VALUE_MAX_CHARS + 7);
        let mut builder = MessageBuilder::new();
        for _ in 0..30_000 {
            builder = builder.line("x");
        }
        let message = builder.build();
        assert_eq!(escaped_len(message.text()), MESSAGE_MAX_CHARS);
        assert!(message.text().ends_with(TRUNCATION_MARKER));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn escaped_truncation_is_a_marked_prefix(
            text in prop::collection::vec(prop_oneof![Just('&'), Just('<'), Just('>'), Just('\n'), any::<char>()], 0..6000),
            limit in 13usize..5000,
        ) {
            let text: String = text.into_iter().collect();
            let output = bounded_text(&text, limit);
            prop_assert!(escaped_len(&output) <= limit);
            if escaped_len(&text) <= limit { prop_assert_eq!(output, text); }
            else {
                prop_assert!(output.ends_with(TRUNCATION_MARKER));
                prop_assert!(text.starts_with(output.strip_suffix(TRUNCATION_MARKER).unwrap()));
            }
        }
        #[test]
        fn arbitrary_field_counts_obey_aggregate_budget(
            seeds in prop::collection::vec((".{0,30}", 0usize..300), 0..80),
        ) {
            let mut builder = MessageBuilder::new();
            for (seed, repeat) in &seeds { builder = builder.kv("error", seed.repeat(*repeat)); }
            let message = builder.kv("report", 42).line("Check payment before retrying.").build();
            prop_assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
            prop_assert!(message.text().ends_with("Check payment before retrying.") || message.text().ends_with(TRUNCATION_MARKER));
        }
    }
}
