local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_decompose" },
  produces = {
    "github-proxy.github_issue_create_request",
  },
  stall_window = "2m",
  retry = { max_attempts = 2, base = "5s", cap = "10s" },
}

local MAX_RUNTIME_ID_LEN = 180

local function safe_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "_")
  safe = safe:gsub("_+", "_"):gsub("^_+", ""):gsub("_+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

local function marker_body_file(repo, pr_number)
  local id = "decompose-" .. safe_segment(repo) .. "-pr-" .. safe_segment(pr_number)
  if #id > MAX_RUNTIME_ID_LEN then
    id = id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return "/tmp/fkst-github-devloop-" .. id .. ".md"
end

local function parse_failure_key(decompose)
  return core._dedup_key({
    "github-devloop",
    "decompose-parse-failure",
    tostring(decompose.proposal_id),
    tostring(decompose.version),
  })
end

local function judgment_worktree(role, identity)
  local runtime = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
  if runtime.exit_code ~= 0 then
    error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime.stderr))
  end
  local worktree = core.judgment_worktree_path(runtime.stdout, role, identity)
  local mkdir = exec_sync({ cmd = core.mkdir_p_cmd(worktree), timeout = 30 })
  if mkdir.exit_code ~= 0 then
    error("github-devloop: judgment scratch directory setup failed: " .. tostring(mkdir.stderr))
  end
  return worktree
end

local function decompose_plan(decompose, current_issue, content_fetch)
  local prompt = core.build_decompose_prompt(decompose, current_issue, content_fetch)
  local result = spawn_codex_sync(core.judgment_codex_opts(
    prompt,
    judgment_worktree("decompose", decompose.dedup_key)
  ))
  if type(result) == "table" and result.exit_code ~= nil and result.exit_code ~= 0 then
    error("github-devloop: decompose codex failed: " .. tostring(result.stderr or ""))
  end
  local stdout = type(result) == "table" and result.stdout or result
  local issues = core.parse_decompose_plan(stdout)
  if issues ~= nil then
    return issues
  end

  local key = parse_failure_key(decompose)
  local previous = tonumber(cache_get(key) or "0") or 0
  cache_set(key, tostring(previous + 1))
  if previous < 1 then
    error("github-devloop: decompose JSON parse failed; retrying")
  end
  return core.fallback_decompose_plan(decompose)
end

local function read_current_pr(repo, pr_number)
  local pr_view = core.gh_exec({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr decompose view failed: " .. tostring(pr_view.stderr))
  end
  return core.parse_pr_view_origin(pr_view.stdout)
end

local function write_decomposed_marker(repo, decompose, count)
  local path = marker_body_file(repo, decompose.pr_number)
  file.write(path, core.decomposed_comment_body(decompose, count))
  local result = core.gh_exec({ cmd = core.gh_pr_comment_cmd(repo, decompose.pr_number, path), timeout = 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: gh pr decomposed marker comment failed: " .. tostring(result.stderr))
  end

  local confirmed_pr = read_current_pr(repo, decompose.pr_number)
  if not core.has_decomposed_marker(confirmed_pr.comments, decompose.proposal_id, decompose.version, decompose.pr_number) then
    error("github-devloop: decomposed marker not yet visible after write; retrying")
  end
end

function pipeline(event)
  local decompose = event.payload or {}
  if not core.is_supported_decompose(decompose) then
    core.log_entry("decompose", event, "unknown", decompose.dedup_key)
    core.log_cas_decision("decompose", "unknown", { state = nil, version = nil }, "blocked", "decomposed", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("decompose", event, decompose.proposal_id, decompose.dedup_key)
  local entity = core.parse_entity_proposal_id(decompose.proposal_id)
  if entity == nil or entity.issue_number == nil then
    core.log_cas_decision("decompose", decompose.proposal_id, { state = nil, version = nil }, "blocked", "decomposed", "skip-foreign(proposal_id)", "proposal_id is outside issue-backed github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number
  local _, pr_number = core.parse_pr_source_ref(decompose.source_ref)
  if tostring(pr_number or "") ~= tostring(decompose.pr_number) then
    core.log_cas_decision("decompose", decompose.proposal_id, { state = nil, version = nil }, "blocked", "decomposed", "skip-foreign(source_ref)", "source_ref PR does not match decompose payload")
    return
  end

  local lock_key = core.transition_lock_key(decompose.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("decompose", decompose.proposal_id, { state = nil, version = nil }, "blocked", "decomposed", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local current_pr = read_current_pr(repo, decompose.pr_number)
    core.log_forged_markers("decompose", decompose.proposal_id, current_pr.comments)

    local state = core.current_entity_state(current_pr.comments, decompose.proposal_id)
    if not core.has_fix_reconcile_marker(current_pr.comments, decompose.proposal_id, decompose.version)
      or state.state ~= "blocked"
      or tostring(state.version or "") ~= tostring(decompose.version) then
      core.log_cas_decision("decompose", decompose.proposal_id, state, "blocked", "decomposed", "retry-pending(blocked-fix-reconcile-not-visible)", "blocked/fix-reconcile marker is not yet visible")
      error("github-devloop: blocked fix reconcile marker not yet visible for decompose; retrying")
    end
    if core.has_decomposed_marker(current_pr.comments, decompose.proposal_id, decompose.version, decompose.pr_number) then
      core.log_cas_decision("decompose", decompose.proposal_id, state, "blocked", "decomposed", "skip-idempotent(decomposed marker already visible)", "decompose already applied")
      return
    end

    local issue_view = core.gh_exec({ cmd = core.gh_issue_view_decompose_cmd(repo, issue_number), timeout = 30 })
    if issue_view.exit_code ~= 0 then
      error("github-devloop: gh issue decompose view failed: " .. tostring(issue_view.stderr))
    end
    local current_issue = core.parse_issue_view_decompose(issue_view.stdout)
    local depth = core.decompose_lineage_depth(current_issue.body)
    if depth >= core.max_decompose_depth() then
      core.log_cas_decision("decompose", decompose.proposal_id, state, "blocked", "decomposed", "skip-depth-cap(decompose-lineage)", "decompose lineage depth cap reached")
      return
    end
    decompose.current_issue_body = current_issue.body
    local content_fetch = core.context_fetch_from_bundle({
      dept = "decompose",
      repo = repo,
      issue_number = issue_number,
      pr_number = decompose.pr_number,
      proposal_id = decompose.proposal_id,
      version = decompose.dedup_key,
      tick = event.ts,
    })
    local issues = decompose_plan(decompose, current_issue, content_fetch)
    local count = math.min(#issues, core.max_decompose_issues())
    if count < 1 then
      issues = core.fallback_decompose_plan(decompose)
      count = 1
    end

    if core.write_mode() ~= "real" then
      core.log_cas_decision("decompose", decompose.proposal_id, state, "blocked", "decomposed", "dry-run(marker-write-required)", "FKST_GITHUB_WRITE=1 is required before issue create requests")
      return
    end
    write_decomposed_marker(repo, decompose, count)

    core.log_apply("decompose", decompose.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "github-proxy.github_issue_create_request",
    })
    for index = 1, count do
      local create_request = core.build_issue_create_request(repo, decompose, issues[index], index)
      core.log_raise("decompose", decompose.proposal_id, "github-proxy.github_issue_create_request", create_request)
    end
  end)
end

return M
