local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local parsers_pr = require("devloop.parsers.pr")
local S = {}
local check_runs = require("forge.github.check_runs")
local forge_validators = require("devloop.forge_validators")
local config = require("devloop.config")

function S.install(M)
local strings = require("contract.strings")
local substrate_ref_path = ".fkst/substrate-ref"
local substrate_repo = "ChronoAIProject/fkst-substrate"
local substrate_remote = "https://github.com/ChronoAIProject/fkst-substrate.git"
local substrate_branch = "dev"
local bump_branch = "chore/substrate-ref-bump"
local bump_title = "chore: bump fkst-substrate pin"
local substrate_dev_ref = "refs/remotes/fkst-substrate/dev"
local validate_bump_pr
local git_handle
local github_handle

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or base_ids.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for substrate-ref scan")
  end
  return value
end

local function git()
  if git_handle == nil then
    git_handle = require("forge.git").new(exec_argv)
  end
  return git_handle
end

local function github()
  if github_handle == nil then
    github_handle = require("forge.github").new(exec_argv)
  end
  return github_handle
end

local function run_adapter(fn, label)
  local ok, result_or_error = pcall(fn)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result_or_error))
end

local function run_cmd(fn, label)
  local result = run_adapter(fn, label)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function run_argv(argv, timeout, label)
  return run_adapter(function()
    return exec_argv({ argv = argv, timeout = timeout or 30 })
  end, label)
end

local function is_missing_substrate_ref_pin(result)
  if result == nil or result.exit_code == 0 then
    return false
  end
  local stderr = tostring(result.stderr or "")
  return stderr:find("path '" .. substrate_ref_path .. "' does not exist in", 1, true) ~= nil
    or stderr:find("path '" .. substrate_ref_path .. "' exists on disk, but not in", 1, true) ~= nil
end

local function run_gh(fn, label)
  local result = run_adapter(fn, label)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function read_runtime_root()
  local result = run_cmd(function()
    return exec_sync({ cmd = M.read_runtime_root_cmd(), timeout = 30 })
  end, "runtime root read")
  local root = strings.trim(result.stdout)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: FKST_RUNTIME_ROOT is required for substrate-ref bump")
  end
  return root:gsub("/+$", "")
end

function M.git_show_substrate_ref_pin_cmd()
  return "substrate-ref pin read"
end

local function read_pin()
  local result = run_adapter(function()
    return git().show_file("HEAD", substrate_ref_path, 30)
  end, "substrate-ref pin read")
  if is_missing_substrate_ref_pin(result) then
    return nil
  end
  if result.exit_code ~= 0 then
    error("github-devloop: substrate-ref pin read failed: " .. tostring(result.stderr))
  end
  local pin = strings.trim(result.stdout)
  if not forge_validators.is_git_sha(pin) then
    error("github-devloop: invalid .fkst/substrate-ref pin")
  end
  return pin:lower()
end

local function parse_ls_remote(stdout)
  local sha, ref = tostring(stdout or ""):match("^(%x+)%s+(refs/heads/[^%s]+)")
  if ref ~= "refs/heads/" .. substrate_branch or not forge_validators.is_git_sha(sha) then
    return nil
  end
  return sha:lower()
end

local function fetch_substrate_dev_head()
  local result = run_cmd(
    function()
      return git().ls_remote_branch(substrate_remote, substrate_branch, 60)
    end,
    "substrate upstream head read"
  )
  local sha = parse_ls_remote(result.stdout)
  if sha == nil then
    error("github-devloop: substrate upstream head read returned an invalid dev head")
  end
  return sha
end

local function fetch_substrate_dev_ref()
  local result = run_cmd(
    function()
      return git().fetch_remote_branch_to_tracking_ref(substrate_remote, substrate_branch, substrate_dev_ref, 60)
    end,
    "substrate upstream tracking ref fetch"
  )
  return result
end

local function substrate_pin_is_dev_ancestor(pin, target_sha)
  if not forge_validators.is_git_sha(pin) then
    return false, "invalid-substrate-pin"
  end
  if not forge_validators.is_git_sha(target_sha) then
    return false, "invalid-substrate-target"
  end
  fetch_substrate_dev_ref()
  local head = run_cmd(
    function()
      return git().rev_parse_ref_commit(substrate_dev_ref, 30)
    end,
    "substrate upstream tracking ref read"
  )
  local fetched_head = strings.trim(head.stdout)
  if not forge_validators.is_git_sha(fetched_head) then
    return false, "invalid-substrate-dev-head"
  end
  if fetched_head:lower() ~= tostring(target_sha):lower() then
    return false, "substrate-dev-head-mismatch"
  end
  local ancestry = git_mechanics.git_is_ancestor(M.git, pin, fetched_head, 30)
  if ancestry.exit_code == 0 then
    return true, "substrate-pin-valid"
  end
  return false, "substrate-pin-not-dev-ancestor"
end

local function substrate_publishability_reason(reason)
  if reason == "rollup-green" then
    return "substrate-ci-green"
  end
  if reason == "missing-status-rollup" then
    return "substrate-ci-missing"
  end
  if reason == "rollup-pending" then
    return "substrate-ci-pending"
  end
  if reason == "rollup-red" then
    return "substrate-ci-red"
  end
  return "substrate-ci-" .. tostring(strings.sanitize_key(reason or "unknown", false):gsub("/", "-"))
end

local function substrate_commit_publishable(sha)
  if not forge_validators.is_git_sha(sha) then
    return false, "invalid-substrate-target"
  end
  local result = run_gh(function()
    return github().api_get(substrate_repo, "commits/" .. tostring(sha) .. "/check-runs", 60)
  end, "substrate upstream check-runs read")
  local ok, reason = check_runs.commit_check_runs_green(check_runs.parse_commit_check_runs(result.stdout))
  if ok then
    return true, "substrate-ci-green"
  end
  return false, substrate_publishability_reason(reason)
end

local function parse_pr_list(stdout)
  local pages = json.decode(stdout)
  local prs = {}
  if type(pages) ~= "table" then
    return prs
  end
  for _, page in ipairs(pages) do
    if type(page) == "table" then
      for _, pr in ipairs(page) do
        if type(pr) == "table" and pr.number ~= nil then
          table.insert(prs, pr)
        end
      end
    end
  end
  return prs
end

local function pr_number(value)
  local n = tonumber(value)
  if n == nil or n ~= math.floor(n) or n < 1 then
    return nil
  end
  return n
end

local function existing_bump_pr(repo)
  local result = run_gh(function()
    return github().pr_list_head(repo, bump_branch, nil, 30)
  end, "substrate-ref PR list")
  local prs = parse_pr_list(result.stdout)
  if #prs > 1 then
    error("github-devloop: multiple open substrate-ref bump PRs found")
  end
  return prs[1]
end

local function current_bump_pr_number(existing)
  return pr_number(existing and existing.number)
end

local function bump_worktree_path(runtime_root, repo, head_sha)
  local slug = strings.sanitize_key("substrate-ref-" .. tostring(repo), false):gsub("/", "-")
  slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if slug == "" then
    slug = "substrate-ref"
  end
  if #slug > 90 then
    slug = slug:sub(1, 90):gsub("%-+$", "")
  end
  return runtime_root .. "/worktrees/" .. slug .. "-" .. tostring(head_sha):sub(1, 12)
end

local function ensure_parent_dir(path)
  local value = tostring(path or "")
  local parent = value:gsub("/+$", ""):match("^(.*)/[^/]+$") or "."
  local result = run_argv({ "mkdir", "-p", parent }, 30, "substrate-ref directory create")
  if result.exit_code ~= 0 then
    error("github-devloop: substrate-ref directory create failed: " .. tostring(result.stderr))
  end
end

local function remove_worktree_if_present(worktree)
  local value = tostring(worktree or "")
  local dir = run_argv({ "test", "-d", value }, 30, "substrate-ref worktree presence check")
  if dir.exit_code ~= 0 then
    return
  end
  local remove = M.git.worktree_remove(value, 60)
  if remove.exit_code ~= 0 then
    error("github-devloop: git stale substrate-ref worktree remove failed: " .. tostring(remove.stderr))
  end
end

local function write_pr_body(repo, current_pin, target_sha)
  local path = "/tmp/fkst-github-devloop-substrate-ref-bump-" .. base_ids.safe_repo(repo):gsub("/", "-") .. ".md"
  local body = table.concat({
    "Updates `.fkst/substrate-ref` to the current `fkst-substrate` `dev` head.",
    "",
    "- Previous pin: `" .. tostring(current_pin) .. "`",
    "- New pin: `" .. tostring(target_sha) .. "`",
    "",
    "The upstream commit check runs were green before this bump was opened. Consumer CI is the compatibility gate for this package repo.",
    "",
    "⟦AI:FKST⟧",
  }, "\n")
  body = M.with_github_debug_stamp(body, {
    emitter = "github-devloop.substrate-ref.pr-create",
    target = "pr:" .. tostring(repo) .. "#new",
    dedup_key = tostring(current_pin) .. "->" .. tostring(target_sha),
  })
  file.write(path, body .. "\n")
  return path
end

local function fetch_bump_branch_head()
  local fetch = run_adapter(function()
    return git().fetch_branch("origin", bump_branch, 60)
  end, "substrate-ref bump branch fetch")
  if fetch.exit_code ~= 0 then
    return nil
  end
  local head = run_cmd(function()
    return git().remote_branch_head("origin", bump_branch, 30)
  end, "substrate-ref bump branch head")
  local sha = strings.trim(head.stdout)
  if not forge_validators.is_git_sha(sha) then
    error("github-devloop: invalid substrate-ref bump branch head")
  end
  return sha:lower()
end

local function remote_bump_branch_pin(branch_head)
  if branch_head == nil then
    return nil
  end
  local result = run_adapter(function()
    return git().show_file(branch_head, substrate_ref_path, 30)
  end, "substrate-ref bump branch pin read")
  if result.exit_code ~= 0 then
    return nil
  end
  local pin = strings.trim(result.stdout)
  if not forge_validators.is_git_sha(pin) then
    return nil
  end
  return pin:lower()
end

local function remove_existing_branch_worktree(branch)
  local list = run_adapter(function()
    return git().worktree_list(30)
  end, "substrate-ref worktree list")
  if list.exit_code ~= 0 then
    error("github-devloop: substrate-ref worktree list failed exit="
      .. tostring(list.exit_code)
      .. " stderr="
      .. tostring(list.stderr))
  end
  local existing = M.find_worktree_for_branch(list.stdout, branch)
  if existing ~= nil then
    local remove = M.git.worktree_remove(existing, 60)
    if remove.exit_code ~= 0 then
      error("github-devloop: stale substrate-ref branch worktree remove failed: " .. tostring(remove.stderr))
    end
  end
end

local function pin_delta_state(worktree)
  local diff = run_cmd(function()
    return git().diff_name_only(worktree, "HEAD", 30)
  end, "substrate-ref changed paths")
  local name = strings.trim(diff.stdout)
  if name == "" then
    return "empty"
  end
  if name ~= substrate_ref_path then
    error("github-devloop: substrate-ref bump changed unexpected paths")
  end
  return "pin-only"
end

local function create_or_update_branch(repo, base_branch, current_pin, target_sha)
  local old_branch_head = fetch_bump_branch_head()
  local base_head = git_mechanics.current_base_head(M.git, base_branch)
  if base_head == nil then
    error("github-devloop: unable to read base branch head for substrate-ref bump")
  end
  if old_branch_head ~= nil and remote_bump_branch_pin(old_branch_head) == target_sha then
    local ancestry = git_mechanics.git_is_ancestor(M.git, base_head, old_branch_head, 30)
    if ancestry.exit_code == 0 then
      return "already-current"
    end
  end
  local runtime_root = read_runtime_root()
  local worktree = bump_worktree_path(runtime_root, repo, target_sha)
  remove_existing_branch_worktree(bump_branch)
  remove_worktree_if_present(worktree)
  ensure_parent_dir(worktree)
  local action = "updated"
  local added = false
  local ok, err = pcall(function()
    run_cmd(function()
      return git().worktree_add_reset_branch(worktree, bump_branch, base_head, 120)
    end, "substrate-ref worktree add")
    added = true
    local pin_path = worktree:gsub("/+$", "") .. "/" .. substrate_ref_path
    ensure_parent_dir(pin_path)
    file.write(pin_path, target_sha .. "\n")
    if pin_delta_state(worktree) == "empty" then
      action = "base-current"
      return
    end
    run_cmd(function()
      return git().add_all(worktree, 30)
    end, "substrate-ref add")
    run_cmd(function()
      return git().commit_message(worktree, "chore: bump fkst-substrate pin", 60)
    end, "substrate-ref commit")
    if old_branch_head == nil then
      local push = git_mechanics.git_push_worktree_branch_update(M.git, worktree, bump_branch, 120)
      if push.exit_code ~= 0 then
        error("github-devloop: substrate-ref push failed: " .. tostring(push.stderr))
      end
    else
      local push = git().git_push_worktree_branch_update_with_lease(worktree, bump_branch, old_branch_head, 120)
      if push.exit_code ~= 0 then
        error("github-devloop: substrate-ref push failed: " .. tostring(push.stderr))
      end
    end
  end)
  if added then
    local remove = M.git.worktree_remove(worktree, 60)
    if ok and remove.exit_code ~= 0 then
      error("github-devloop: substrate-ref worktree remove failed: " .. tostring(remove.stderr))
    end
  end
  if not ok then
    error(err)
  end
  return action
end

local function create_pr(repo, base_branch, current_pin, target_sha)
  local body_file = write_pr_body(repo, current_pin, target_sha)
  local result = run_gh(function()
    return github().pr_create(repo, bump_branch, base_branch, bump_title, body_file, 60)
  end, "substrate-ref PR create")
  local number = tostring(result.stdout or ""):match("/pull/(%d+)")
  return pr_number(number)
end

local function log_scan(action, fields)
  local parts = { "action=" .. tostring(action) }
  for _, field in ipairs(fields or {}) do
    table.insert(parts, tostring(field))
  end
  M.log_line("info", "substrate_ref_scan", "repo-management-plane", "SUBSTRATE_REF", parts)
end

local function read_pr(pr_number_value, repo)
  local viewed = run_gh(function()
    return github().pr_cli_view(
      repo,
      pr_number_value,
      "headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup",
      30
    )
  end, "substrate-ref PR view")
  local pr = parsers_pr.parse_pr_view_merge(M, viewed.stdout)
  pr.number = pr_number_value
  return pr
end

local function changed_paths(repo, pr_number_value)
  local diff = run_gh(function()
    return github().pr_diff_name_only(repo, pr_number_value, 30)
  end, "substrate-ref PR diff")
  return devloop_base.parse_name_only_paths(diff.stdout)
end

validate_bump_pr = function(repo, base_branch, pr)
  if type(pr) ~= "table" then
    return false, "missing-pr"
  end
  if tostring(pr.state or ""):upper() ~= "OPEN" then
    return false, "pr-not-open"
  end
  if pr.is_draft then
    return false, "draft-pr"
  end
  if tostring(pr.head_ref_name or "") ~= bump_branch then
    return false, "head-branch-mismatch"
  end
  if tostring(pr.base_ref_name or "") ~= tostring(base_branch or "") then
    return false, "base-branch-mismatch"
  end
  if not require("forge.merge.shared").is_same_repo_pr_head(pr, repo) then
    return false, "foreign-head-repository"
  end
  if not forge_validators.is_git_sha(pr.head_sha) then
    return false, "invalid-head-sha"
  end
  local paths = changed_paths(repo, pr.number)
  if #paths ~= 1 or paths[1] ~= substrate_ref_path then
    return false, "unexpected-diff"
  end
  return true, "substrate-ref-bump-ok"
end

local function substrate_ref_merge_marker(pr, target_sha, outcome, reason)
  if not forge_validators.is_positive_pr_number(pr and pr.number) or not forge_validators.is_git_sha(pr and pr.head_sha) or not forge_validators.is_git_sha(target_sha) then
    error("github-devloop: invalid substrate-ref merge marker")
  end
  return '<!-- fkst:github-devloop:substrate-ref-merge:v1 pr="' .. tostring(pr.number)
    .. '" head_sha="' .. tostring(pr.head_sha)
    .. '" target_sha="' .. tostring(target_sha)
    .. '" outcome="' .. tostring(outcome)
    .. '" reason="' .. tostring(strings.sanitize_key(reason or "merge-gate-ok", false):gsub("/", "-"))
    .. '" -->'
end

local function substrate_ref_merge_audit_body(pr, target_sha, outcome, reason)
  return table.concat({
    "github-devloop substrate-ref deterministic merge audit",
    "",
    "The substrate-ref bump is handled by deterministic gates: exact `.fkst/substrate-ref` diff, upstream ancestry, upstream commit CI green, same-repo head, non-draft PR, consumer CI green, mergeability, and matched head merge.",
    "",
    substrate_ref_merge_marker(pr, target_sha, outcome, reason),
    "⟦AI:FKST⟧",
  }, "\n")
end

local function log_bump_merge(action, repo, pr_number_value, reason)
  log_scan(action, {
    "repo=" .. tostring(repo),
    "pr=" .. tostring(pr_number_value or ""),
    "reason=" .. tostring(reason or ""),
  })
end

local function validate_bump_merge_facts(repo, base_branch, pr, target_sha)
  local ok, reason = validate_bump_pr(repo, base_branch, pr)
  if not ok then
    return false, reason
  end
  local branch_head = fetch_bump_branch_head()
  if tostring(branch_head or "") ~= tostring(pr.head_sha or "") then
    return false, "branch-head-mismatch"
  end
  local pin = remote_bump_branch_pin(branch_head)
  local pin_ok, pin_reason = substrate_pin_is_dev_ancestor(pin, target_sha)
  if not pin_ok then
    return false, pin_reason
  end
  local publishable_ok, publishable_reason = substrate_commit_publishable(pin)
  if not publishable_ok then
    return false, publishable_reason
  end
  local gate_ok, gate_reason = M.evaluate_ci_merge_gate(pr, {
    repo = repo,
    dept = "substrate_ref_scan",
    proposal_id = "substrate-ref-merge",
  })
  if not gate_ok then
    return false, gate_reason
  end
  return true, "substrate-ref-merge-ok"
end

local function raise_merge_audit(repo, pr, target_sha, outcome, reason)
  local request = entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr.number,
  }, substrate_ref_merge_audit_body(pr, target_sha, outcome, reason), base_ids.dedup_key({
    "substrate-ref-merge",
    base_ids.safe_repo(repo),
    tostring(pr.number),
    tostring(pr.head_sha),
    tostring(outcome),
    tostring(reason or ""),
  }), entity_lib.pr_source_ref(repo, pr.number))
  M.log_raise("substrate_ref_scan", "substrate-ref-merge", "github-proxy.github_pr_comment_request", request)
  raise("github-proxy.github_pr_comment_request", request)
  return request
end

local function maybe_merge_bump_pr(repo, base_branch, existing, target_sha, write_enabled)
  local number = current_bump_pr_number(existing)
  if number == nil then
    log_bump_merge("merge-skip", repo, nil, "no-open-pr")
    return { status = "no-open-pr", reason = "no-open-pr" }
  end
  local pr = read_pr(number, repo)
  local ok, reason = validate_bump_merge_facts(repo, base_branch, pr, target_sha)
  if not ok then
    log_bump_merge("merge-hold", repo, number, reason)
    return { status = "hold", pr_number = number, reason = reason }
  end
  if not write_enabled then
    log_bump_merge("merge-dry-run", repo, number, "FKST_GITHUB_WRITE!=1")
    return { status = "would-merge", pr_number = number, reason = "FKST_GITHUB_WRITE!=1" }
  end
  local merged, merge_reason, merged_pr = M.run_verified_pr_merge({
    dept = "substrate_ref_scan",
    proposal_id = "substrate-ref-merge",
    repo = repo,
    pr_number = number,
    head_sha = pr.head_sha,
    head_branch = bump_branch,
    base_branch = base_branch,
    match_head_retry_attempts = 2,
    validate_rechecked_pr = function(rechecked)
      return validate_bump_merge_facts(repo, base_branch, rechecked, target_sha)
    end,
  })
  if not merged then
    log_bump_merge("merge-hold", repo, number, merge_reason)
    return { status = "hold", pr_number = number, reason = merge_reason }
  end
  log_bump_merge("merged", repo, number, merge_reason)
  raise_merge_audit(repo, merged_pr or pr, target_sha, "merged", merge_reason)
  return { status = "merged", pr_number = number, reason = merge_reason }
end

local function hold_unpublishable_target(repo, current_pin, target_sha, reason, merge_result)
  log_scan("bump-hold", {
    "repo=" .. repo,
    "from=" .. current_pin,
    "to=" .. target_sha,
    "reason=" .. tostring(reason),
  })
  return {
    status = "hold",
    reason = reason,
    pin = current_pin,
    target = target_sha,
    branch = bump_branch,
    merge = merge_result,
  }
end

function M.substrate_ref_constants()
  return {
    path = substrate_ref_path,
    remote = substrate_remote,
    branch = substrate_branch,
    bump_branch = bump_branch,
    title = bump_title,
  }
end

function M.substrate_ref_scan()
  local cfg = config.devloop_config(M)
  local repo = require_repo(cfg.repo)
  if cfg.write_mode == "real" then
    devloop_base.assert_trusted_bot_configured()
  end

  local current_pin = read_pin()
  if current_pin == nil then
    log_scan("no-substrate-pin", {
      "repo=" .. repo,
      "path=" .. substrate_ref_path,
      "disposition=no-substrate-pin",
    })
    return { status = "no-substrate-pin", path = substrate_ref_path }
  end
  local target_sha = fetch_substrate_dev_head()
  if current_pin == target_sha then
    log_scan("unchanged", {
      "repo=" .. repo,
      "pin=" .. current_pin,
    })
    return { status = "current", pin = current_pin, target = target_sha }
  end

  if cfg.write_mode ~= "real" then
    local existing = existing_bump_pr(repo)
    local merge_result = nil
    if existing ~= nil then
      merge_result = maybe_merge_bump_pr(repo, cfg.upstream_branch, existing, target_sha, false)
    end
    local publishable_ok, publishable_reason = substrate_commit_publishable(target_sha)
    if not publishable_ok then
      return hold_unpublishable_target(repo, current_pin, target_sha, publishable_reason, merge_result)
    end
    log_scan("bump-planned", {
      "mode=" .. cfg.write_mode,
      "repo=" .. repo,
      "from=" .. current_pin,
      "to=" .. target_sha,
      "branch=" .. bump_branch,
      "existing_pr=" .. tostring(existing and existing.number or ""),
    })
    return {
      status = "planned",
      pin = current_pin,
      target = target_sha,
      existing_pr = existing and existing.number or nil,
      branch = bump_branch,
      merge = merge_result,
    }
  end

  local final_existing = nil
  local branch_action = nil
  local created_pr_number = nil
  local preupdate_merge_result = nil
  local target_hold_reason = nil
  with_lock("github-devloop/substrate-ref/" .. base_ids.safe_repo(repo), function()
    final_existing = existing_bump_pr(repo)
    if final_existing ~= nil then
      preupdate_merge_result = maybe_merge_bump_pr(repo, cfg.upstream_branch, final_existing, target_sha, true)
      if preupdate_merge_result.status == "merged" then
        return
      end
    end
    local publishable_ok, publishable_reason = substrate_commit_publishable(target_sha)
    if not publishable_ok then
      branch_action = "target-not-publishable"
      target_hold_reason = publishable_reason
      preupdate_merge_result = preupdate_merge_result or { status = "hold", reason = publishable_reason }
      return
    end
    branch_action = create_or_update_branch(repo, cfg.upstream_branch, current_pin, target_sha)
    if final_existing == nil and branch_action ~= "base-current" then
      created_pr_number = create_pr(repo, cfg.upstream_branch, current_pin, target_sha)
      log_scan("pr-created", {
        "mode=real",
        "repo=" .. repo,
        "from=" .. current_pin,
        "to=" .. target_sha,
        "branch=" .. bump_branch,
        "pr=" .. tostring(created_pr_number or ""),
      })
    elseif final_existing ~= nil then
      log_scan("pr-updated", {
        "mode=real",
        "repo=" .. repo,
        "from=" .. current_pin,
        "to=" .. target_sha,
        "branch=" .. bump_branch,
        "pr=" .. tostring(final_existing.number),
        "branch_action=" .. tostring(branch_action),
      })
    else
      log_scan("base-current", {
        "mode=real",
        "repo=" .. repo,
        "from=" .. current_pin,
        "to=" .. target_sha,
      })
    end
  end)

  if preupdate_merge_result ~= nil and preupdate_merge_result.status == "merged" then
    return {
      status = "merged",
      pin = current_pin,
      target = target_sha,
      existing_pr = final_existing and final_existing.number or nil,
      pr_number = final_existing and final_existing.number or nil,
      branch = bump_branch,
      merge = preupdate_merge_result,
    }
  end
  if preupdate_merge_result ~= nil and branch_action == "already-current" then
    return {
      status = "updated",
      pin = current_pin,
      target = target_sha,
      existing_pr = final_existing and final_existing.number or nil,
      pr_number = final_existing and final_existing.number or nil,
      branch = bump_branch,
      merge = preupdate_merge_result,
    }
  end
  if branch_action == "target-not-publishable" then
    return hold_unpublishable_target(
      repo,
      current_pin,
      target_sha,
      target_hold_reason or "substrate-ci-missing",
      preupdate_merge_result
    )
  end
  if branch_action == "base-current" then
    return { status = "current", pin = current_pin, target = target_sha }
  end
  if final_existing ~= nil and branch_action ~= "already-current" then
    final_existing = existing_bump_pr(repo)
  end
  local merge_result = maybe_merge_bump_pr(repo, cfg.upstream_branch, final_existing or { number = created_pr_number }, target_sha, true)
  return {
    status = final_existing == nil and "created" or "updated",
    pin = current_pin,
    target = target_sha,
    existing_pr = final_existing and final_existing.number or nil,
    pr_number = final_existing and final_existing.number or created_pr_number,
    branch = bump_branch,
    merge = merge_result,
  }
end
end

return S
