---
"fkst-hosted": minor
---

Install the `codex` CLI into the pod image (operator-pinned `CODEX_VERSION`) and render a per-session codex `config.toml` selecting an LLM provider for every fkst-substrate run (issue #112). The default is the NyxID-proxied `chrono-llm` service (OpenAI Responses API, authenticated as the session user via the per-session `NYXID_ACCESS_TOKEN`); users can override it from the vault with structured provider fields (`CODEX_BASE_URL`/`CODEX_MODEL`/`CODEX_WIRE_API`/`CODEX_ENV_KEY` + an API-key secret) or a full raw `config.toml` (`CODEX_CONFIG_TOML`), with precedence raw > structured > default. The config is written into a per-session `CODEX_HOME` (0700 dir, 0600 file) layered as a platform-managed env var on the engine child. No fkst-substrate engine change; the provider API key never appears in the rendered config or logs. Adds `FKST_HOSTED_CODEX_MODEL` and `FKST_HOSTED_CHRONO_LLM_BASE_URL` config (fail-closed when blank).
