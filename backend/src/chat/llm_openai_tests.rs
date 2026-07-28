//! Transport + decoder tests for [`OpenAiCompatClient`] (sibling `#[path]` module,
//! mirroring the repo's wiremock style in
//! `session_backend/opensandbox/lifecycle_tests.rs`).
//!
//! Two layers are covered deliberately:
//! * whole-request behaviour through a real HTTP round trip (wiremock), including
//!   the exact JSON shape the provider receives;
//! * the SSE decoder driven directly with synthetic chunks, so frames split ACROSS
//!   network-chunk boundaries are exercised — something a buffered mock body cannot
//!   reproduce.

use axum::body::Bytes;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::llm::{ChatMessage, ToolDef};
use super::*;

const API_KEY: &str = "chat_secret_key_abc123";

fn client(base: &str) -> OpenAiCompatClient {
    OpenAiCompatClient::new(
        reqwest::Url::parse(base).expect("base url"),
        SecretString::from(API_KEY.to_string()),
    )
}

fn turn() -> TurnRequest {
    TurnRequest {
        model: "test-model".to_string(),
        messages: vec![
            ChatMessage::text(ChatRole::System, "be helpful"),
            ChatMessage::text(ChatRole::User, "list my sessions"),
        ],
        tools: Vec::new(),
    }
}

/// Collect a whole turn's items, failing the test on the first error.
async fn collect_items(client: &OpenAiCompatClient, req: TurnRequest) -> Vec<StreamItem> {
    let mut stream = client.stream_turn(req).await.expect("turn must start");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item.expect("stream item must not error"));
    }
    items
}

/// Expect a pre-stream failure. A plain `expect_err` cannot be used: the `Ok` side
/// is a boxed stream, which has no `Debug`.
async fn expect_turn_error(client: &OpenAiCompatClient, req: TurnRequest) -> LlmError {
    match client.stream_turn(req).await {
        Ok(_) => panic!("the turn must not start"),
        Err(error) => error,
    }
}

/// Build an SSE body from `data:` payloads (one frame each).
fn sse_body(payloads: &[&str]) -> String {
    payloads
        .iter()
        .map(|p| format!("data: {p}\n\n"))
        .collect::<String>()
}

/// Drive the decoder directly over caller-controlled chunk boundaries.
async fn decode_chunks(chunks: Vec<&str>) -> Vec<Result<StreamItem, LlmError>> {
    let owned: Vec<Result<Bytes, reqwest::Error>> = chunks
        .into_iter()
        .map(|c| Ok(Bytes::from(c.to_string())))
        .collect();
    let mut stream = decode_sse(futures::stream::iter(owned).boxed());
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    items
}

// ---- happy path -----------------------------------------------------------

#[tokio::test]
async fn streams_text_deltas_then_done() {
    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_body(&[
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"delta":{"content":" world"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ]),
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {API_KEY}")))
        .and(body_partial_json(serde_json::json!({
            "model": "test-model",
            "stream": true,
            "messages": [
                { "role": "system", "content": "be helpful" },
                { "role": "user", "content": "list my sessions" },
            ],
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let items = collect_items(&client(&format!("{}/v1", server.uri())), turn()).await;
    assert_eq!(
        items,
        vec![
            StreamItem::TextDelta("Hello".to_string()),
            StreamItem::TextDelta(" world".to_string()),
            StreamItem::Done {
                finish_reason: "stop".to_string()
            },
        ]
    );
}

#[tokio::test]
async fn base_url_with_trailing_slash_hits_the_same_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        // The load-bearing assertion: `/v1/` must not lose its `v1` segment (which
        // `Url::join` would do).
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .expect(1)
        .mount(&server)
        .await;

    let items = collect_items(&client(&format!("{}/v1/", server.uri())), turn()).await;
    assert!(items.is_empty(), "a bare [DONE] yields no items");
}

#[tokio::test]
async fn tools_are_serialized_in_the_openai_function_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(serde_json::json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_overview",
                    "description": "what the caller can see",
                    "parameters": { "type": "object", "properties": {} },
                },
            }],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .expect(1)
        .mount(&server)
        .await;

    let mut req = turn();
    req.tools = vec![ToolDef {
        name: "get_overview".to_string(),
        description: "what the caller can see".to_string(),
        parameters: serde_json::json!({ "type": "object", "properties": {} }),
    }];
    collect_items(&client(&format!("{}/v1", server.uri())), req).await;
}

#[tokio::test]
async fn assistant_tool_calls_and_tool_results_round_trip_to_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(serde_json::json!({
            "messages": [
                { "role": "user", "content": "hi" },
                {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "get_overview", "arguments": "{}" },
                    }],
                },
                { "role": "tool", "content": "{\"status\":200}", "tool_call_id": "call_1" },
            ],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .expect(1)
        .mount(&server)
        .await;

    let mut req = turn();
    req.messages = vec![
        ChatMessage::text(ChatRole::User, "hi"),
        ChatMessage::assistant_tool_calls(
            "",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_overview".to_string(),
                arguments_json: "{}".to_string(),
            }],
        ),
        ChatMessage::tool_result("call_1", "{\"status\":200}"),
    ];
    collect_items(&client(&format!("{}/v1", server.uri())), req).await;
}

// ---- tool-call fragment reassembly ---------------------------------------

#[tokio::test]
async fn fragmented_tool_call_arguments_reassemble_into_one_item() {
    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_body(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"tail_log_file","arguments":"{\"session"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"_id\":\"s1\",\"path\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"run.log\"}"}}]},"finish_reason":"tool_calls"}]}"#,
        ]),
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let items = collect_items(&client(&format!("{}/v1", server.uri())), turn()).await;
    let StreamItem::ToolCalls(calls) = items.into_iter().next().expect("one item must be produced")
    else {
        panic!("expected reassembled tool calls");
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_abc");
    assert_eq!(calls[0].name, "tail_log_file");
    // The whole point: the concatenated arguments are valid JSON again.
    let args: serde_json::Value =
        serde_json::from_str(&calls[0].arguments_json).expect("arguments must reassemble to json");
    assert_eq!(args["session_id"], "s1");
    assert_eq!(args["path"], "run.log");
}

#[tokio::test]
async fn parallel_tool_calls_keep_their_indexes_apart() {
    let items = decode_chunks(vec![
        &sse_body(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"get_overview","arguments":"{}"}},{"index":1,"id":"b","function":{"name":"list_log_runs","arguments":"{\"session_id\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":":\"s1\"}"}}]},"finish_reason":"tool_calls"}]}"#,
        ]),
        "data: [DONE]\n\n",
    ])
    .await;
    let Ok(StreamItem::ToolCalls(calls)) = items.into_iter().next().expect("one item") else {
        panic!("expected tool calls");
    };
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "get_overview");
    assert_eq!(calls[1].name, "list_log_runs");
    assert_eq!(calls[1].arguments_json, r#"{"session_id":"s1"}"#);
}

#[tokio::test]
async fn tool_calls_are_flushed_when_the_body_ends_without_a_finish_reason() {
    // Some providers close the body after the last fragment. Dropping the call
    // would silently lose the model's work.
    let items = decode_chunks(vec![
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"get_overview","arguments":"{}"}}]}}]}"#,
        "\n\n",
    ])
    .await;
    assert_eq!(items.len(), 1, "the pending call must still be emitted");
    let Ok(StreamItem::ToolCalls(calls)) = &items[0] else {
        panic!("expected tool calls, got {:?}", items[0]);
    };
    assert_eq!(calls[0].name, "get_overview");
}

// ---- decoder edge cases ---------------------------------------------------

#[tokio::test]
async fn frames_split_across_chunk_boundaries_decode_intact() {
    let items = decode_chunks(vec![
        r#"data: {"choices":[{"delta":{"content":"Hel"#,
        r#"lo"}}]}"#,
        "\n\ndata: [DONE]\n\n",
    ])
    .await;
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].as_ref().expect("no error"),
        &StreamItem::TextDelta("Hello".to_string())
    );
}

#[tokio::test]
async fn multibyte_characters_split_across_chunks_are_not_corrupted() {
    // "日" is three bytes; the split lands mid-character. Buffering BYTES (not a
    // lossy string) is what keeps this intact.
    let encoded = r#"data: {"choices":[{"delta":{"content":"日本"}}]}"#;
    let bytes = encoded.as_bytes();
    let split = encoded.find('日').expect("find") + 1;
    let head = String::from_utf8_lossy(&bytes[..split]).to_string();
    let tail = String::from_utf8_lossy(&bytes[split..]).to_string();
    // Sanity: the lossy split really is lossy, so the test would catch a
    // string-buffering regression.
    assert_ne!(format!("{head}{tail}"), encoded);

    let owned: Vec<Result<Bytes, reqwest::Error>> = vec![
        Ok(Bytes::from(bytes[..split].to_vec())),
        Ok(Bytes::from(bytes[split..].to_vec())),
        Ok(Bytes::from_static(b"\n\n")),
    ];
    let mut stream = decode_sse(futures::stream::iter(owned).boxed());
    let item = stream.next().await.expect("one item").expect("no error");
    assert_eq!(item, StreamItem::TextDelta("日本".to_string()));
}

#[tokio::test]
async fn crlf_frames_and_keep_alive_comments_are_handled() {
    let items = decode_chunks(vec![
        ": keep-alive\r\n\r\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\r\n\r\n",
        ": keep-alive\n\n",
        "data: [DONE]\n\n",
    ])
    .await;
    assert_eq!(items.len(), 1, "comment frames must be skipped");
    assert_eq!(
        items[0].as_ref().expect("no error"),
        &StreamItem::TextDelta("ok".to_string())
    );
}

#[tokio::test]
async fn multiple_data_lines_in_one_frame_are_concatenated() {
    let items = decode_chunks(vec![
        "data: {\"choices\":[{\"delta\":\ndata: {\"content\":\"x\"}}]}\n\n",
    ])
    .await;
    assert_eq!(
        items[0].as_ref().expect("no error"),
        &StreamItem::TextDelta("x".to_string())
    );
}

#[tokio::test]
async fn choiceless_chunks_are_ignored() {
    // Usage-only trailer chunks are common and carry nothing to emit.
    let items = decode_chunks(vec![
        r#"data: {"usage":{"total_tokens":7},"choices":[]}"#,
        "\n\ndata: [DONE]\n\n",
    ])
    .await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn malformed_json_payload_is_a_protocol_error() {
    let items = decode_chunks(vec!["data: {not json}\n\n"]).await;
    assert_eq!(items.len(), 1);
    match items.into_iter().next().expect("one item") {
        Err(LlmError::Protocol(message)) => {
            assert!(message.contains("json"), "message must explain: {message}")
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn everything_after_a_malformed_frame_is_abandoned() {
    let items = decode_chunks(vec![
        "data: {bad}\n\n",
        r#"data: {"choices":[{"delta":{"content":"never"}}]}"#,
        "\n\n",
    ])
    .await;
    assert_eq!(items.len(), 1, "the stream must stop at the protocol error");
    assert!(items[0].is_err());
}

// ---- api errors -----------------------------------------------------------

#[tokio::test]
async fn unauthorized_maps_to_an_api_error_carrying_the_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(r#"{"error":{"message":"invalid api key"}}"#),
        )
        .mount(&server)
        .await;

    let error = expect_turn_error(&client(&format!("{}/v1", server.uri())), turn()).await;
    match error {
        LlmError::Api { status, detail } => {
            assert_eq!(status, 401);
            assert!(
                detail.contains("invalid api key"),
                "provider detail must survive: {detail}"
            );
        }
        other => panic!("expected an api error, got {other:?}"),
    }
}

#[tokio::test]
async fn error_detail_is_bounded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("x".repeat(10_000)))
        .mount(&server)
        .await;

    let error = expect_turn_error(&client(&format!("{}/v1", server.uri())), turn()).await;
    match error {
        LlmError::Api { detail, .. } => assert_eq!(detail.len(), ERROR_DETAIL_MAX_BYTES),
        other => panic!("expected an api error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unreachable_provider_is_a_transport_error() {
    // Port 0 never accepts a connection, so this exercises the pre-stream failure
    // path without depending on a firewall or DNS behaviour.
    let error = expect_turn_error(&client("http://127.0.0.1:0/v1"), turn()).await;
    assert!(matches!(error, LlmError::Transport(_)), "got {error:?}");
}
