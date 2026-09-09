//! Proposal tools for the resources a session runs *on*: repositories, named environment
//! profiles, and the App installation.
//!
//! Same contract as the session proposals in [`super::propose`]: each tool VALIDATES a
//! draft, hands the typed proposal to the orchestrator out of band, and returns the model
//! only a lean acknowledgement. Nothing is written until the user confirms the card and
//! the SPA calls the pre-existing endpoint with their own token.
//!
//! `draft_environment_profile` is the one tool that touches a secret dimension, and it
//! handles it by NOT handling it: the model may name the secrets a profile needs, and the
//! confirmation card collects the values. The tool's schema has no field for a secret
//! value, so there is no path by which one could reach the model or the wire.

use std::sync::Arc;

use async_trait::async_trait;

use super::super::actions;
use super::super::llm::ToolDef;
use super::propose::{optional_bool, optional_list, presented, rejected, string_array};
use super::{optional_str, required_str, ChatTool, ToolCtx, ToolError, ToolOutcome, ToolRegistry};

/// Read an optional `{ "NAME": "value" }` object as ordered key/value pairs.
///
/// Ordered pairs rather than a map so the confirmation card renders the variables in the
/// order the model wrote them — the order the user reasoned about.
fn optional_string_map(
    args: &serde_json::Value,
    key: &str,
) -> Result<Vec<(String, String)>, ToolError> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Object(map)) => map
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|text| (name.clone(), text.to_string()))
                    .ok_or_else(|| {
                        ToolError::InvalidArgs(format!(
                            "{key}.{name} must be a string (numbers and booleans must be quoted)"
                        ))
                    })
            })
            .collect(),
        Some(_) => Err(ToolError::InvalidArgs(format!(
            "{key} must be an object of string values"
        ))),
    }
}

// ---- propose_create_repository -------------------------------------------

struct ProposeCreateRepository;

#[async_trait]
impl ChatTool for ProposeCreateRepository {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "propose_create_repository".to_string(),
            description:
                "Draft a NEW GitHub repository for the user to review and confirm. This does NOT \
                 create anything — it presents a card the user must confirm, which then creates \
                 the repository under their own GitHub account or an organization they belong \
                 to. The fkst App is NOT installed on a freshly created repository; tell the \
                 user they install it afterwards before a session can run there."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Repository name: 1-100 chars of [A-Za-z0-9._-]." },
                    "owner": { "type": "string", "description": "Organization to create under. Omit for the user's personal account." },
                    "private": { "type": "boolean", "description": "Create it private. Defaults to true — prefer private unless the user asked for a public repository." },
                    "description": { "type": "string", "description": "Optional short repository description." },
                },
                "required": ["name"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let name = required_str(&args, "name")?;
        let owner = optional_str(&args, "owner")?;
        // Private by default: creating a PUBLIC repository by accident is the mistake that
        // cannot be undone by flipping the flag back.
        let private = optional_bool(&args, "private")?.unwrap_or(true);
        let description = optional_str(&args, "description")?;
        Ok(
            match actions::propose_create_repository(owner, &name, private, description) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

// ---- draft_environment_profile -------------------------------------------

struct DraftEnvironmentProfile;

#[async_trait]
impl ChatTool for DraftEnvironmentProfile {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "draft_environment_profile".to_string(),
            description:
                "Draft a saved environment profile (create or replace) for the user to review and \
                 confirm. A profile is the reusable install-and-variables setup a trigger's \
                 `### Environment` section names. This does NOT save anything — the user confirms \
                 the card, and confirming runs the install commands in an isolated validation pod \
                 before anything is stored, so a failing command means nothing is saved. \
                 NEVER put a secret VALUE anywhere in this call: list the secret NAMES in \
                 `secret_names` and the user types the values into the card themselves. Replacing \
                 an existing profile REPLACES it wholesale, so include everything it should keep \
                 — call `get_environment_profile` first to read what it currently has."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile_name": {
                        "type": "string",
                        "description": "Profile name: lowercase letters, digits and inner dashes, 1-40 chars.",
                    },
                    "install": string_array(
                        "Ordered shell commands that install the toolchain, e.g. `npm ci`. At least one is required; they run in a validation pod on confirm.",
                    ),
                    "variables": {
                        "type": "object",
                        "description": "Non-secret environment variables as {\"NAME\": \"value\"}. Names must match ^[A-Za-z_][A-Za-z0-9_]*$. Never put a credential here — use `secret_names`.",
                        "additionalProperties": { "type": "string" },
                    },
                    "secret_names": string_array(
                        "NAMES ONLY of the secrets this profile needs, e.g. [\"NPM_TOKEN\"]. The user enters the values on the confirmation card; never send a value.",
                    ),
                },
                "required": ["profile_name", "install"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
        let profile_name = required_str(&args, "profile_name")?;
        let install = optional_list(&args, "install")?;
        let variables = optional_string_map(&args, "variables")?;
        let secret_names = optional_list(&args, "secret_names")?;

        // Look the profile up so the card can say "replace" rather than "create". A failed
        // or non-200 lookup yields `None`, and the card then states the ambiguity instead of
        // guessing — an over-claimed "replace" would be worse than an honest "unknown".
        let replaces_existing = existing_profile_names(ctx)
            .await
            .map(|names| names.iter().any(|name| name == &profile_name));

        Ok(
            match actions::propose_save_environment_profile(
                &profile_name,
                install,
                variables,
                secret_names,
                replaces_existing,
            ) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

/// The caller's existing profile names, or `None` when the lookup could not answer.
///
/// Deliberately swallows both the dispatch fault and a non-200: this is a COSMETIC lookup
/// feeding one word on the card, and failing the whole draft because the list endpoint was
/// briefly unavailable would be a worse trade than an honest "create/replace unknown".
async fn existing_profile_names(ctx: &ToolCtx) -> Option<Vec<String>> {
    let response = ctx
        .dispatch
        .get("/api/v1/users/me/environment-profiles", &ctx.bearer, None)
        .await
        .ok()?;
    if response.status != 200 {
        return None;
    }
    Some(
        response.body["environment_profiles"]
            .as_array()?
            .iter()
            .filter_map(|entry| entry["name"].as_str().map(str::to_string))
            .collect(),
    )
}

// ---- propose_delete_environment_profile ----------------------------------

struct ProposeDeleteEnvironmentProfile;

#[async_trait]
impl ChatTool for ProposeDeleteEnvironmentProfile {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "propose_delete_environment_profile".to_string(),
            description:
                "Propose deleting one of the user's saved environment profiles, for them to \
                 review and confirm. This does NOT delete anything. Deleting is permanent and \
                 the profile's secret values cannot be recovered; any trigger whose \
                 `### Environment` names it will stop resolving. Only draft this when the user \
                 clearly asked to remove that profile."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile_name": { "type": "string", "description": "The profile name exactly as list_environment_profiles reports it." },
                },
                "required": ["profile_name"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let profile_name = required_str(&args, "profile_name")?;
        Ok(
            match actions::propose_delete_environment_profile(&profile_name) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

// ---- propose_uninstall_app -----------------------------------------------

struct ProposeUninstallApp;

#[async_trait]
impl ChatTool for ProposeUninstallApp {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "propose_uninstall_app".to_string(),
            description:
                "Propose uninstalling the fkst GitHub App from one account, for the user to \
                 review and confirm. This does NOT uninstall anything. It is the widest action \
                 available: it removes fkst from EVERY repository on that account at once and \
                 stops every session running there. Only draft it on an explicit request naming \
                 the account, and always state the reason."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "owner": { "type": "string", "description": "The account (user or org login) to uninstall from." },
                    "reason": { "type": "string", "description": "Why uninstalling is being proposed; shown on the card." },
                },
                "required": ["owner", "reason"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let reason = required_str(&args, "reason")?;
        Ok(match actions::propose_uninstall_app(&owner, &reason) {
            Ok(proposal) => presented(proposal),
            Err(error) => rejected(error),
        })
    }
}

/// Register the resource proposal tools.
pub(super) fn register(registry: &mut ToolRegistry) {
    registry.register(Arc::new(ProposeCreateRepository));
    registry.register(Arc::new(DraftEnvironmentProfile));
    registry.register(Arc::new(ProposeDeleteEnvironmentProfile));
    // Last of the drafting tools: the widest-blast-radius action should not be the first
    // thing the model reads when it is scanning for something to do.
    registry.register(Arc::new(ProposeUninstallApp));
}

#[cfg(test)]
#[path = "propose_resources_tests.rs"]
mod tests;
