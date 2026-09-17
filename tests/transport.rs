#![cfg(any(feature = "async", feature = "blocking"))]
use slack::{Error, SendOptions, SentMessage, SlackMessage};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

const TOKEN: &str = "xoxb-super-secret";
const CONTENT: &str = "private operational message";

async fn send(
    base: String,
    blocking: bool,
    options: SendOptions,
    timeout: Duration,
) -> Result<SentMessage, Error> {
    send_message(
        base,
        blocking,
        options,
        timeout,
        SlackMessage::plain(CONTENT),
    )
    .await
}

async fn send_message(
    base: String,
    blocking: bool,
    options: SendOptions,
    timeout: Duration,
    message: SlackMessage,
) -> Result<SentMessage, Error> {
    if blocking {
        #[cfg(feature = "blocking")]
        return tokio::task::spawn_blocking(move || {
            slack::blocking::BlockingSlackBot::new(TOKEN, "CDEFAULT")
                .with_base_url(base)
                .with_timeout(timeout)
                .send_with_options(&message, options)
        })
        .await
        .unwrap();
    }
    #[cfg(feature = "async")]
    return slack::SlackBot::new(TOKEN, "CDEFAULT")
        .with_base_url(base)
        .with_timeout(timeout)
        .send_with_options(&message, options)
        .await;
    #[cfg(not(feature = "async"))]
    unreachable!()
}
fn transports() -> Vec<bool> {
    let mut values = Vec::new();
    if cfg!(feature = "async") {
        values.push(false);
    }
    if cfg!(feature = "blocking") {
        values.push(true);
    }
    values
}

#[tokio::test]
async fn sends_channel_override_and_thread_with_safe_defaults() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/chat.postMessage"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(body_json(serde_json::json!({"channel":"COTHER","text":CONTENT,"mrkdwn":false,"parse":"none","link_names":false,"unfurl_links":false,"unfurl_media":false,"thread_ts":"123.000001"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"channel":"COTHER","ts":"123.000002"})))
            .expect(1).mount(&server).await;
        let sent = send(
            server.uri(),
            blocking,
            SendOptions {
                channel: Some("COTHER".into()),
                thread_ts: Some("123.000001".into()),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(sent.channel, "COTHER");
        assert_eq!(sent.ts, "123.000002");
        server.verify().await;
    }
}

#[tokio::test]
async fn failures_are_classified_without_retries_or_sensitive_data() {
    for blocking in transports() {
        let cases = [
            (
                200,
                r#"{"ok":false,"error":"not_in_channel"}"#,
                Error::SlackApi {
                    code: "not_in_channel".into(),
                },
            ),
            (
                200,
                r#"{"ok":true,"channel":"CDEFAULT"}"#,
                Error::MalformedResponse,
            ),
            (200, "not json", Error::MalformedResponse),
            (
                200,
                r#"{"ok":false,"error":"xoxb-super-secret private operational message"}"#,
                Error::SlackApi {
                    code: "unknown_error".into(),
                },
            ),
            (
                403,
                "xoxb-super-secret private operational message",
                Error::Http { status: 403 },
            ),
            (
                500,
                "xoxb-super-secret private operational message",
                Error::Http { status: 500 },
            ),
            (
                429,
                "rate limited",
                Error::RateLimited {
                    retry_after: Some(Duration::from_secs(17)),
                },
            ),
        ];
        for (status, body, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(body)
                        .insert_header("Retry-After", "17"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let error = send(
                server.uri(),
                blocking,
                SendOptions::default(),
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
            assert_eq!(error, expected);
            for text in [error.to_string(), format!("{error:?}")] {
                assert!(!text.contains(TOKEN));
                assert!(!text.contains(CONTENT));
            }
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn timeout_is_one_attempt() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(300)))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            send(
                server.uri(),
                blocking,
                SendOptions::default(),
                Duration::from_millis(80)
            )
            .await,
            Err(Error::Transport)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn redirects_do_not_forward_credentials_or_retry() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(path("/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/elsewhere", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            send(
                server.uri(),
                blocking,
                SendOptions::default(),
                Duration::from_secs(2)
            )
            .await,
            Err(Error::Http { status: 302 })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejects_invalid_request_without_network() {
    for blocking in transports() {
        let server = MockServer::start().await;
        assert!(matches!(
            send(
                server.uri(),
                blocking,
                SendOptions {
                    channel: Some("".into()),
                    ..Default::default()
                },
                Duration::from_secs(2)
            )
            .await,
            Err(Error::Validation(_))
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn retry_after_is_optional_and_must_be_seconds() {
    for blocking in transports() {
        for header_value in [None, Some("invalid")] {
            let server = MockServer::start().await;
            let mut response = ResponseTemplate::new(429);
            if let Some(value) = header_value {
                response = response.insert_header("retry-after", value);
            }
            Mock::given(method("POST"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                send(
                    server.uri(),
                    blocking,
                    SendOptions::default(),
                    Duration::from_secs(2)
                )
                .await,
                Err(Error::RateLimited { retry_after: None })
            );
        }
    }
}

#[tokio::test]
async fn literal_and_formatted_payloads_have_distinct_semantics() {
    for blocking in transports() {
        for (message, wire, mrkdwn) in [
            (
                SlackMessage::plain("a_b *x* `z` <@U123> &"),
                "a_b *x* `z` &lt;@U123&gt; &amp;",
                false,
            ),
            (
                SlackMessage::mrkdwn("*heading* <https://example.com|link>"),
                "*heading* <https://example.com|link>",
                true,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(body_json(serde_json::json!({
                "channel":"CDEFAULT", "text":wire, "mrkdwn":mrkdwn,
                "parse":"none", "link_names":false, "unfurl_links":false, "unfurl_media":false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok":true,"channel":"CDEFAULT","ts":"123.000001"
            })))
            .expect(1)
            .mount(&server)
            .await;
            send_message(
                server.uri(),
                blocking,
                SendOptions::default(),
                Duration::from_secs(2),
                message,
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn permits_long_messages_and_exact_encoded_limit_without_truncation() {
    for blocking in transports() {
        for (message, expected_text) in [
            (SlackMessage::plain("é".repeat(5000)), "é".repeat(5000)),
            (
                SlackMessage::plain("🙂".repeat(40_000)),
                "🙂".repeat(40_000),
            ),
            (SlackMessage::plain("&".repeat(8000)), "&amp;".repeat(8000)),
            (SlackMessage::mrkdwn("_".repeat(40_000)), "_".repeat(40_000)),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok":true,"channel":"CDEFAULT","ts":"123.000001"
                })))
                .expect(1)
                .mount(&server)
                .await;
            send_message(
                server.uri(),
                blocking,
                SendOptions::default(),
                Duration::from_secs(2),
                message,
            )
            .await
            .unwrap();
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests[0].body_json::<serde_json::Value>().unwrap()["text"],
                expected_text
            );
        }
    }
}

#[tokio::test]
async fn refuses_oversized_messages_before_network_for_both_transports() {
    for blocking in transports() {
        for (message, chars) in [
            (SlackMessage::plain("é".repeat(40_001)), 40_001),
            (SlackMessage::plain("&".repeat(8001)), 40_005),
            (SlackMessage::mrkdwn("x".repeat(40_001)), 40_001),
        ] {
            let server = MockServer::start().await;
            assert_eq!(
                send_message(
                    server.uri(),
                    blocking,
                    SendOptions::default(),
                    Duration::from_secs(2),
                    message
                )
                .await,
                Err(Error::MessageTooLong {
                    chars,
                    limit: 40_000
                })
            );
            assert!(server.received_requests().await.unwrap().is_empty());
        }
    }
}
