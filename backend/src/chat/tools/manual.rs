//! The `search_manual` tool: the concierge's only way to state a platform rule.
//!
//! Unlike every other tool this one never leaves the process — no dispatch, no bearer
//! token, no network. The manual is compiled in and identical for every user, so there is
//! nothing to authorize.

use std::sync::Arc;

use async_trait::async_trait;

use super::super::knowledge;
use super::super::llm::ToolDef;
use super::{required_str, ChatTool, ToolCtx, ToolError, ToolOutcome, ToolRegistry};

struct SearchManual;

#[async_trait]
impl ChatTool for SearchManual {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "search_manual".to_string(),
            description: "Search the fkst-hosted operator manual for how the platform WORKS: the \
                 trigger-issue grammar and its `###` sections, package and manifest \
                 references, work labels and collisions, every fkst-* status label and what \
                 clears it, the session lifecycle, environments, logs, the dashboard, \
                 deployment access, and the REST API. Use it for any \"how does X work\", \
                 \"what does this label mean\", or \"how do I …\" question, and to explain a \
                 live tool result against the documented rule. Returns the matching sections \
                 verbatim; when nothing matches it returns the table of contents so you can \
                 retry with better terms."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words describing the topic, e.g. \"unrouted assignee\", \
                                        \"work label collision\", \"environment secrets\".",
                    },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let query = required_str(&args, "query")?;
        let sections = knowledge::lookup(
            &query,
            knowledge::DEFAULT_MAX_SECTIONS,
            knowledge::DEFAULT_MAX_BYTES,
        );
        tracing::debug!(matches = sections.len(), "chat manual lookup");

        let mut result = serde_json::json!({
            "sections": sections
                .iter()
                .map(|section| serde_json::json!({
                    "id": section.id,
                    "title": section.title,
                    "content": section.body,
                }))
                .collect::<Vec<_>>(),
        });
        // The table of contents is included ONLY on a miss. On a hit it would be pure
        // noise in the model's context; on a miss it is the recovery path — the model can
        // see what the manual actually covers and search again with the right words.
        if sections.is_empty() {
            result["toc"] = knowledge::toc()
                .into_iter()
                .map(|(id, title)| serde_json::json!({ "id": id, "title": title }))
                .collect();
        }

        Ok(ToolOutcome {
            result_json: result,
            truncated: false,
            // In-process: there is no HTTP status to report.
            status: None,
        })
    }
}

/// Register the manual-search tool.
pub(super) fn register(registry: &mut ToolRegistry) {
    registry.register(Arc::new(SearchManual));
}

#[cfg(test)]
#[path = "manual_tests.rs"]
mod tests;
