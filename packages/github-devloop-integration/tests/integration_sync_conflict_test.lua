local h = require("tests.devloop_helpers")
local cache_seed_helpers = require("tests.cache_seed_helpers")
local t = h.t
local core = h.core
local _ = cache_seed_helpers

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function command_output(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok, _, status = handle:close()
  return {
    exit_code = ok and 0 or (status or 1),
    output = output or "",
  }
end

local function read_command(command)
  local result = command_output(command)
  if result.exit_code ~= 0 then
    error("sync_conflict fixture command failed: " .. tostring(command) .. "\n" .. tostring(result.output))
  end
  return result.output
end

local function run_command(command)
  read_command(command)
end

local function repo_root()
  return (read_command("pwd"):gsub("%s+$", ""))
end

local function temp_root(name)
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-sync-conflict-" .. tostring(name) .. ".XXXXXX")):gsub("%s+$", ""))
end

local function render_argv(argv)
  local parts = {}
  for _, arg in ipairs(argv) do
    table.insert(parts, shell_quote(arg))
  end
  return table.concat(parts, " ")
end

local function write_restart_lifecycle_fixture(root, source, mode)
  run_command("python3 - " .. shell_quote(root) .. " " .. shell_quote(source) .. " " .. shell_quote(mode) .. [[ <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
source = Path(sys.argv[2])
mode = sys.argv[3]
sys.path.insert(0, str(source / "scripts"))
import check_repo_restart_lifecycle as ratchet

target = root / "packages/github-devloop/departments/loop"
target.mkdir(parents=True, exist_ok=True)
(target / "main.lua").write_text(
    'local function pipeline() return "pipeline" end\nreturn { pipeline = pipeline }\n',
    encoding="utf-8",
)
(root / "migration").mkdir(parents=True, exist_ok=True)
(root / ratchet.ALLOWLIST).write_text("writer:1\n", encoding="utf-8")

def observation(observation_id, version):
    return {
        "schema": ratchet.OBS_SCHEMA,
        "observation_id": observation_id,
        "owner": "github-devloop",
        "site": {
            "path": "packages/github-devloop/departments/loop/main.lua",
            "symbol": "pipeline",
            "ordinal": observation_id,
        },
        "boundary": "writer",
        "typed_intent": {
            "kind": "state-transition",
            "source_state": "thinking",
            "source_boundary": "writer",
            "target": "blocked",
            "cause_schema_id": "state-marker.v1",
            "generation_epoch": {"generation": "1", "epoch": "1"},
            "lineage": {"proposal_id": observation_id},
        },
        "old_inputs": {
            "current_fact": {"state": "thinking"},
            "caller_from_states": ["thinking"],
            "incoming_version": version,
            "target_version": version + "-next",
            "handoff_reference": None,
        },
        "old_outcome": {
            "status": "ok",
            "reason_code": "ok",
            "cas_outcome": "applied",
            "emitted_effects": [{
                "effect_id": "effect-" + observation_id,
                "sink_kind": "comment",
                "authority_class": "lifecycle-authoritative",
                "ordinal": 1,
            }],
            "observable_writes": [{"kind": "comment"}],
            "handoff_direct_lookup_count": 0,
            "timeout_evidence_source": None,
        },
        "evidence_refs": [{"kind": "fixture", "ref": "old-execution:" + observation_id}],
    }

inventory_path = root / ratchet.INVENTORY
if inventory_path.exists():
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
else:
    inventory = {
        "schema": ratchet.SCHEMA,
        "version": 1,
        "source_tree": ["packages/github-devloop/departments/loop/main.lua"],
        "old_behavior_observations": [observation("obs-base", "v1")],
        "old_pending_projection": [],
        "production_writer_sites": [{
            "site_id": "writer:1",
            "path": "packages/github-devloop/departments/loop/main.lua",
            "symbol": "pipeline",
            "ordinal": "writer",
        }],
        "effect_sink_sites": [],
        "row_replay_sites": [],
        "published_intent_sites": [],
        "receiver_activation_acceptors": [],
        "consumer_entry_acceptors": [],
        "direct_constructor_sites": [],
        "shared_issue_row_exports": [],
        "ops_issue_row_reader_sites": [],
        "owner_observation_fact_sites": [],
        "grantless_sink_sites": [],
        "unobserved_sites": [{
            "site_id": "writer:1",
            "category": "production_writer_sites",
            "path": "packages/github-devloop/departments/loop/main.lua",
            "symbol": "pipeline",
            "ordinal": "writer",
            "why": "base",
        }],
        "watched_files": ["packages/github-devloop/departments/loop/main.lua"],
    }

if mode == "dev":
    inventory["old_behavior_observations"].append(observation("obs-dev", "v-dev"))
elif mode == "integration":
    (root / "docs").mkdir(parents=True, exist_ok=True)
    (root / "docs/integration-note.md").write_text("integration note\n", encoding="utf-8")
    inventory["watched_files"].append("docs/integration-note.md")
elif mode != "base":
    raise SystemExit("unknown mode: " + mode)

inventory["artifact_sha256"] = ratchet.artifact_sha256_for_document(inventory)
ratchet.write_inventory(root, inventory)
PY
]])
end

local function resolve_checksum_conflict_with_wrong_hash(path)
  run_command("python3 - " .. shell_quote(path) .. [[ <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
out = []
i = 0
conflicts = 0
while i < len(lines):
    if lines[i].startswith("<<<<<<< "):
        conflicts += 1
        ours = []
        i += 1
        while i < len(lines) and not lines[i].startswith("======="):
            ours.append(lines[i])
            i += 1
        i += 1
        while i < len(lines) and not lines[i].startswith(">>>>>>>"):
            i += 1
        i += 1
        out.extend(ours)
    else:
        out.append(lines[i])
        i += 1
if conflicts != 1:
    raise SystemExit(f"expected exactly one conflict block, found {conflicts}")
path.write_text("\n".join(out) + "\n", encoding="utf-8")
PY
]])
end

local function event(extra)
  local payload = {
    schema = "github-devloop.v1",
    repo = "owner/repo",
    upstream_branch = "dev",
    integration_branch = "integration/dev",
    upstream_sha = "aaaa1111",
    integration_sha = "bbbb2222",
    dedup_key = core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "aaaa1111"),
    source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev"),
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

local function opts(name, write)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_WRITE = write or "1",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    },
  }
end

local function run_conflict(payload, run_opts)
  return t.run_department("departments/sync_conflict/main.lua", {
    queue = "devloop_sync_conflict",
    payload = payload or event(),
  }, run_opts or opts("sync-conflict"))
end

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = value,
    },
  }, run_opts)
end

local function mock_fetch_and_heads(upstream_sha, integration_sha)
  t.mock_command("git fetch 'origin' 'dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'dev'^{commit}", { stdout = (upstream_sha or "aaaa1111") .. "\n", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (integration_sha or "bbbb2222") .. "\n", stderr = "", exit_code = 0 })
end

local function mock_conflicting_worktree(unmerged_stdout)
  t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-rt", stderr = "", exit_code = 0 })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --no-ff --no-commit", { stdout = "", stderr = "conflict", exit_code = 1 })
  t.mock_command("ls-files -u", { stdout = unmerged_stdout or "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
end

local function mock_successful_codex_resolution()
  t.mock_command("codex exec", { stdout = "resolved", stderr = "", exit_code = 0 })
  t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("commit -F", { stdout = "[detached cccc3333] Sync dev into integration/dev\n", stderr = "", exit_code = 0 })
end

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

local function self_hash_normalizer_call()
  for _, call in ipairs(t.command_calls()) do
    if call.program == "python3"
      and tostring((call.args or {})[2] or ""):find("check_repo_restart_lifecycle.py", 1, true) ~= nil then
      return call
    end
  end
  return nil
end

local function assert_sync_conflict_worktree_call()
  local calls = codex_calls()
  t.eq(#calls, 1)
  t.is_true(calls[1].rendered:find(" -C ", 1, true) ~= nil)
  t.is_true(calls[1].rendered:find("/worktrees/sync-owner-repo-dev-integration-dev-", 1, true) ~= nil)
  t.is_nil(calls[1].rendered:find("/judgment-worktrees/", 1, true))
  t.is_true(calls[1].stdin:find("isolated runtime branch-sync worktree", 1, true) ~= nil)
  t.is_true(calls[1].stdin:find("not the supervise source checkout", 1, true) ~= nil)
  t.is_true(calls[1].stdin:find("Do not clone, checkout another branch", 1, true) ~= nil)
end

local function mock_real_push(integration_recheck, pushed_head)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (integration_recheck or "bbbb2222") .. "\n", stderr = "", exit_code = 0 })
  if integration_recheck == nil or integration_recheck == "bbbb2222" then
    t.mock_command("rev-parse HEAD", { stdout = (pushed_head or "cccc3333") .. "\n", stderr = "", exit_code = 0 })
    t.mock_command("push origin HEAD:refs/heads/", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (pushed_head or "cccc3333") .. "\n", stderr = "", exit_code = 0 })
  end
end

local function mock_cleanup()
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_sync_conflict_real_self_hashed_manifest_divergence_recomputes_artifact_sha = function()
    local source = repo_root()
    local root = temp_root("self-hash")
    run_command("git init -b integration/dev " .. shell_quote(root))
    run_command("git -C " .. shell_quote(root) .. " config user.email fkst-test@example.invalid")
    run_command("git -C " .. shell_quote(root) .. " config user.name fkst-test")
    write_restart_lifecycle_fixture(root, source, "base")
    run_command("git -C " .. shell_quote(root) .. " add .")
    run_command("git -C " .. shell_quote(root) .. " commit -m " .. shell_quote("base inventory"))

    run_command("git -C " .. shell_quote(root) .. " switch -c dev")
    write_restart_lifecycle_fixture(root, source, "dev")
    run_command("git -C " .. shell_quote(root) .. " add .")
    run_command("git -C " .. shell_quote(root) .. " commit -m " .. shell_quote("dev inventory observation"))

    run_command("git -C " .. shell_quote(root) .. " switch integration/dev")
    write_restart_lifecycle_fixture(root, source, "integration")
    run_command("git -C " .. shell_quote(root) .. " add .")
    run_command("git -C " .. shell_quote(root) .. " commit -m " .. shell_quote("integration inventory observation"))

    local merge = command_output("git -C " .. shell_quote(root) .. " merge --no-ff --no-commit dev")
    t.is_true(merge.exit_code ~= 0, merge.output)
    t.eq(
      read_command("git -C " .. shell_quote(root) .. " ls-files -u | cut -f2 | sort -u"),
      "migration/restart-lifecycle.inventory.json\n"
    )

    local inventory_path = root .. "/migration/restart-lifecycle.inventory.json"
    resolve_checksum_conflict_with_wrong_hash(inventory_path)
    run_command("git -C " .. shell_quote(root) .. " add " .. shell_quote("migration/restart-lifecycle.inventory.json"))
    t.eq(read_command("git -C " .. shell_quote(root) .. " ls-files -u"), "")

    local before = command_output(
      "python3 "
        .. shell_quote(source .. "/scripts/check_repo_restart_lifecycle.py")
        .. " --root "
        .. shell_quote(root)
    )
    t.is_true(before.exit_code ~= 0, before.output)
    t.is_true(before.output:find("artifact_sha256 mismatch", 1, true) ~= nil, before.output)

    run_command(render_argv(core.sync_conflict_self_hash_normalizer_argv(
      source,
      root,
      "migration/restart-lifecycle.inventory.json"
    )))

    local after = command_output(
      "python3 "
        .. shell_quote(source .. "/scripts/check_repo_restart_lifecycle.py")
        .. " --root "
        .. shell_quote(root)
    )
    t.eq(after.exit_code, 0, after.output)
    t.is_true(after.output:find("self-hash-matched", 1, true) ~= nil, after.output)
  end,

  test_sync_conflict_codex_success_commits_and_guarded_pushes = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_successful_codex_resolution()
    mock_real_push()
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-success", "1"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("codex exec"), 1)
    assert_sync_conflict_worktree_call()
    t.eq(h.count_calls("ls-files -u"), 3)
    t.eq(h.count_calls("commit -F"), 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 1)
  end,

  test_sync_conflict_normalizes_self_hashed_manifest_after_codex = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree("100644 abc 1\tmigration/restart-lifecycle.inventory.json\n")
    t.mock_command("codex exec", { stdout = "resolved", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("pwd", { stdout = "/trusted/fkst-packages\n", stderr = "", exit_code = 0 })
    t.mock_command("python3 -B", { stdout = "OK: restart lifecycle inventory is schema-valid, shrink-only, independent, and self-hash-matched\n", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("commit -F", { stdout = "[detached cccc3333] Sync dev into integration/dev\n", stderr = "", exit_code = 0 })
    mock_real_push()
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-self-hash", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("codex exec"), 1)
    t.eq(h.count_calls("python3 -B"), 1)
    local normalizer = self_hash_normalizer_call()
    t.is_true(normalizer ~= nil)
    t.eq(normalizer.args[3], "--root")
    t.is_true(normalizer.args[4]:find("/worktrees/sync-owner-repo-dev-integration-dev-", 1, true) ~= nil)
    t.is_nil(normalizer.args[2]:find(normalizer.args[4], 1, true))
    t.eq(h.count_calls("commit -F"), 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 1)
  end,

  test_sync_conflict_codex_failure_errors_without_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "", stderr = "failed", exit_code = 1 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-codex-failure", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_leftover_conflict_errors_without_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-leftover", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_leftover_conflict_at_attempt_cap_escalates_without_failure = function()
    local payload = event()
    local remaining = "100644 abc 1\tcore.lua\n"
    local fingerprint = core.sync_conflict_fingerprint(payload, remaining)
    local run_opts = opts("sync-conflict-leftover-terminal", "1")
    seed_cache(core.sync_conflict_attempt_key(payload, fingerprint), tostring(core.max_sync_conflict_attempts() - 1), run_opts)
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = remaining, stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(payload, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
    t.eq(h.count_calls("commit -F"), 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.is_true(create.payload.body:find("Attempt: " .. tostring(core.max_sync_conflict_attempts()), 1, true) ~= nil)
    t.is_true(create.payload.body:find("Reason: sync conflict remains unresolved after codex completed", 1, true) ~= nil)
    t.is_true(create.payload.dedup_key:find("sync-conflict-escalation", 1, true) ~= nil)
  end,

  test_sync_conflict_attempt_cap_escalates_before_codex = function()
    local payload = event()
    local remaining = "100644 abc 1\tcore.lua\n"
    local fingerprint = core.sync_conflict_fingerprint(payload, remaining)
    local run_opts = opts("sync-conflict-pre-codex-terminal", "1")
    seed_cache(core.sync_conflict_attempt_key(payload, fingerprint), tostring(core.max_sync_conflict_attempts()), run_opts)
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_cleanup()

    local result = run_conflict(payload, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("codex exec"), 0)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.is_true(create.payload.body:find("Attempt: " .. tostring(core.max_sync_conflict_attempts()), 1, true) ~= nil)
    t.is_true(create.payload.body:find("Reason: sync conflict retry budget already exhausted before codex", 1, true) ~= nil)
  end,

  test_sync_conflict_staged_conflict_marker_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", {
      stdout = "core.lua:1: leftover conflict marker\n",
      stderr = "",
      exit_code = 2,
    })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-staged-marker", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_staged_whitespace_after_add_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", {
      stdout = "core.lua:2: trailing whitespace.\n",
      stderr = "",
      exit_code = 2,
    })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-staged-after-add", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_unmerged_reappears_after_add_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-unmerged-after-add", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_integration_head_moved_before_push_skips_unsafe_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_successful_codex_resolution()
    mock_real_push("dddd4444")
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-head-moved", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("commit -F"), 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,
}
