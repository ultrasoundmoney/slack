use std::fmt::Display;

/// Conservative library budget, not Slack's hard limit.
pub const MESSAGE_MAX_CHARS: usize = 4_000;
const VALUE_MAX_CHARS: usize = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParseMode {
    #[default]
    Plain,
    Mrkdwn,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum ValueBudget {
    #[default]
    Default,
    Unlimited,
    Chars(usize),
}

/// Text is private so every message obeys the final-message budget.
#[derive(Clone)]
pub struct SlackMessage {
    text: String,
    mode: ParseMode,
}
impl std::fmt::Debug for SlackMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackMessage")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}
impl SlackMessage {
    pub fn plain(text: impl Display) -> Self {
        MessageBuilder::default().line(text).build()
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn parse_mode(&self) -> ParseMode {
        self.mode
    }
}

#[derive(Default)]
pub struct MessageBuilder {
    mode: ParseMode,
    parts: Vec<(String, &'static str)>,
}
impl MessageBuilder {
    pub fn new(mode: ParseMode) -> Self {
        Self {
            mode,
            parts: Vec::new(),
        }
    }
    pub fn line(mut self, text: impl Display) -> Self {
        self.parts.push((text.to_string(), ""));
        self
    }
    pub fn bold_line(mut self, text: impl Display) -> Self {
        self.parts.push((text.to_string(), "*"));
        self
    }
    pub fn heading(self, text: impl Display) -> Self {
        self.bold_line(text)
    }
    pub fn kv(self, key: impl Display, value: impl Display) -> Self {
        self.kv_with_budget(key, value, ValueBudget::Default)
    }
    pub fn kv_with_budget(
        self,
        key: impl Display,
        value: impl Display,
        budget: ValueBudget,
    ) -> Self {
        self.line(format!("{key}: {}", budget_value(value, budget)))
    }
    pub fn kv_code(mut self, key: impl Display, value: impl Display) -> Self {
        self.parts.push((
            format!("{key}: {}", budget_value(value, ValueBudget::Default)),
            "`",
        ));
        self
    }
    pub fn error(self, key: impl Display, value: impl Display) -> Self {
        self.error_with_budget(key, value, ValueBudget::Default)
    }
    pub fn error_with_budget(
        mut self,
        key: impl Display,
        value: impl Display,
        budget: ValueBudget,
    ) -> Self {
        self.parts
            .push((format!("{key}:\n{}", budget_value(value, budget)), "```"));
        self
    }
    pub fn build(self) -> SlackMessage {
        let mut text = String::new();
        let mut remaining = MESSAGE_MAX_CHARS;
        for (index, (value, wrapper)) in self.parts.iter().enumerate() {
            let wrapper = if self.mode == ParseMode::Mrkdwn {
                *wrapper
            } else {
                ""
            };
            let separator = usize::from(index > 0);
            let overhead = separator + 2 * wrapper.len();
            // Reserve a truncation indicator if subsequent content cannot fit.
            if remaining <= overhead + 1 {
                break;
            }
            let (rendered, truncated) = render(value, self.mode, remaining - overhead - 1);
            if index > 0 {
                text.push('\n');
            }
            text.push_str(wrapper);
            text.push_str(&rendered);
            text.push_str(wrapper);
            remaining -= overhead + rendered.chars().count();
            if truncated || (index + 1 < self.parts.len() && remaining <= 9) {
                text.push('…');
                break;
            }
        }
        SlackMessage {
            text,
            mode: self.mode,
        }
    }
}
fn budget_value(value: impl Display, budget: ValueBudget) -> String {
    let value = value.to_string();
    let limit = match budget {
        ValueBudget::Default => VALUE_MAX_CHARS,
        ValueBudget::Unlimited => return value,
        ValueBudget::Chars(n) => n,
    };
    if value.chars().count() <= limit {
        return value;
    }
    if limit == 0 {
        return String::new();
    }
    value
        .chars()
        .take(limit - 1)
        .chain(std::iter::once('…'))
        .collect()
}
fn render(value: &str, mode: ParseMode, budget: usize) -> (String, bool) {
    let mut out = String::new();
    let mut remaining = budget;
    for c in value.chars() {
        // Slack does not support general backslash escaping. Neutralize user-supplied
        // markup delimiters with visible Unicode equivalents; only builders add markup.
        let replacement = match (mode, c) {
            (_, '&') => "&amp;".to_owned(),
            (_, '<') => "&lt;".to_owned(),
            (_, '>') => "&gt;".to_owned(),
            (ParseMode::Mrkdwn, '*') => "∗".to_owned(),
            (ParseMode::Mrkdwn, '_') => "＿".to_owned(),
            (ParseMode::Mrkdwn, '~') => "∼".to_owned(),
            (ParseMode::Mrkdwn, '`') => "ˋ".to_owned(),
            _ => c.to_string(),
        };
        let count = replacement.chars().count();
        if count > remaining {
            return (out, true);
        }
        out.push_str(&replacement);
        remaining -= count;
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn budget_and_balanced_markup(value in ".{0,10000}") {
            let m = MessageBuilder::new(ParseMode::Mrkdwn).heading(&value).error("error", &value).build();
            prop_assert!(m.text().chars().count() <= MESSAGE_MAX_CHARS);
            prop_assert_eq!(m.text().matches('*').count() % 2, 0);
            prop_assert_eq!(m.text().matches('`').count() % 6, 0);
            prop_assert!(!m.text().contains('<'));
        }
        #[test]
        fn values_obey_budget(value in ".{0,3000}", limit in 0usize..1000) {
            prop_assert!(budget_value(&value, ValueBudget::Chars(limit)).chars().count() <= limit);
        }
    }
    #[test]
    fn escapes_mentions_and_markup() {
        let m = MessageBuilder::new(ParseMode::Mrkdwn)
            .line("<!channel> <@U123> & *x* ```")
            .build();
        assert_eq!(m.text(), "&lt;!channel&gt; &lt;@U123&gt; &amp; ∗x∗ ˋˋˋ");
        assert_eq!(SlackMessage::plain("<@U123>").text(), "&lt;@U123&gt;");
    }
    #[test]
    fn truncation_preserves_later_fields_and_entities() {
        let m = MessageBuilder::default()
            .error("error", "é".repeat(5000))
            .kv("slot", 42)
            .build();
        assert!(m.text().contains('…'));
        assert!(m.text().ends_with("slot: 42"));
        let m = SlackMessage::plain("&".repeat(5000));
        assert!(m.text().ends_with("&amp;…"));
    }
}
