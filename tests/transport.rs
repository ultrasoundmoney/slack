//! Runs every case against both transports, with a local mock in place of Slack.
#![cfg(any(feature = "async", feature = "blocking"))]
use std::time::Duration;

use serde_json::{Value, json};
use slack::{Error, Message, MessageBuilder, Retry, SentMessage, Stage};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, body_partial_json, header, method, path},
};

const TOKEN: &str = "xoxb-test-token";

#[derive(Clone)]
struct Setup {
    base: String,
    timeout: Duration,
    retry: Retry,
}

impl Setup {
    fn new(server: &MockServer) -> Self {
        Self {
            base: server.uri(),
            timeout: Duration::from_secs(2),
            retry: Retry::none(),
        }
    }

    fn retry(self, retry: Retry) -> Self {
        Self { retry, ..self }
    }

    fn timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }
}

fn transports() -> Vec<bool> {
    [
        (cfg!(feature = "async"), false),
        (cfg!(feature = "blocking"), true),
    ]
    .into_iter()
    .filter_map(|(enabled, blocking)| enabled.then_some(blocking))
    .collect()
}

#[allow(unused_variables, unreachable_code)]
async fn send(
    blocking: bool,
    setup: Setup,
    message: Message,
    parent: Option<SentMessage>,
) -> Result<SentMessage, Error> {
    if blocking {
        #[cfg(feature = "blocking")]
        return tokio::task::spawn_blocking(move || {
            let bot = slack::blocking::SlackBot::new(TOKEN, "CDEFAULT")?
                .with_base_url(setup.base)
                .with_timeout(setup.timeout)
                .with_retry(setup.retry);
            match parent {
                Some(parent) => bot.reply(&parent, &message),
                None => bot.send(&message),
            }
        })
        .await
        .unwrap();
    }
    #[cfg(feature = "async")]
    {
        let bot = slack::SlackBot::new(TOKEN, "CDEFAULT")?
            .with_base_url(setup.base)
            .with_timeout(setup.timeout)
            .with_retry(setup.retry);
        return match parent {
            Some(parent) => bot.reply(&parent, &message).await,
            None => bot.send(&message).await,
        };
    }
    unreachable!()
}

#[allow(unused_variables, unreachable_code)]
async fn call(blocking: bool, setup: Setup, method: &'static str) -> Result<Value, Error> {
    if blocking {
        #[cfg(feature = "blocking")]
        return tokio::task::spawn_blocking(move || {
            slack::blocking::SlackBot::new(TOKEN, "CDEFAULT")?
                .with_base_url(setup.base)
                .call(method, &json!({}))
        })
        .await
        .unwrap();
    }
    #[cfg(feature = "async")]
    {
        return slack::SlackBot::new(TOKEN, "CDEFAULT")?
            .with_base_url(setup.base)
            .call(method, &json!({}))
            .await;
    }
    unreachable!()
}

fn ok(ts: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"ok": true, "channel": "CDEFAULT", "ts": ts}))
}

async fn requests(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect()
}

#[tokio::test]
async fn posts_literal_text_with_safe_defaults_and_threads_replies() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(body_json(json!({
                "channel": "COTHER",
                "text": "a_b *x* `z` &lt;@U123&gt; &amp;",
                "mrkdwn": false,
                "parse": "none",
                "link_names": false,
                "unfurl_links": false,
                "unfurl_media": false,
                "thread_ts": "1.1",
            })))
            .respond_with(ok("1.2"))
            .expect(1)
            .mount(&server)
            .await;

        let parent = SentMessage {
            channel: "COTHER".into(),
            ts: "1.1".into(),
        };
        let sent = send(
            blocking,
            Setup::new(&server),
            Message::plain("a_b *x* `z` <@U123> &"),
            Some(parent),
        )
        .await
        .unwrap();

        assert_eq!(sent.ts, "1.2");
    }
}

#[tokio::test]
async fn builder_messages_carry_blocks_and_fallback_text() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok("1.2"))
            .mount(&server)
            .await;
        let message = MessageBuilder::new("alert").code("error", "<boom>").build();

        send(blocking, Setup::new(&server), message.clone(), None)
            .await
            .unwrap();

        let body = &requests(&server).await[0];
        assert_eq!(body["text"], "alert\nerror:\n&lt;boom&gt;");
        assert_eq!(&body["blocks"], message.blocks().unwrap());
    }
}

#[tokio::test]
async fn rejected_blocks_fall_back_to_text() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(body_partial_json(
            json!({"blocks": [{"type": "rich_text"}]}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "msg_blocks_too_long"})),
        )
        .expect(1)
        .mount(&server)
        .await;
        Mock::given(method("POST"))
            .respond_with(ok("1.2"))
            .mount(&server)
            .await;

        let message = MessageBuilder::new("alert").kv("slot", 1).build();
        send(blocking, Setup::new(&server), message, None)
            .await
            .unwrap();

        let bodies = requests(&server).await;
        assert_eq!(bodies.len(), 2);
        assert!(bodies[1].get("blocks").is_none());
        assert_eq!(bodies[1]["text"], "alert\nslot: 1");
    }
}

#[tokio::test]
async fn failures_keep_what_slack_said() {
    for blocking in transports() {
        let cases = [
            (
                200,
                r#"{"ok":false,"error":"not_in_channel"}"#,
                "slack api error not_in_channel",
            ),
            (
                500,
                "upstream exploded",
                "slack http status 500: upstream exploded",
            ),
            (200, "not json", "malformed slack response"),
            (
                200,
                r#"{"ok":true,"channel":"C"}"#,
                "malformed slack response",
            ),
        ];
        for (status, body, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;

            let error = send(blocking, Setup::new(&server), Message::plain("hi"), None)
                .await
                .unwrap_err();

            assert!(error.to_string().starts_with(expected), "{error}");
        }
    }
}

#[tokio::test]
async fn rate_limits_report_retry_after() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "17"))
            .mount(&server)
            .await;

        let error = send(blocking, Setup::new(&server), Message::plain("hi"), None)
            .await
            .unwrap_err();

        assert_eq!(error.retry_after(), Some(Duration::from_secs(17)));
    }
}

#[tokio::test]
async fn timeout_is_an_uncertain_exchange_failure() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok("1.2").set_delay(Duration::from_millis(300)))
            .expect(1)
            .mount(&server)
            .await;

        let error = send(
            blocking,
            Setup::new(&server).timeout(Duration::from_millis(80)),
            Message::plain("hi"),
            None,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                error,
                Error::Transport {
                    stage: Stage::Exchange,
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.delivery_uncertain());
    }
}

#[tokio::test]
async fn refused_connection_is_a_connect_failure() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);

    for blocking in transports() {
        let setup = Setup {
            base: base.clone(),
            timeout: Duration::from_secs(2),
            retry: Retry::none(),
        };

        let error = send(blocking, setup, Message::plain("hi"), None)
            .await
            .unwrap_err();

        assert!(
            matches!(
                error,
                Error::Transport {
                    stage: Stage::Connect,
                    ..
                }
            ),
            "blocking={blocking}: {error}"
        );
        assert!(!error.delivery_uncertain());
    }
}

#[tokio::test]
async fn at_least_once_retries_until_sent() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ok("1.2"))
            .mount(&server)
            .await;

        let setup = Setup::new(&server).retry(Retry::at_least_once([Duration::ZERO; 3]));
        let sent = send(blocking, setup, Message::plain("hi"), None)
            .await
            .unwrap();

        assert_eq!(sent.ts, "1.2");
        assert_eq!(requests(&server).await.len(), 3);
    }
}

#[tokio::test]
async fn when_undelivered_does_not_resend_uncertain_failures() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let setup = Setup::new(&server).retry(Retry::when_undelivered([Duration::ZERO; 3]));
        let error = send(blocking, setup, Message::plain("hi"), None)
            .await
            .unwrap_err();

        assert!(error.delivery_uncertain());
        assert_eq!(requests(&server).await.len(), 1);
    }
}

#[tokio::test]
async fn retries_stop_at_permanent_failures() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": false, "error": "channel_not_found"})),
            )
            .mount(&server)
            .await;

        let setup = Setup::new(&server).retry(Retry::at_least_once([Duration::ZERO; 3]));
        let error = send(blocking, setup, Message::plain("hi"), None)
            .await
            .unwrap_err();

        assert_eq!(error.api_code(), Some("channel_not_found"));
        assert_eq!(requests(&server).await.len(), 1);
    }
}

#[tokio::test]
async fn redirects_are_not_followed() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", format!("{}/x", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let error = send(blocking, Setup::new(&server), Message::plain("hi"), None)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Http { status: 302, .. }), "{error}");
    }
}

#[tokio::test]
async fn call_returns_the_whole_response() {
    for blocking in transports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apps.connections.open"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok": true, "url": "wss://example.test"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let response = call(blocking, Setup::new(&server), "apps.connections.open")
            .await
            .unwrap();

        assert_eq!(response["url"], "wss://example.test");
    }
}

#[tokio::test]
async fn empty_messages_never_reach_the_network() {
    for blocking in transports() {
        let server = MockServer::start().await;

        let error = send(blocking, Setup::new(&server), Message::plain("  "), None)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Invalid(_)));
        assert!(requests(&server).await.is_empty());
    }
}

#[test]
fn bad_tokens_fail_at_construction() {
    #[cfg(feature = "async")]
    assert!(matches!(
        slack::SlackBot::new("xoxb token", "C1"),
        Err(Error::Invalid(_))
    ));
    #[cfg(feature = "blocking")]
    assert!(matches!(
        slack::blocking::SlackBot::new("", "C1"),
        Err(Error::Invalid(_))
    ));
}
