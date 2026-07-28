//! Tests for the model↔tools loop (sibling `#[path]` module).
//!
//! Every case scripts a stub [`ChatModelClient`] and (where a tool is involved) a
//! stub registry, so the assertions are about the LOOP — event order, message
//! threading, bounds, failure mapping — with no provider or router involved.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream::BoxStream;

use super::super::config::ChatConfig;
use super::super::llm::{ChatModelClient, ToolDef};
use super::super::test_support::{call, config, ctx, HangingClient, ScriptedClient};
use super::super::tools::{ChatTool, ToolOutcome, ToolRegistry};
use super::*;

/// A tool returning a fixed outcome, recording how often it was called.
struct StubTool {
    name: String,
    outcome: serde_json::Value,
    status: Option<u16>,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl ChatTool for StubTool {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: self.name.clone(),
            description: "a stub tool".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, super::super::tools::ToolError> {
        self.calls.lock().expect("calls").push(args);
        Ok(ToolOutcome {
            result_json: self.outcome.clone(),
            truncated: false,
            status: self.status,
        })
    }
}

fn user(text: &str) -> ChatClientMessage {
    ChatClientMessage {
        role: ChatClientRole::User,
        content: text.to_string(),
    }
}

fn assistant(text: &str) -> ChatClientMessage {
    ChatClientMessage {
        role: ChatClientRole::Assistant,
        content: text.to_string(),
    }
}

/// Run a turn and collect every emitted event.
async fn run(
    client: Arc<dyn ChatModelClient>,
    registry: ToolRegistry,
    messages: Vec<ChatClientMessage>,
    tweak: impl FnOnce(&mut ChatConfig),
) -> Vec<ChatStreamEvent> {
    let mut cfg = config();
    tweak(&mut cfg);
    let runtime = Arc::new(ChatRuntime::with_client(cfg, client, registry));
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    run_turn(runtime, ctx(), messages, tx).await;
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

/// A compact label per event, so an order assertion reads as the sequence it is.
fn shape(events: &[ChatStreamEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            ChatStreamEvent::Delta { text } => format!("delta({text})"),
            ChatStreamEvent::ToolCall { name, .. } => format!("tool_call({name})"),
            ChatStreamEvent::ToolResult { name, status, .. } => {
                format!("tool_result({name},{status})")
            }
            ChatStreamEvent::ActionProposal { .. } => "action_proposal".to_string(),
            ChatStreamEvent::Done { finish_reason, .. } => format!("done({finish_reason})"),
            ChatStreamEvent::Error { code, .. } => format!("error({code})"),
        })
        .collect()
}

// ---- pure text ------------------------------------------------------------

#[tokio::test]
async fn a_text_only_turn_streams_deltas_then_done() {
    let client = ScriptedClient::new(vec![Ok(vec![
        StreamItem::TextDelta("Hello".to_string()),
        StreamItem::TextDelta(" there".to_string()),
        StreamItem::Done {
            finish_reason: "stop".to_string(),
        },
    ])]);
    let events = run(client, ToolRegistry::new(), vec![user("hi")], |_| {}).await;
    assert_eq!(
        shape(&events),
        vec!["delta(Hello)", "delta( there)", "done(stop)"]
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_a_finish_reason_still_completes() {
    // Some providers just close the body. That is a completed answer, not an error.
    let client = ScriptedClient::new(vec![Ok(vec![StreamItem::TextDelta("ok".to_string())])]);
    let events = run(client, ToolRegistry::new(), vec![user("hi")], |_| {}).await;
    assert_eq!(shape(&events), vec!["delta(ok)", "done(stop)"]);
}

#[tokio::test]
async fn the_system_prompt_leads_the_conversation_and_the_client_history_follows() {
    let client = ScriptedClient::new(vec![Ok(vec![StreamItem::Done {
        finish_reason: "stop".to_string(),
    }])]);
    let probe = client.clone();
    run(
        client,
        ToolRegistry::new(),
        vec![user("first"), assistant("answer"), user("second")],
        |_| {},
    )
    .await;

    let requests = probe.requests();
    let roles: Vec<ChatRole> = requests[0].messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            ChatRole::System,
            ChatRole::User,
            ChatRole::Assistant,
            ChatRole::User
        ]
    );
    assert!(
        requests[0].messages[0].content.contains("fkst concierge"),
        "the platform system prompt must lead"
    );
    assert_eq!(requests[0].messages[3].content, "second");
}

// ---- tool round trips ----------------------------------------------------

fn registry_with(tool: StubTool) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));
    registry
}

#[tokio::test]
async fn one_tool_round_trip_emits_call_then_result_then_the_answer() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({ "status": 200, "body": { "accounts": [] } }),
        status: Some(200),
        calls: calls.clone(),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![
            StreamItem::TextDelta("Looking…".to_string()),
            StreamItem::ToolCalls(vec![call("c1", "get_overview", "{}")]),
        ]),
        Ok(vec![
            StreamItem::TextDelta("You have no accounts.".to_string()),
            StreamItem::Done {
                finish_reason: "stop".to_string(),
            },
        ]),
    ]);
    let probe = client.clone();
    let events = run(client, registry, vec![user("what do I have?")], |_| {}).await;

    assert_eq!(
        shape(&events),
        vec![
            "delta(Looking…)",
            "tool_call(get_overview)",
            "tool_result(get_overview,200)",
            "delta(You have no accounts.)",
            "done(stop)",
        ]
    );
    assert_eq!(calls.lock().expect("calls").len(), 1);

    // The second request must carry the assistant tool-call message and the tool
    // result, or the model cannot see what it learned.
    let second = &probe.requests()[1];
    let roles: Vec<ChatRole> = second.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            ChatRole::System,
            ChatRole::User,
            ChatRole::Assistant,
            ChatRole::Tool
        ]
    );
    assert_eq!(second.messages[3].tool_call_id.as_deref(), Some("c1"));
    assert!(second.messages[3].content.contains("accounts"));
}

#[tokio::test]
async fn every_registered_tool_is_advertised_to_the_model() {
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({}),
        status: None,
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![Ok(vec![StreamItem::Done {
        finish_reason: "stop".to_string(),
    }])]);
    let probe = client.clone();
    run(client, registry, vec![user("hi")], |_| {}).await;
    let tools: Vec<String> = probe.requests()[0]
        .tools
        .iter()
        .map(|t| t.name.clone())
        .collect();
    assert_eq!(tools, vec!["get_overview"]);
}

#[tokio::test]
async fn an_in_process_tool_without_a_status_reports_200() {
    let registry = registry_with(StubTool {
        name: "search_manual".to_string(),
        outcome: serde_json::json!({ "sections": [] }),
        // In-process tools have no HTTP status.
        status: None,
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "search_manual",
            r#"{"query":"x"}"#,
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let events = run(client, registry, vec![user("how?")], |_| {}).await;
    assert_eq!(
        shape(&events),
        vec![
            "tool_call(search_manual)",
            "tool_result(search_manual,200)",
            "done(stop)",
        ]
    );
}

#[tokio::test]
async fn a_denied_tool_call_reports_its_status_and_the_turn_continues() {
    let registry = registry_with(StubTool {
        name: "list_log_runs".to_string(),
        outcome: serde_json::json!({ "status": 403, "body": { "error": "forbidden" } }),
        status: Some(403),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "list_log_runs",
            r#"{"session_id":"s1"}"#,
        )])]),
        Ok(vec![
            StreamItem::TextDelta("You do not have log access.".to_string()),
            StreamItem::Done {
                finish_reason: "stop".to_string(),
            },
        ]),
    ]);
    let events = run(client, registry, vec![user("why did it fail?")], |_| {}).await;
    assert_eq!(
        shape(&events),
        vec![
            "tool_call(list_log_runs)",
            "tool_result(list_log_runs,403)",
            "delta(You do not have log access.)",
            "done(stop)",
        ]
    );
}

#[tokio::test]
async fn invalid_tool_arguments_are_reported_to_the_model_not_fatal() {
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "get_overview",
            "{not json",
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let probe = client.clone();
    let events = run(client, ToolRegistry::new(), vec![user("hi")], |_| {}).await;
    assert_eq!(
        shape(&events),
        vec![
            "tool_call(get_overview)",
            "tool_result(get_overview,200)",
            "done(stop)",
        ]
    );
    // The model must be TOLD what went wrong so it can retry within the turn.
    let tool_message = probe.requests()[1]
        .messages
        .last()
        .expect("a tool result message")
        .content
        .clone();
    assert!(
        tool_message.contains("invalid tool arguments"),
        "got {tool_message}"
    );
}

#[tokio::test]
async fn an_unknown_tool_name_is_reported_to_the_model_not_fatal() {
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "hallucinated_tool",
            "{}",
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let probe = client.clone();
    let events = run(client, ToolRegistry::new(), vec![user("hi")], |_| {}).await;
    assert_eq!(
        shape(&events).last().map(String::as_str),
        Some("done(stop)"),
        "a hallucinated tool name must not kill the turn"
    );
    let tool_message = probe.requests()[1]
        .messages
        .last()
        .expect("a tool result message")
        .content
        .clone();
    assert!(tool_message.contains("unknown tool"), "got {tool_message}");
}

#[tokio::test]
async fn a_zero_argument_call_with_an_empty_payload_still_runs() {
    // Providers emit `""` (not `"{}"`) for a call with no arguments.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({ "status": 200, "body": {} }),
        status: Some(200),
        calls: calls.clone(),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "get_overview",
            "",
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    run(client, registry, vec![user("hi")], |_| {}).await;
    assert_eq!(
        calls.lock().expect("calls").len(),
        1,
        "an empty arguments payload means no arguments"
    );
}

#[tokio::test]
async fn parallel_tool_calls_each_get_their_own_event_pair() {
    let mut registry = ToolRegistry::new();
    for name in ["get_overview", "list_environment_profiles"] {
        registry.register(Arc::new(StubTool {
            name: name.to_string(),
            outcome: serde_json::json!({ "status": 200, "body": {} }),
            status: Some(200),
            calls: Arc::new(Mutex::new(Vec::new())),
        }));
    }
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![
            call("c1", "get_overview", "{}"),
            call("c2", "list_environment_profiles", "{}"),
        ])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let events = run(client, registry, vec![user("hi")], |_| {}).await;
    assert_eq!(
        shape(&events),
        vec![
            "tool_call(get_overview)",
            "tool_result(get_overview,200)",
            "tool_call(list_environment_profiles)",
            "tool_result(list_environment_profiles,200)",
            "done(stop)",
        ]
    );
}

// ---- bounds ---------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn the_turn_deadline_ends_a_hanging_provider() {
    let events = run(
        Arc::new(HangingClient),
        ToolRegistry::new(),
        vec![user("hi")],
        |cfg| cfg.turn_deadline_secs = 10,
    )
    .await;
    assert_eq!(shape(&events), vec!["error(deadline_exceeded)"]);
}

#[tokio::test]
async fn exhausting_the_tool_budget_ends_the_turn_with_a_stable_code() {
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({ "status": 200, "body": {} }),
        status: Some(200),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    // The model calls a tool on every iteration and never answers.
    let turns = (0..2)
        .map(|_| {
            Ok(vec![StreamItem::ToolCalls(vec![call(
                "c1",
                "get_overview",
                "{}",
            )])])
        })
        .collect();
    let events = run(
        ScriptedClient::new(turns),
        registry,
        vec![user("hi")],
        |cfg| cfg.max_tool_iterations = 2,
    )
    .await;
    assert_eq!(
        shape(&events).last().map(String::as_str),
        Some("error(tool_budget_exhausted)")
    );
}

#[tokio::test]
async fn a_provider_failure_becomes_a_generic_error_frame() {
    let events = run(
        ScriptedClient::new(vec![Err(LlmError::Api {
            status: 401,
            detail: "invalid api key sk-secret".to_string(),
        })]),
        ToolRegistry::new(),
        vec![user("hi")],
        |_| {},
    )
    .await;
    assert_eq!(shape(&events), vec!["error(llm_error)"]);
    // The provider's detail can quote a key or a request; it must stay in the log.
    let ChatStreamEvent::Error { message, .. } = &events[0] else {
        panic!("expected an error frame");
    };
    assert!(
        !message.contains("sk-secret"),
        "provider detail must not reach the client: {message}"
    );
}

#[tokio::test]
async fn a_mid_stream_provider_failure_becomes_an_error_frame() {
    struct FailingMidStream;
    #[async_trait]
    impl ChatModelClient for FailingMidStream {
        async fn stream_turn(
            &self,
            _req: TurnRequest,
        ) -> Result<BoxStream<'static, Result<StreamItem, LlmError>>, LlmError> {
            Ok(futures::stream::iter(vec![
                Ok(StreamItem::TextDelta("partial".to_string())),
                Err(LlmError::Transport("connection reset".to_string())),
            ])
            .boxed())
        }
    }
    let events = run(
        Arc::new(FailingMidStream),
        ToolRegistry::new(),
        vec![user("hi")],
        |_| {},
    )
    .await;
    assert_eq!(shape(&events), vec!["delta(partial)", "error(llm_error)"]);
}

// ---- history truncation --------------------------------------------------

#[test]
fn an_over_long_history_is_truncated_oldest_first() {
    let history: Vec<ChatClientMessage> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                user(&format!("u{i}"))
            } else {
                assistant(&format!("a{i}"))
            }
        })
        .collect();
    let kept = truncate_history(history, 4);
    assert_eq!(kept.len(), 4);
    assert_eq!(kept[0].content, "u6", "the oldest messages must be dropped");
    assert_eq!(kept[3].content, "a9");
}

#[test]
fn a_history_within_the_cap_is_untouched() {
    let history = vec![user("a"), assistant("b"), user("c")];
    let kept = truncate_history(history, 40);
    assert_eq!(kept.len(), 3);
}

#[tokio::test]
async fn truncation_keeps_the_users_question_last() {
    // The invariant the endpoint validates on input must survive truncation, or the
    // model would be asked to continue from its own last answer.
    let mut history: Vec<ChatClientMessage> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                user(&format!("u{i}"))
            } else {
                assistant(&format!("a{i}"))
            }
        })
        .collect();
    history.push(user("the real question"));

    let client = ScriptedClient::new(vec![Ok(vec![StreamItem::Done {
        finish_reason: "stop".to_string(),
    }])]);
    let probe = client.clone();
    run(client, ToolRegistry::new(), history, |cfg| {
        cfg.history_max_messages = 5
    })
    .await;

    let messages = &probe.requests()[0].messages;
    // 1 system + 5 kept history messages.
    assert_eq!(messages.len(), 6);
    assert_eq!(messages[5].role, ChatRole::User);
    assert_eq!(messages[5].content, "the real question");
}

// ---- session references --------------------------------------------------

fn sessions_body(owner: &str, name: &str, triggers: &[i64]) -> serde_json::Value {
    let sessions: Vec<serde_json::Value> = triggers
        .iter()
        .map(|n| {
            serde_json::json!({
                "session_id": format!("sess-{n}"),
                "name": format!("session {n}"),
                "status_labels": ["fkst-substrate-active", "fkst-picked-up"],
                "trigger": { "number": n },
            })
        })
        .collect();
    serde_json::json!({
        "status": 200,
        "body": { "owner": owner, "name": name, "installed": true, "sessions": sessions },
    })
}

#[tokio::test]
async fn a_sessions_tool_result_yields_deduped_session_refs() {
    let registry = registry_with(StubTool {
        name: "list_repo_sessions".to_string(),
        // The same two sessions on both calls, so dedup is exercised.
        outcome: sessions_body("Acme", "Site", &[7, 9]),
        status: Some(200),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![
            call(
                "c1",
                "list_repo_sessions",
                r#"{"owner":"acme","name":"site"}"#,
            ),
            call(
                "c2",
                "list_repo_sessions",
                r#"{"owner":"acme","name":"site"}"#,
            ),
        ])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let events = run(client, registry, vec![user("what is running?")], |_| {}).await;

    let ChatStreamEvent::Done { session_refs, .. } = events.last().expect("a done frame") else {
        panic!("expected a done frame, got {:?}", events.last());
    };
    assert_eq!(session_refs.len(), 2, "duplicates must collapse");
    assert_eq!(
        session_refs[0],
        SessionRef {
            // The response's canonical casing wins over the caller's arguments.
            owner: "Acme".to_string(),
            name: "Site".to_string(),
            session_id: Some("sess-7".to_string()),
            trigger_number: 7,
            title: Some("session 7".to_string()),
            status_label: Some("fkst-substrate-active".to_string()),
        }
    );
}

#[tokio::test]
async fn a_turn_without_a_sessions_tool_yields_no_refs() {
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({ "status": 200, "body": { "accounts": [] } }),
        status: Some(200),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "get_overview",
            "{}",
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let events = run(client, registry, vec![user("hi")], |_| {}).await;
    let ChatStreamEvent::Done { session_refs, .. } = events.last().expect("a done frame") else {
        panic!("expected a done frame");
    };
    assert!(session_refs.is_empty());
}

#[tokio::test]
async fn a_failed_sessions_lookup_yields_no_refs() {
    // A card must never be built from an unauthorized or failed read.
    let registry = registry_with(StubTool {
        name: "list_repo_sessions".to_string(),
        outcome: serde_json::json!({ "status": 403, "body": { "error": "forbidden" } }),
        status: Some(403),
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let client = ScriptedClient::new(vec![
        Ok(vec![StreamItem::ToolCalls(vec![call(
            "c1",
            "list_repo_sessions",
            r#"{"owner":"acme","name":"site"}"#,
        )])]),
        Ok(vec![StreamItem::Done {
            finish_reason: "stop".to_string(),
        }]),
    ]);
    let events = run(client, registry, vec![user("hi")], |_| {}).await;
    let ChatStreamEvent::Done { session_refs, .. } = events.last().expect("a done frame") else {
        panic!("expected a done frame");
    };
    assert!(session_refs.is_empty());
}

#[test]
fn session_refs_are_capped() {
    let mut refs = Vec::new();
    let body = sessions_body("acme", "site", &(1..=20).collect::<Vec<i64>>());
    collect_session_refs(
        &mut refs,
        "list_repo_sessions",
        &serde_json::json!({"owner":"acme","name":"site"}),
        &body,
    );
    assert_eq!(refs.len(), MAX_SESSION_REFS);
}

#[test]
fn a_session_without_a_trigger_number_is_skipped() {
    // Without a trigger number there is nothing to link to.
    let mut refs = Vec::new();
    let body = serde_json::json!({
        "status": 200,
        "body": {
            "owner": "acme", "name": "site",
            "sessions": [{ "session_id": "s1", "name": "n", "trigger": {} }],
        },
    });
    collect_session_refs(
        &mut refs,
        "list_repo_sessions",
        &serde_json::json!({}),
        &body,
    );
    assert!(refs.is_empty());
}

// ---- client disconnect ---------------------------------------------------

#[tokio::test]
async fn a_disconnected_client_aborts_the_turn_promptly() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(StubTool {
        name: "get_overview".to_string(),
        outcome: serde_json::json!({ "status": 200, "body": {} }),
        status: Some(200),
        calls: calls.clone(),
    });
    let client = ScriptedClient::new(vec![Ok(vec![
        StreamItem::TextDelta("first".to_string()),
        StreamItem::ToolCalls(vec![call("c1", "get_overview", "{}")]),
    ])]);
    let runtime = Arc::new(ChatRuntime::with_client(config(), client, registry));

    // A closed receiver stands in for the browser going away.
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(rx);
    run_turn(runtime, ctx(), vec![user("hi")], tx).await;

    assert!(
        calls.lock().expect("calls").is_empty(),
        "no tool should run after the client disconnects"
    );
}

// ---- previews -----------------------------------------------------------

#[test]
fn the_args_preview_is_bounded_by_characters() {
    let long = "é".repeat(500);
    let preview = truncate_chars(&long, ARGS_PREVIEW_MAX_CHARS);
    assert_eq!(preview.chars().count(), ARGS_PREVIEW_MAX_CHARS);
    assert!(
        preview.chars().all(|c| c == 'é'),
        "a multi-byte character must never be split"
    );
}

#[test]
fn a_short_args_payload_is_previewed_verbatim() {
    assert_eq!(
        truncate_chars(r#"{"owner":"acme"}"#, ARGS_PREVIEW_MAX_CHARS),
        r#"{"owner":"acme"}"#
    );
}
