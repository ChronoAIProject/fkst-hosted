# SPEC: GitHub authored-content trust filter at the gh call ingress (whitelist system)

Status: accepted (sshx philosopher debate: 3 codex perspectives + ChatGPT Pro oracle, converged 2026-07-09)
Owner intent (user): "直接在 gh 调用的地方劫持掉,加入白名单系统,直接把未授权用户的信息完全隔离在系统之外.
直接在 gh 那里全部过滤了,下游可以对这个事情完全透明,因为未知者信息压根进不来." + "安全多做一层也没关系,那个 PR 也可以不 revert."

## 1. Problem / root cause

GitHub issue/PR/comment prose is attacker-controllable input (prompt-injection surface). Today the trust filter
(PR#2012, `devloop.content_provenance`) runs ONLY at the DOWNSTREAM codex-context-bundle boundary
(`devloop.context_bundle`). That is too late and too narrow: untrusted authored bytes have already crossed the `gh`
adapter boundary and can sit in shared entity views, caches, and any other consumer before the bundle filter runs.

Root cause (all 4 perspectives converged): **the trust boundary is the `gh` execution capability, not the codex
bundle.** The fix is to make unfiltered GitHub-authored prose unreachable to business code — redact non-whitelisted
authors' prose AT INGRESS so it never enters the system. Downstream then needs no knowledge of trust: untrusted
content is unrepresentable.

## 2. Verified ground truth (seek-truth-from-facts)

1. **There are TWO package-level `gh` egress wrappers, both delegating to the engine `exec_argv` primitive:**
   - `forge.github.exec.run` (via `handle._exec`) — used by all `forge.github` adapter reads/writes; feeds the codex
     bundle (`context_bundle.lua` → `M.gh_issue_view`/`M.gh_pr_view_context`) and `forge`-routed entity reads.
   - `devloop.gh_exec` (`libraries/devloop/gh_exec.lua`) — a thin `exec_argv` wrapper (argv[1] built via
     `table.concat({"g","h"})`, which evades the literal-`gh` G-ADAPTER scan). Used by `github-proxy`
     (`core.lua`, `core/blocked_by.lua`, `core/marker_guard.lua`), `devloop.github_proxy_entity_view`
     (`gh_exec_cached`), and `devloop.sweep_bounds`. **It reads authored comment bodies** and
     `github_proxy_entity_view`/`rest_view` reconstruct shared views carrying `body` + `comments[].body`.
   The engine `exec_argv` is the only true single chokepoint but lives in the substrate and is generic (not gh-only),
   so the package-layer answer is: mediate BOTH egress wrappers. "Filter at the gh call site" ⇒ filter at both.

2. **Dependency direction:** `forge` may depend ONLY on `contract`. `devloop` depends on `forge`. So the ONE filter
   mechanism must live in `forge` (reachable by both `forge.github.exec.run` and `devloop.gh_exec`).

3. **State-machine safety linchpin (verified):** markers are read ONLY from trusted comments
   (`devloop.parsers.misc.is_trusted_comment` = `author == bot_login`; used throughout `devloop.markers.facts`).
   The ingress whitelist (bot ∪ managed ∪ authorized) ⊇ marker-trust set (bot). Bot/whitelisted prose passes
   verbatim, so version-CAS / dependency-gate / merge-gate see identical trusted markers. Non-trusted prose is
   redacted — and the state machine already ignores non-trusted comments. Safe by construction, IF re-encode is faithful.

4. Existing filter mechanism (`content_provenance.lua`) is portable (deps: `contract.strings` + global `json.decode`),
   author-login-based, replaces prose with an un-spoofable `[fkst:blocked-github-content:v1 ...]` marker, and is
   byte-identical passthrough when nothing is redacted.

## 3. Design decision (converged concrete plan)

### 3.1 Central mediation with declared stdout capability (tension A)
Do NOT blind-filter all stdout (heuristic; fails open on `--include` header-prefixed authored bodies, corrupts
check-runs / `--name-only` / `--jq` scalars / write acks / GraphQL). Do NOT scatter per-adapter filter calls
(future-bypass bug). Instead: **every `gh` call declares a `stdout_policy`; the egress applies the filter iff the
policy is an authored-content shape; a missing policy is a hard error (fail-closed).**

- `stdout_policy ∈ { content_json(shape), trusted_metadata_json, plain_text, write_response, no_stdout }`.
- `content_json` shapes: `issue_view`, `pr_view`, `issue_comments`, `pr_comments`, `issue_list`, `pr_list`,
  `reviews` (+ `graphql` authored paths if/when added). Only `content_json` stdout is transformed.
- Applied at BOTH egress wrappers (`forge.github.exec.run` and `devloop.gh_exec`) using the ONE forge mechanism.

Bypass prevention is layered (repo "one canonical way + mechanically forbid bypass"):
- **Capability restriction:** no exported `gh` execution path returns stdout without a declared policy.
- **Runtime guard:** the egress rejects missing/unknown `stdout_policy` (fail-closed).
- **Conformance ratchet (`G-GITHUB-CONTENT-INGRESS`, replaces `G-GITHUB-CONTENT-GATE`):** fails on any raw
  `exec_argv({argv={"gh",...}})` / obfuscated gh head / `_exec`-style call outside the two mediated egress wrappers,
  and on any authored-content read that does not declare a `content_json` policy.

### 3.2 Policy via dependency injection (tension B) — forge never reads env
- `forge` owns the MECHANISM (parse gh JSON, find author-owned prose fields, redact untrusted bodies).
- `devloop` owns the POLICY: build a `trusted_author_policy` (whitelist) from env
  (`FKST_GITHUB_BOT_LOGIN` required anchor ∪ `FKST_DEVLOOP_MANAGED_BOT_LOGINS` ∪ `FKST_GITHUB_AUTHORIZED_LOGINS`)
  and inject it into `forge.github.new(exec, opts)` and `devloop.gh_exec(..., policy)`.
- Production wiring threads ONE policy through every production constructor of a gh handle
  (`forge.ports.production_handles`, `forge.github.production_handle`, github-proxy handle construction, etc.).
- Fail-closed: in production an authored-content read with NO injected policy is an error. Tests inject an explicit
  policy (or an explicit test-disabled policy). Forge treats the policy as opaque data (no `FKST_*` knowledge in forge).

### 3.3 Redaction form: marker replacement (tension C)
Replace each untrusted author's prose field (title/body/comment body/review body) with the existing un-spoofable
`[fkst:blocked-github-content:v1 ...]` marker. Preserve JSON shape, author.login, ids, timestamps, urls, array
length/order. Do NOT drop comment objects (perturbs counts/ordering/last-comment logic/pagination) and do NOT
erase-to-empty (ambiguous legitimate value; hides that a security decision occurred). Downstream MUST trust only
server-side `author.login`, never marker text. **Idempotent:** a field already holding a blocked marker is not
re-redacted (so an optional bundle-layer second pass cannot double-wrap).

### 3.4 Scope (tension D)
Filter every server-side-author-owned PROSE field that can reach codex or state parsing:
issue title+body (`issue.user/author.login`), PR title+body, issue/PR comment bodies, PR review bodies + review
comment bodies (when a reviews shape is fetched), and GraphQL-equivalent authored paths.
**Out of scope for this change (documented follow-ups):**
- **Intake trust** — that a non-whitelisted OPENER's issue must not autonomously drive work. Ingress filtering makes
  such an issue's body a `[blocked]` marker (visible metadata only); it does NOT weaken the filter to let task text
  through. Whether such an issue is *eligible* for work is a separate intake-trust policy (a later change): work
  begins only from trusted-author content/labels/bot records.
- **PR diff / patch trust** — `gh pr diff` is untrusted patch text with no per-field author login; the author
  redactor cannot sanitize it. It stays under the existing `UNTRUSTED-NOTICE.txt` bundle data-boundary warning
  (which is KEPT — it warns codex that requirements/diff/bundle files are untrusted data, not a duplicate whitelist).
  PR/head trust is gated separately.

### 3.5 content_provenance / PR#2012 (revert optional; ONE mechanism)
- Move the mechanism to `libraries/forge/github/content_filter.lua` (forge-owned). Generalize it from the fixed
  issue/pr `kind` shape to a recursive author-prose redactor over the supported gh shapes (object + `--slurp`
  nested arrays), keeping byte-identical passthrough + faithful re-encode + idempotency.
- `devloop.content_provenance` becomes a thin re-export of the forge module (or is deleted and `context_bundle`
  calls forge directly) — NEVER a second divergent copy.
- KEEP the bundle-boundary application (`context_bundle.lua`) as defense-in-depth (user: "多做一层没关系"), now
  calling the SAME forge mechanism. On already-ingress-filtered data it finds nothing to redact (harmless inner net).

## 4. Invariants (proof obligations — enforced by test)

- **I1 byte-identical passthrough:** when 0 redactions, egress stdout is byte-identical to raw gh stdout.
- **I2 state-machine fidelity:** after redaction, every NON-redacted field the state machine consumes is
  semantically unchanged. Golden + semantic-equality fixtures for: state marker, review-result, merge-ready,
  dependency (blockedBy), merged facts, version-CAS ordering, labels, assignees. Bot-authored marker bodies pass verbatim.
- **I3 shape fidelity:** empty arrays stay arrays (not `{}`); null stays null; large GitHub integer ids preserved
  exactly; Unicode preserved; `--slurp` nested-array nesting preserved. (This is the highest risk — the custom
  re-encoder's array/object ambiguity on empty tables.)
- **I4 idempotency:** filter(filter(x)) == filter(x).
- **I5 non-content untouched:** non-JSON stdout (diffs, `--name-only`, `--jq` scalars), write responses, and
  `trusted_metadata`/`no_stdout` policies are returned unchanged.
- **I6 fail-closed:** authored-content read with missing/unknown policy in production → error, not silent passthrough.
- **I7 no bypass:** ratchet green ⇒ no gh egress outside the two mediated wrappers; every authored read declares a policy.
- **I8 cache safety:** pre-change cached gh stdout / normalized views / context bundles cannot serve unfiltered
  content post-deploy — bump the relevant cache-key prefixes (`github-proxy/view`, `github-devloop` ghread,
  entity-list, context-bundle) so stale entries are re-fetched through the filter.

## 5. Test requirements
Golden fixtures per supported shape (issue_view, pr_view, issue/pr comments `--slurp`, issue/pr list, REST issue+
comments, GraphQL if added). For each: (a) all-trusted → byte-identical; (b) mixed → only untrusted prose becomes
markers, all else semantically equal; (c) idempotency; (d) empty-array/null/large-int/timestamp/nested/Unicode
fidelity. Plus: fail-closed on missing policy; non-JSON/write untouched; the state-machine parser suite passes on
redacted shared views; full `scripts/run.sh test` (single + composed conformance) + `check_repo.py` green.

## 6. Non-goals
Intake eligibility policy for untrusted openers; PR diff/patch content trust; substrate-level `exec_argv` mediation;
GraphQL authored-path coverage beyond what is actually read today (add shapes only when a real read path exists).

⟦AI:FKST⟧
