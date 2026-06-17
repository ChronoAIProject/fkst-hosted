---
---

Add the `fkst-control-plane-operation-manual` agent skill (`skills/fkst-control-plane-operation-manual/SKILL.md`): a code-accurate manual for driving the fkst control plane through the NyxID CLI, covering the five core flows (create a repo via goal trigger `create_new`, bootstrap `.fkst/`, trigger a substrate session with inline args, check sessions via goals + `active_session_id`, and stop a session) with the exact request body and response shape for each call. Documentation only — no backend, frontend, or API behavior changes.
