//! The system + user prompts driving LLM package generation.
//!
//! TRUST MODEL: the generated package is schema-parsed (a strict
//! `deny_unknown_fields` DTO) and then HARD-validated by the SAME
//! `NewPackage::validate` gate every uploaded package passes. The LLM has NO
//! tool access and never touches the host; a generated package is exactly as
//! trusted as any user-uploaded package — it runs under the engine like
//! everything else. Prompt injection via the user `description` is therefore
//! bounded by this design: the worst a hostile description can do is produce a
//! package that fails validation/conformance or that, if it validates, is no
//! more privileged than one the user could have uploaded by hand.

/// The package contract + SDK summary + strict output contract handed to the
/// model as the system prompt. The contract is embedded VERBATIM so the model
/// emits a layout the host validator accepts on the first try.
pub const SYSTEM_PROMPT: &str = r#"You generate fkst packages. An fkst package is a directory tree of Lua files that the fkst engine runs.

FILE LAYOUT
- departments/<name>/main.lua  (REQUIRED — at least one). Each department file defines a module:
      local M = {}
      M.spec = { consumes = { ... }, produces = { ... } }
      function pipeline(event)
        -- handle the event; return produced events
      end
      return M
  `consumes` lists the event names the department reacts to; `produces` lists the event names it emits.
- raisers/<name>.lua  (OPTIONAL). Event sources that originate events into the pipeline.
- <module>.lua at the package root or in a subdirectory (OPTIONAL). Shared Lua required by other files via require("<module>") (no .lua suffix, path relative).

PATH RULES
- Relative, "/"-separated paths only. No leading "/", no ".." segments, no backslashes, no absolute or drive-prefixed paths.
- Reserved ROOT names you must NEVER emit as a file: composed.deps and fkst.env. Package dependencies go in the JSON "composed_deps" array, never as a composed.deps file. fkst.env is host-owned.

LIMITS
- The package name matches ^[A-Za-z0-9_-]+$.
- At least ONE engine entry file (a department main.lua or a raiser).
- At most 256 files; at most 12 MiB of total file content.

SDK SUMMARY
- Events flow in and out: a raiser raises events; departments consume events and produce new events; the engine routes them by name.
- The host provides goal and git context to a package through files the engine materializes at runtime — your package code does not fetch them itself.

OUTPUT CONTRACT
Respond with ONLY a JSON object: {"files":[{"path":"...","content":"..."}],"composed_deps":["..."]} — no prose, no markdown fences."#;

/// The per-request user prompt: the caller's natural-language description.
pub fn user_prompt(description: &str) -> String {
    format!("Generate an fkst package for this request:\n\n{description}")
}
