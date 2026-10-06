use std::fmt::Display;

/// Maximum top-level `text` length accepted by this crate, before Slack truncates it.
/// Counted after escaping literal text, excluding JSON string encoding.
pub const MESSAGE_MAX_CHARS: usize = 40_000;
const DEFAULT_VALUE_LIMIT: usize = 4_000;
const TRUNCATION_MARKER: &str = "… [truncated]";

fn escaped_width(c: char) -> usize {
    match c {
        '&' => 5,
        '<' | '>' => 4,
        _ => 1,
    }
}

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
    let marker = if limit >= TRUNCATION_MARKER.chars().count() {
        TRUNCATION_MARKER
    } else {
        "…"
    };
    let budget = limit - marker.chars().count();
    let mut used = 0;
    let prefix: String = text
        .chars()
        .take_while(|&c| {
            used += escaped_width(c);
            used <= budget
        })
        .collect();
    prefix + marker
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

/// Plain-text lines with automatic field and aggregate size protection.
pub struct MessageBuilder {
    fields: Vec<Field>,
    value_limit: usize,
}

struct Field {
    key: Option<String>,
    value: String,
}

impl Default for MessageBuilder {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            value_limit: DEFAULT_VALUE_LIMIT,
        }
    }
}
impl MessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn line(mut self, text: impl Display) -> Self {
        self.fields.push(Field {
            key: None,
            value: text.to_string(),
        });
        self
    }
    pub fn kv(mut self, key: impl Display, value: impl Display) -> Self {
        self.fields.push(Field {
            key: Some(key.to_string()),
            value: value.to_string(),
        });
        self
    }
    /// Set the escaped-character cap for every line, value, and label at build time.
    /// Defaults to 4,000; clamped to 1..=40,000. Independent of call order.
    pub fn value_limit(mut self, limit: usize) -> Self {
        self.value_limit = limit.clamp(1, MESSAGE_MAX_CHARS);
        self
    }
    /// Shrink the largest values to a common cap when their total is too large.
    /// Short values and field order are preserved. If labels and minimal values
    /// alone cannot fit, omit trailing fields with a visible omission count.
    pub fn build(self) -> SlackMessage {
        let mut fields: Vec<_> = self
            .fields
            .into_iter()
            .map(|field| Field {
                key: field.key.map(|key| bounded_text(&key, self.value_limit)),
                value: bounded_text(&field.value, self.value_limit),
            })
            .collect();
        let widths: Vec<_> = fields
            .iter()
            .map(|field| escaped_len(&field.value))
            .collect();
        let label_width = |field: &Field| field.key.as_ref().map_or(0, |key| escaped_len(key) + 2);
        let mut labels: usize = fields.iter().map(label_width).sum();
        let mut minimum: usize = widths.iter().map(|width| usize::from(*width > 0)).sum();
        let original_count = fields.len();
        let suffix = |count| {
            if count == original_count {
                String::new()
            } else {
                format!("[{} fields omitted]", original_count - count)
            }
        };
        let overhead = |labels: usize, count: usize| {
            labels
                + count.saturating_sub(1)
                + if count == original_count {
                    0
                } else {
                    suffix(count).len() + usize::from(count > 0)
                }
        };
        while overhead(labels, fields.len()) + minimum > MESSAGE_MAX_CHARS {
            let field = fields.pop().expect("an omission suffix alone always fits");
            labels -= label_width(&field);
            minimum -= usize::from(widths[fields.len()] > 0);
        }
        let available = MESSAGE_MAX_CHARS - overhead(labels, fields.len());
        // min(width, cap) conservatively accounts for complete escaped characters:
        // a prefix can leave a few unused characters rather than split an entity.
        let mut low = 1;
        let mut high = self.value_limit;
        while low < high {
            let cap = low + (high - low).div_ceil(2);
            let total: usize = widths[..fields.len()]
                .iter()
                .map(|width| (*width).min(cap))
                .sum();
            if total <= available {
                low = cap;
            } else {
                high = cap - 1;
            }
        }
        let omitted = suffix(fields.len());
        let mut lines: Vec<_> = fields
            .into_iter()
            .map(|field| {
                let value = bounded_text(&field.value, low);
                match field.key {
                    Some(key) => format!("{key}: {value}"),
                    None => value,
                }
            })
            .collect();
        if !omitted.is_empty() {
            lines.push(omitted);
        }
        SlackMessage {
            text: lines.join("\n"),
            mrkdwn: false,
        }
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
    fn aggregate_shrinks_large_fields_equally_and_keeps_short_tail() {
        let mut builder = MessageBuilder::new().line("alert");
        for _ in 0..20 {
            builder = builder.kv("error", "x".repeat(5000));
        }
        let message = builder
            .kv("report_id", 42)
            .line("Check payment before retrying.")
            .build();
        assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
        assert!(
            message
                .text()
                .ends_with("report_id: 42\nCheck payment before retrying.")
        );
        let errors: Vec<_> = message
            .text()
            .lines()
            .filter(|line| line.starts_with("error: "))
            .collect();
        assert_eq!(errors.len(), 20);
        assert!(
            errors
                .iter()
                .all(|line| *line == errors[0] && line.ends_with(TRUNCATION_MARKER))
        );
        assert!(!message.text().contains("fields omitted"));
    }

    #[test]
    fn overrides_apply_at_build_time_and_are_clamped() {
        let text = "x".repeat(5000);
        let first = MessageBuilder::new()
            .value_limit(6000)
            .kv("key", &text)
            .build();
        let last = MessageBuilder::new()
            .kv("key", &text)
            .value_limit(6000)
            .build();
        assert_eq!(first.text(), last.text());
        assert_eq!(first.text(), format!("key: {text}"));
        assert_eq!(
            MessageBuilder::new()
                .line("&")
                .value_limit(0)
                .build()
                .text(),
            "…"
        );
        assert!(
            escaped_len(
                MessageBuilder::new()
                    .line("&".repeat(100_000))
                    .value_limit(usize::MAX)
                    .build()
                    .text()
            ) <= MESSAGE_MAX_CHARS
        );
        assert_eq!(
            MessageBuilder::default()
                .line("short")
                .kv("slot", 42)
                .build()
                .text(),
            "short\nslot: 42"
        );
    }

    #[test]
    fn too_many_fields_or_large_labels_have_visible_omission_counts() {
        let mut builder = MessageBuilder::new();
        for _ in 0..30_000 {
            builder = builder.line("x");
        }
        let message = builder.build();
        let retained = message.text().lines().filter(|line| *line == "x").count();
        assert!(
            message
                .text()
                .ends_with(&format!("[{} fields omitted]", 30_000 - retained))
        );
        assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
        let message = MessageBuilder::new()
            .value_limit(40_000)
            .kv("&".repeat(100_000), "value")
            .build();
        assert_eq!(message.text(), "[1 fields omitted]");
        let message = MessageBuilder::new()
            .kv("&".repeat(100_000), "value")
            .build();
        assert!(message.text().ends_with("… [truncated]: value"));
        assert!(escaped_len(message.text()) <= 4007);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn escaped_truncation_is_a_marked_prefix(
            text in prop::collection::vec(prop_oneof![Just('&'), Just('<'), Just('>'), Just('\n'), any::<char>()], 0..6000),
            limit in 1usize..5000,
        ) {
            let text: String = text.into_iter().collect();
            let output = bounded_text(&text, limit);
            prop_assert!(escaped_len(&output) <= limit);
            if escaped_len(&text) <= limit { prop_assert_eq!(output, text); }
            else {
                let marker = if limit >= TRUNCATION_MARKER.chars().count() { TRUNCATION_MARKER } else { "…" };
                prop_assert!(output.ends_with(marker));
                prop_assert!(text.starts_with(output.strip_suffix(marker).unwrap()));
            }
        }
        #[test]
        fn arbitrary_field_counts_obey_aggregate_and_keep_short_context(
            seeds in prop::collection::vec((".{0,30}", 0usize..300), 0..80),
            limit in 32usize..40_001,
        ) {
            let mut builder = MessageBuilder::new().value_limit(limit);
            for (seed, repeat) in &seeds { builder = builder.kv("error", seed.repeat(*repeat)); }
            let message = builder.kv("report", 42).line("Check payment before retrying.").build();
            prop_assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
            prop_assert!(message.text().ends_with("report: 42\nCheck payment before retrying."));
            prop_assert!(!message.text().contains("fields omitted"));
        }
    }
}
