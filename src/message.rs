use std::fmt::Display;

/// Maximum top-level `text` length accepted by this crate, before Slack truncates it.
/// Counted after escaping literal text, excluding JSON string encoding.
pub const MESSAGE_MAX_CHARS: usize = 40_000;

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

/// A message keeps its original text; plain text is escaped only for transmission.
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
    /// Preserve literal text without parsing it as formatting or mention syntax.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
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

    /// Original source text, before any transport escaping.
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

/// Optional convenience for assembling plain-text lines; no field budgets or formatting.
#[derive(Default)]
pub struct MessageBuilder {
    lines: Vec<String>,
}
impl MessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn line(mut self, text: impl Display) -> Self {
        self.lines.push(text.to_string());
        self
    }
    pub fn kv(self, key: impl Display, value: impl Display) -> Self {
        self.line(format!("{key}: {value}"))
    }
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
    fn builder_does_not_truncate_large_values() {
        let value = "é".repeat(5000);
        let message = MessageBuilder::new()
            .line("alert")
            .kv("error", &value)
            .kv("slot", 42)
            .build();
        assert_eq!(message.text(), format!("alert\nerror: {value}\nslot: 42"));
    }
    #[test]
    fn truncation_handles_zero_and_unicode() {
        assert_eq!(truncate_text("abc", 0), "");
        assert_eq!(truncate_text("abc", 1), "…");
        assert_eq!(truncate_text("é🙂界x", 3), "é🙂…");
    }
}
