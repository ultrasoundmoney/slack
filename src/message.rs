use std::fmt::Display;

use serde_json::{Value, json};

/// Ceiling for a message's top-level `text`, counted after Slack's `&`, `<`, `>` escaping.
/// Slack truncates beyond it.
pub const MESSAGE_MAX_CHARS: usize = 40_000;
/// Total text the builder puts in its block. Slack's limit for rich text is undocumented;
/// `msg_blocks_too_long` has been reported from about 13,000 characters.
const BLOCK_MAX_CHARS: usize = 12_000;
/// Per value cap, so one huge error can't crowd out the rest of an alert.
const FIELD_MAX_CHARS: usize = 3_000;
/// Room kept for the line noting omitted fields.
const OMISSION_RESERVE: usize = 64;
const TRUNCATION_MARKER: &str = "… [truncated]";

fn escaped_width(c: char) -> usize {
    match c {
        '&' => 5,
        '<' | '>' => 4,
        _ => 1,
    }
}

fn width(text: &str, width: impl Fn(char) -> usize) -> usize {
    text.chars().map(width).sum()
}

fn take_width(text: &str, limit: usize, width: impl Fn(char) -> usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|&c| {
            used += width(c);
            used <= limit
        })
        .collect()
}

// Looks at no more than a prefix of `text`, so a multi-megabyte dump costs little.
fn truncate_with(text: &str, limit: usize, width: impl Fn(char) -> usize + Copy) -> String {
    let mut used = 0;
    let fits = text.chars().all(|c| {
        used += width(c);
        used <= limit
    });
    if fits {
        return text.to_owned();
    }

    let marker_width = self::width(TRUNCATION_MARKER, width);
    if limit < marker_width {
        return take_width(text, limit, width);
    }

    take_width(text, limit - marker_width, width) + TRUNCATION_MARKER
}

/// Shortens `text` to at most `max_chars` characters, ending in `… [truncated]` when cut.
///
/// Counts Unicode scalar values, so UTF-8 stays intact but a grapheme cluster can be split.
pub fn truncate(text: &str, max_chars: usize) -> String {
    truncate_with(text, max_chars, |_| 1)
}

fn truncate_escaped(text: &str, max_chars: usize) -> String {
    truncate_with(text, max_chars, escaped_width)
}

/// Escapes Slack's control characters `&`, `<` and `>`, which is all literal text needs once
/// mrkdwn is off.
pub fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What gets posted: literal `text`, optionally rendered through Block Kit `blocks`.
///
/// When blocks are present Slack shows them, and uses `text` for notifications and as the
/// fallback this crate sends if Slack rejects the blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    text: String,
    blocks: Option<Value>,
}

impl Message {
    /// Literal text, shown exactly as given. Cut to fit [`MESSAGE_MAX_CHARS`] after escaping.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: truncate_escaped(&text.into(), MESSAGE_MAX_CHARS),
            blocks: None,
        }
    }

    /// Attaches raw Block Kit blocks, e.g. buttons. Sent as given; the text stays the fallback.
    pub fn with_blocks(mut self, blocks: Value) -> Self {
        self.blocks = Some(blocks);
        self
    }

    /// The literal text, before escaping.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn blocks(&self) -> Option<&Value> {
        self.blocks.as_ref()
    }

    #[cfg(any(feature = "async", feature = "blocking"))]
    pub(crate) fn without_blocks(&self) -> Self {
        Self {
            text: self.text.clone(),
            blocks: None,
        }
    }
}

#[derive(Clone, Debug)]
enum Part {
    Context(String),
    Line(String),
    Kv(String, String),
    Code(String, String),
    Link(String, String),
}

impl Part {
    fn cost(&self) -> usize {
        let chars = |text: &str| text.chars().count();
        match self {
            Self::Context(text) | Self::Line(text) => chars(text) + 1,
            Self::Kv(a, b) | Self::Code(a, b) | Self::Link(a, b) => chars(a) + chars(b) + 3,
        }
    }

    /// The part cut down to `budget`, or `None` when too little room is left to be useful.
    fn shrink(self, budget: usize) -> Option<Self> {
        let room = |label: &str| {
            budget
                .checked_sub(label.chars().count() + 3)
                .filter(|room| *room > TRUNCATION_MARKER.chars().count())
        };
        match self {
            Self::Context(text) => room("").map(|room| Self::Context(truncate(&text, room))),
            Self::Line(text) => room("").map(|room| Self::Line(truncate(&text, room))),
            Self::Kv(key, value) => room(&key).map(|room| Self::Kv(key, truncate(&value, room))),
            Self::Code(label, value) => {
                room(&label).map(|room| Self::Code(label, truncate(&value, room)))
            }
            Self::Link(..) => None,
        }
    }
}

/// Keeps parts in order until the budget runs out, cuts the one that overflows and notes how
/// many were left out.
fn fit(parts: Vec<Part>, budget: usize) -> Vec<Part> {
    let mut remaining = budget.saturating_sub(OMISSION_RESERVE);
    let mut fitted = Vec::new();
    let mut parts = parts.into_iter();

    for part in parts.by_ref() {
        let cost = part.cost();
        if cost <= remaining {
            remaining -= cost;
            fitted.push(part);
        } else {
            fitted.extend(part.shrink(remaining));
            break;
        }
    }

    let omitted = parts.count();
    if omitted > 0 {
        fitted.push(Part::Line(format!("… {omitted} more fields omitted")));
    }

    fitted
}

fn text_element(text: &str, style: Option<Value>) -> Option<Value> {
    (!text.is_empty()).then(|| match style {
        Some(style) => json!({"type": "text", "text": text, "style": style}),
        None => json!({"type": "text", "text": text}),
    })
}

fn bold(text: &str) -> Option<Value> {
    text_element(text, Some(json!({"bold": true})))
}

fn italic(text: &str) -> Option<Value> {
    text_element(text, Some(json!({"italic": true})))
}

fn plain(text: &str) -> Option<Value> {
    text_element(text, None)
}

enum Row {
    Inline(Vec<Value>),
    Preformatted(String),
}

/// Builds an operational message: a bold title, then fields, then an optional footer.
///
/// Values are literal. Formatting comes from structure, rendered as a rich text block, so log
/// lines and errors never need escaping. A plain text rendering of the same content goes along
/// as the notification text and fallback.
///
/// Every value is capped at 3,000 characters and the whole message at about 12,000. When fields
/// overflow, the overflowing one is cut and later ones are dropped with a note. The footer is
/// never dropped, so put the instruction the reader must see there.
#[derive(Clone, Debug)]
pub struct MessageBuilder {
    title: String,
    parts: Vec<Part>,
    footer: Option<String>,
}

fn field(value: impl Display) -> String {
    truncate(&value.to_string(), FIELD_MAX_CHARS)
}

impl MessageBuilder {
    pub fn new(title: impl Display) -> Self {
        Self {
            title: field(title),
            parts: Vec::new(),
            footer: None,
        }
    }

    /// A de-emphasized line, e.g. `network: mainnet | geo: rbx`.
    pub fn context(mut self, text: impl Display) -> Self {
        self.parts.push(Part::Context(field(text)));
        self
    }

    pub fn line(mut self, text: impl Display) -> Self {
        self.parts.push(Part::Line(field(text)));
        self
    }

    /// A short value after a bold key. For multi-line values use [`Self::code`].
    pub fn kv(mut self, key: impl Display, value: impl Display) -> Self {
        self.parts.push(Part::Kv(field(key), field(value)));
        self
    }

    /// A labelled preformatted block, for errors, logs and anything multi-line.
    pub fn code(mut self, label: impl Display, text: impl Display) -> Self {
        self.parts.push(Part::Code(field(label), field(text)));
        self
    }

    pub fn link(mut self, label: impl Display, url: impl Display) -> Self {
        self.parts.push(Part::Link(field(label), field(url)));
        self
    }

    /// The last line, kept whatever else gets cut. Replaces an earlier footer.
    pub fn footer(mut self, text: impl Display) -> Self {
        self.footer = Some(field(text));
        self
    }

    pub fn build(self) -> Message {
        let footer = self.footer.unwrap_or_default();
        let budget =
            BLOCK_MAX_CHARS.saturating_sub(self.title.chars().count() + footer.chars().count() + 2);
        let parts = fit(self.parts, budget);

        Message {
            text: render_text(&self.title, &parts, &footer),
            blocks: Some(render_blocks(&self.title, &parts, &footer)),
        }
    }
}

fn render_text(title: &str, parts: &[Part], footer: &str) -> String {
    let body = std::iter::once(title.to_owned())
        .chain(parts.iter().map(|part| match part {
            Part::Context(text) | Part::Line(text) => text.clone(),
            Part::Kv(key, value) => format!("{key}: {value}"),
            Part::Code(label, value) => format!("{label}:\n{value}"),
            Part::Link(label, url) => format!("{label}: {url}"),
        }))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if footer.is_empty() {
        return truncate_escaped(&body, MESSAGE_MAX_CHARS);
    }

    let budget = MESSAGE_MAX_CHARS - width(footer, escaped_width) - 1;
    format!("{}\n{footer}", truncate_escaped(&body, budget))
}

fn rows(title: &str, parts: &[Part], footer: &str) -> Vec<Row> {
    let inline = |element: Option<Value>| Row::Inline(element.into_iter().collect());

    std::iter::once(inline(bold(title)))
        .chain(parts.iter().flat_map(|part| match part {
            Part::Context(text) => vec![inline(italic(text))],
            Part::Line(text) => vec![inline(plain(text))],
            Part::Kv(key, value) => vec![Row::Inline(
                bold(&format!("{key}: "))
                    .into_iter()
                    .chain(plain(value))
                    .collect(),
            )],
            Part::Code(label, value) => vec![
                inline(bold(&format!("{label}:"))),
                // Slack rejects empty text elements.
                Row::Preformatted(if value.is_empty() {
                    " ".to_string()
                } else {
                    value.clone()
                }),
            ],
            Part::Link(label, url) => vec![Row::Inline(vec![
                json!({"type": "link", "url": url, "text": label}),
            ])],
        }))
        .chain(std::iter::once(inline(plain(footer))))
        .filter(|row| !matches!(row, Row::Inline(elements) if elements.is_empty()))
        .collect()
}

fn section(elements: Vec<Value>) -> Value {
    json!({"type": "rich_text_section", "elements": elements})
}

fn render_blocks(title: &str, parts: &[Part], footer: &str) -> Value {
    let mut elements = Vec::new();
    let mut current: Vec<Value> = Vec::new();

    for row in rows(title, parts, footer) {
        match row {
            Row::Inline(row) => {
                if !current.is_empty() {
                    current.extend(plain("\n"));
                }
                current.extend(row);
            }
            Row::Preformatted(text) => {
                if !current.is_empty() {
                    elements.push(section(std::mem::take(&mut current)));
                }
                elements.push(json!({
                    "type": "rich_text_preformatted",
                    "elements": [{"type": "text", "text": text}],
                }));
            }
        }
    }

    if !current.is_empty() {
        elements.push(section(current));
    }

    json!([{"type": "rich_text", "elements": elements}])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn escaped_len(text: &str) -> usize {
        width(text, escaped_width)
    }

    fn block_text(message: &Message) -> String {
        fn collect(value: &Value, out: &mut String) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(text)) = map.get("text") {
                        out.push_str(text);
                    }
                    map.values().for_each(|value| collect(value, out));
                }
                Value::Array(values) => values.iter().for_each(|value| collect(value, out)),
                _ => {}
            }
        }
        let mut out = String::new();
        collect(message.blocks().unwrap(), &mut out);
        out
    }

    #[test]
    fn builder_renders_structure_as_rich_text() {
        let message = MessageBuilder::new("persistence failed")
            .context("mainnet | tyo")
            .kv("slot", 42)
            .code("error", "pool timed out\n<&>")
            .link("dashboard", "https://example.com")
            .footer("restart to resume submissions")
            .build();

        assert_eq!(
            message.text(),
            "persistence failed\nmainnet | tyo\nslot: 42\nerror:\npool timed out\n<&>\ndashboard: https://example.com\nrestart to resume submissions"
        );
        assert_eq!(
            message.blocks().unwrap(),
            &json!([{"type": "rich_text", "elements": [
                {"type": "rich_text_section", "elements": [
                    {"type": "text", "text": "persistence failed", "style": {"bold": true}},
                    {"type": "text", "text": "\n"},
                    {"type": "text", "text": "mainnet | tyo", "style": {"italic": true}},
                    {"type": "text", "text": "\n"},
                    {"type": "text", "text": "slot: ", "style": {"bold": true}},
                    {"type": "text", "text": "42"},
                    {"type": "text", "text": "\n"},
                    {"type": "text", "text": "error:", "style": {"bold": true}},
                ]},
                {"type": "rich_text_preformatted", "elements": [{"type": "text", "text": "pool timed out\n<&>"}]},
                {"type": "rich_text_section", "elements": [
                    {"type": "link", "url": "https://example.com", "text": "dashboard"},
                    {"type": "text", "text": "\n"},
                    {"type": "text", "text": "restart to resume submissions"},
                ]},
            ]}])
        );
    }

    #[test]
    fn overflow_cuts_fields_but_keeps_the_footer() {
        let builder = (0..10).fold(MessageBuilder::new("alert"), |builder, n| {
            builder.code(format!("error {n}"), "x".repeat(10_000))
        });
        let message = builder.footer("Check payment before retrying.").build();

        assert!(message.text().ends_with("\nCheck payment before retrying."));
        assert!(message.text().contains("more fields omitted"));
        assert!(block_text(&message).ends_with("Check payment before retrying."));
        assert!(block_text(&message).chars().count() <= BLOCK_MAX_CHARS);
        assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
    }

    #[test]
    fn empty_values_produce_no_empty_elements() {
        let message = MessageBuilder::new("")
            .kv("key", "")
            .code("log", "")
            .build();

        assert_eq!(message.text(), "key: \nlog:\n");
        assert!(
            !message
                .blocks()
                .unwrap()
                .to_string()
                .contains(r#""text":"""#)
        );
    }

    #[test]
    fn plain_messages_have_no_blocks_and_are_bounded_after_escaping() {
        for text in ["x".repeat(1_048_576), "<&🙂".repeat(100_000)] {
            let message = Message::plain(&text);
            assert!(message.blocks().is_none());
            assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
            assert!(text.starts_with(message.text().strip_suffix(TRUNCATION_MARKER).unwrap()));
        }

        let exact = "&".repeat(8_000);
        assert_eq!(Message::plain(&exact).text(), exact);
    }

    #[test]
    fn truncate_handles_tiny_limits_and_unicode() {
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(
            truncate("é🙂界x".repeat(10).as_str(), 15),
            "é🙂… [truncated]"
        );
    }

    proptest! {
        #[test]
        fn escaping_round_trips(text in ".{0,2000}") {
            let escaped = escape_text(&text);
            prop_assert!(!escaped.contains('<') && !escaped.contains('>'));
            let decoded = escaped.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
            prop_assert_eq!(decoded, text);
        }

        #[test]
        fn truncate_is_a_marked_prefix_within_budget(text in ".{0,2000}", limit in 0usize..2000) {
            let output = truncate(&text, limit);
            prop_assert!(output.chars().count() <= limit);
            if text.chars().count() <= limit {
                prop_assert_eq!(output, text);
            } else {
                let prefix = output.strip_suffix(TRUNCATION_MARKER).unwrap_or(&output);
                prop_assert!(text.starts_with(prefix));
            }
        }

        #[test]
        fn builder_always_fits_and_keeps_the_footer(
            fields in prop::collection::vec((".{0,20}", 0usize..400), 0..60),
        ) {
            let builder = fields.iter().fold(MessageBuilder::new("alert"), |builder, (seed, repeat)| {
                builder.kv("value", seed.repeat(*repeat))
            });
            let message = builder.footer("Check payment before retrying.").build();

            prop_assert!(escaped_len(message.text()) <= MESSAGE_MAX_CHARS);
            prop_assert!(block_text(&message).chars().count() <= BLOCK_MAX_CHARS);
            prop_assert!(message.text().ends_with("Check payment before retrying."));
        }
    }
}
