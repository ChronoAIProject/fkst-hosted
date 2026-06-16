local S = {}

function S.install(M)
local strings = require("std.strings")
local substrate_ref_path = ".fkst/substrate-ref"
local substrate_remote = "https://github.com/ChronoAIProject/fkst-substrate.git"
local substrate_branch = "dev"
local bump_branch = "chore/substrate-ref-bump"
local bump_title = "chore: bump fkst-substrate pin"
local lifecycle_version_prefix = "substrate-ref-bump"

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or M.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for substrate-ref scan")
  end
  return value
end

local function run_cmd(cmd, timeout, label)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function is_missing_substrate_ref_pin(result)
  if result == nil or result.exit_code == 0 then
    return false
  end
  local stderr = tostring(result.stderr or "")
  return stderr:find("path '" .. substrate_ref_path .. "' does not exist in", 1, true) ~= nil
    or stderr:find("path '" .. substrate_ref_path .. "' exists on disk, but not in", 1, true) ~= nil
end

local function run_gh(cmd, timeout, label)
  local result = M.gh_exec({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function read_runtime_root()
  local result = run_cmd(M.read_runtime_root_cmd(), 30, "runtime root read")
  local root = M._trim(result.stdout)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: FKST_RUNTIME_ROOT is required for substrate-ref bump")
  end
  return root:gsub("/+$", "")
end

function M.git_show_substrate_ref_pin_cmd()
  return "git show " .. M._shell_single_quote("HEAD:" .. substrate_ref_path)
end

local function read_pin()
  local result = exec_sync({ cmd = M.git_show_substrate_ref_pin_cmd(), timeout = 30 })
  if is_missing_substrate_ref_pin(result) then
    return nil
  end
  if result.exit_code ~= 0 then
    error("github-devloop: git show substrate-ref pin failed: " .. tostring(result.stderr))
  end
  local pin = M._trim(result.stdout)
  if not M._is_git_sha(pin) then
    error("github-devloop: invalid .fkst/substrate-ref pin")
  end
  return pin:lower()
end

local function parse_ls_remote(stdout)
  local sha, ref = tostring(stdout or ""):match("^(%x+)%s+(refs/heads/[^%s]+)")
  if ref ~= "refs/heads/" .. substrate_branch or not M._is_git_sha(sha) then
    return nil
  end
  return sha:lower()
end

local function fetch_substrate_dev_head()
  local result = run_cmd(
    M.git_ls_remote_branch_cmd(substrate_remote, substrate_branch),
    60,
    "git ls-remote fkst-substrate dev"
  )
  local sha = parse_ls_remote(result.stdout)
  if sha == nil then
    error("github-devloop: git ls-remote did not return a valid fkst-substrate dev head")
  end
  return sha
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
  local result = run_gh(M.gh_pr_list_head_cmd(repo, bump_branch), 30, "gh substrate-ref PR list")
  local prs = parse_pr_list(result.stdout)
  if #prs > 1 then
    error("github-devloop: multiple open substrate-ref bump PRs found")
  end
  return prs[1]
end

local function current_bump_pr_number(existing)
  return pr_number(existing and existing.number)
end

local function lifecycle_version(head_sha)
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid substrate-ref lifecycle head sha")
  end
  return lifecycle_version_prefix .. "/" .. tostring(head_sha)
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

local function write_pr_body(repo, current_pin, target_sha)
  local path = "/tmp/fkst-github-devloop-substrate-ref-bump-" .. M.safe_repo(repo):gsub("/", "-") .. ".md"
  local body = table.concat({
    "Updates `.fkst/substrate-ref` to the current `fkst-substrate` `dev` head.",
    "",
    "- Previous pin: `" .. tostring(current_pin) .. "`",
    "- New pin: `" .. tostring(target_sha) .. "`",
    "",
    "CI is the verification gate for package compatibility with the new engine pin.",
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
  local fetch = exec_sync({ cmd = M.git_fetch_branch_cmd("origin", bump_branch), timeout = 60 })
  if fetch.exit_code ~= 0 then
    return nil
  end
  local head = run_cmd(M.git_remote_branch_head_cmd("origin", bump_branch), 30, "git substrate-ref bump branch head")
  local sha = M._trim(head.stdout)
  if not M._is_git_sha(sha) then
    error("github-devloop: invalid substrate-ref bump branch head")
  end
  return sha:lower()
end

local function remote_bump_branch_pin(branch_head)
  if branch_head == nil then
    return nil
  end
  local result = exec_sync({
    cmd = "git show " .. M._shell_single_quote(tostring(branch_head) .. ":" .. substrate_ref_path),
    timeout = 30,
  })
  if result.exit_code ~= 0 then
    return nil
  end
  local pin = M._trim(result.stdout)
  if not M._is_git_sha(pin) then
    return nil
  end
  return pin:lower()
end

local function remove_existing_branch_worktree(branch)
  local list = run_cmd(M.git_worktree_list_cmd(), 30, "git substrate-ref worktree list")
  local existing = M.find_worktree_for_branch(list.stdout, branch)
  if existing ~= nil then
    run_cmd(M.git_worktree_remove_cmd(existing), 60, "git stale substrate-ref branch worktree remove")
  end
end

local function pin_delta_state(worktree)
  local diff = run_cmd("git -C " .. M._shell_single_quote(worktree) .. " diff --name-only HEAD", 30, "git diff name-only")
  local name = M._trim(diff.stdout)
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
  if remote_bump_branch_pin(old_branch_head) == target_sha then
    return "already-current"
  end
  local base_head = M.current_base_head(base_branch)
  if base_head == nil then
    error("github-devloop: unable to read base branch head for substrate-ref bump")
  end
  local runtime_root = read_runtime_root()
  local worktree = bump_worktree_path(runtime_root, repo, target_sha)
  remove_existing_branch_worktree(bump_branch)
  run_cmd(M.git_worktree_remove_if_present_cmd(worktree), 60, "git stale substrate-ref worktree remove")
  local action = "updated"
  local added = false
  local ok, err = pcall(function()
    run_cmd(M.git_worktree_add_reset_branch_cmd(worktree, bump_branch, base_head), 120, "git substrate-ref worktree add")
    added = true
    run_cmd(M.git_write_file_cmd(worktree, substrate_ref_path, target_sha .. "\n"), 30, "write substrate-ref pin")
    if pin_delta_state(worktree) == "empty" then
      action = "base-current"
      return
    end
    run_cmd(M.git_add_all_cmd(worktree), 30, "git substrate-ref add")
    run_cmd(M.git_commit_cmd(worktree, "chore: bump fkst-substrate pin"), 60, "git substrate-ref commit")
    if old_branch_head == nil then
      run_cmd(M.git_push_worktree_branch_update_cmd(worktree, bump_branch), 120, "git substrate-ref push")
    else
      run_cmd(
        M.git_push_worktree_branch_update_with_lease_cmd(worktree, bump_branch, old_branch_head),
        120,
        "git substrate-ref push"
      )
    end
  end)
  if added then
    local remove = exec_sync({ cmd = M.git_worktree_remove_cmd(worktree), timeout = 60 })
    if ok and remove.exit_code ~= 0 then
      error("github-devloop: git substrate-ref worktree remove failed: " .. tostring(remove.stderr))
    end
  end
  if not ok then
    error(err)
  end
  return action
end

local function create_pr(repo, base_branch, current_pin, target_sha)
  local body_file = write_pr_body(repo, current_pin, target_sha)
  local result = run_gh(M.gh_pr_create_cmd(repo, bump_branch, base_branch, bump_title, body_file), 60, "gh substrate-ref PR create")
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

local function parse_name_only_paths(stdout)
  local paths = {}
  local seen = {}
  for line in tostring(stdout or ""):gmatch("[^\r\n]+") do
    local path = line:gsub("^%s+", ""):gsub("%s+$", "")
    if path ~= "" and not seen[path] then
      table.insert(paths, path)
      seen[path] = true
    end
  end
  table.sort(paths)
  return paths
end

local function read_pr(pr_number_value, repo)
  local viewed = run_gh(M.gh_pr_view_merge_cmd(repo, pr_number_value), 30, "gh substrate-ref PR view")
  local pr = M.parse_pr_view_merge(viewed.stdout)
  pr.number = pr_number_value
  return pr
end

local function changed_paths(repo, pr_number_value)
  local diff = run_gh(M.gh_pr_diff_name_only_cmd(repo, pr_number_value), 30, "gh substrate-ref PR diff")
  return parse_name_only_paths(diff.stdout)
end

local function mechanical_review_proposal_id(repo, pr_number_value, head_sha)
  return M.pr_review_proposal_id(repo, pr_number_value, lifecycle_version(head_sha), head_sha)
end

local function mechanical_review_dedup_key(repo, pr_number_value, head_sha)
  return M._dedup_key({
    "substrate-ref-bump",
    "review",
    M.safe_repo(repo),
    tostring(pr_number_value),
    tostring(head_sha),
  })
end

local function backing_issue_dedup_key(repo, pr_number_value)
  return M._dedup_key({
    "substrate-ref-bump",
    "backing-issue",
    M.safe_repo(repo),
    tostring(pr_number_value),
  })
end

local function backing_issue_number(pr, dedup_key)
  for _, comment in ipairs(M._trusted_marker_comments(pr and pr.comments or {})) do
    local body = M._comment_body(comment)
    for marker in body:gmatch("<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->") do
      if marker:match('dedup="([^"]+)"') == tostring(dedup_key) then
        local number = pr_number(marker:match('issue="(%d+)"'))
        if number ~= nil then
          return number
        end
      end
    end
  end
  return nil
end

local function validate_bump_pr(repo, base_branch, pr)
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
  if not M.is_same_repo_pr_head(pr, repo) then
    return false, "foreign-head-repository"
  end
  if not M._is_git_sha(pr.head_sha) then
    return false, "invalid-head-sha"
  end
  local paths = changed_paths(repo, pr.number)
  if #paths ~= 1 or paths[1] ~= substrate_ref_path then
    return false, "unexpected-diff"
  end
  return true, "substrate-ref-bump-ok"
end

local function backing_issue_create_request(repo, pr, current_pin, target_sha, dedup_key)
  local body = table.concat({
    "Backing issue for the autonomous `fkst-substrate` pin bump PR.",
    "",
    "PR: #" .. tostring(pr.number),
    "Branch: `" .. bump_branch .. "`",
    "Base: `" .. tostring(pr.base_ref_name) .. "`",
    "Previous pin: `" .. tostring(current_pin) .. "`",
    "Target pin: `" .. tostring(target_sha) .. "`",
    "",
    "This issue owns the normal `github-devloop` lifecycle for the bump PR. The scanner only advances the PR after the trusted parent ledger records this issue number.",
  }, "\n")
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = bump_title,
    body = body,
    assignees = { M.claim_owner() },
    dedup_key = dedup_key,
    parent_comment_target = {
      repo = repo,
      pr_number = pr.number,
    },
    source_ref = M.pr_source_ref(repo, pr.number),
  }
end

local function lifecycle_comments(repo, issue_number, pr, head_sha)
  local version = lifecycle_version(head_sha)
  local proposal_id = M.proposal_id(repo, issue_number)
  local review_proposal = mechanical_review_proposal_id(repo, pr.number, head_sha)
  local review_dedup = mechanical_review_dedup_key(repo, pr.number, head_sha)
  return table.concat({
    "github-devloop substrate-ref bump lifecycle: backing issue approval",
    "",
    "This PR changes only `.fkst/substrate-ref`; the backing issue owns lifecycle state and the central merge gate owns CI, mergeability, same-repo, and head checks.",
    "",
    M.pr_origin_marker(proposal_id, issue_number, bump_branch, version, pr.base_ref_name),
    M.state_marker(proposal_id, "merge-ready", version),
    M.review_result_marker(review_proposal, proposal_id, "approve", review_dedup),
    M.merge_ready_marker(proposal_id, pr.number, version, review_proposal, review_dedup, head_sha),
    "⟦AI:FKST⟧",
  }, "\n")
end

local function ensure_bump_lifecycle(repo, base_branch, existing, current_pin, target_sha)
  local number = current_bump_pr_number(existing)
  if number == nil then
    log_scan("merge-lifecycle-skip", {
      "repo=" .. repo,
      "reason=no-open-pr",
    })
    return nil
  end
  local pr = read_pr(number, repo)
  local ok, reason = validate_bump_pr(repo, base_branch, pr)
  if not ok then
    log_scan("merge-lifecycle-hold", {
      "repo=" .. repo,
      "pr=" .. tostring(number),
      "reason=" .. tostring(reason),
    })
    return nil
  end
  local issue_dedup = backing_issue_dedup_key(repo, number)
  local issue_number = backing_issue_number(pr, issue_dedup)
  if issue_number == nil then
    local request = backing_issue_create_request(repo, pr, current_pin, target_sha, issue_dedup)
    log_scan("backing-issue-requested", {
      "repo=" .. repo,
      "pr=" .. tostring(number),
      "dedup_key=" .. tostring(issue_dedup),
    })
    M.log_raise("substrate_ref_scan", "substrate-ref-bump/" .. tostring(number), "github-proxy.github_issue_create_request", request)
    raise("github-proxy.github_issue_create_request", request)
    return nil
  end
  local version = lifecycle_version(pr.head_sha)
  local proposal_id = M.proposal_id(repo, issue_number)
  local review_proposal = mechanical_review_proposal_id(repo, number, pr.head_sha)
  local review_dedup = mechanical_review_dedup_key(repo, number, pr.head_sha)
  if not M.has_state_marker(pr.comments, proposal_id, "merge-ready", version)
    or M.merge_ready_fact(pr.comments, proposal_id, version, number, pr.head_sha) == nil
    or not M.has_review_result_marker(pr.comments, review_proposal, proposal_id, "approve", review_dedup) then
    local request = M.build_entity_comment_request({
      kind = "pr",
      repo = repo,
      number = pr.number,
    }, lifecycle_comments(repo, issue_number, pr, pr.head_sha), M._dedup_key({
      "substrate-ref-bump",
      "lifecycle",
      M.safe_repo(repo),
      tostring(issue_number),
      tostring(number),
      tostring(pr.head_sha),
    }), M.pr_source_ref(repo, number))
    local label_request = M.build_state_label_request(
      repo,
      issue_number,
      "merge-ready",
      M._dedup_key({
        "substrate-ref-bump",
        "label",
        M.safe_repo(repo),
        tostring(issue_number),
        tostring(number),
        tostring(pr.head_sha),
      }),
      M.issue_source_ref(repo, issue_number)
    )
    M.log_raise("substrate_ref_scan", proposal_id, "github-proxy.github_pr_comment_request", request)
    M.log_raise("substrate_ref_scan", proposal_id, "github-proxy.github_issue_label_request", label_request)
    raise("github-proxy.github_pr_comment_request", request)
    raise("github-proxy.github_issue_label_request", label_request)
    return nil
  end
  local payload = M.build_devloop_merge_ready_payload(proposal_id, number, version, {
    review_proposal_id = review_proposal,
    review_dedup_key = review_dedup,
    reviewed_head_sha = pr.head_sha,
  }, M.pr_source_ref(repo, number))
  M.log_raise("substrate_ref_scan", proposal_id, "devloop_merge_ready", payload)
  raise("devloop_merge_ready", payload)
  return payload
end

function M.substrate_ref_constants()
  return {
    path = substrate_ref_path,
    remote = substrate_remote,
    branch = substrate_branch,
    bump_branch = bump_branch,
    title = bump_title,
    lifecycle_version = lifecycle_version_prefix,
  }
end

function M.substrate_ref_scan()
  local cfg = M.devloop_config()
  local repo = require_repo(cfg.repo)
  if cfg.write_mode == "real" then
    M.assert_trusted_bot_configured()
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
    }
  end

  local final_existing = nil
  local branch_action = nil
  local created_pr_number = nil
  with_lock("github-devloop/substrate-ref/" .. M.safe_repo(repo), function()
    final_existing = existing_bump_pr(repo)
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

  if branch_action == "base-current" then
    return { status = "current", pin = current_pin, target = target_sha }
  end
  local merge_payload = ensure_bump_lifecycle(repo, cfg.upstream_branch, final_existing or { number = created_pr_number }, current_pin, target_sha)
  return {
    status = final_existing == nil and "created" or "updated",
    pin = current_pin,
    target = target_sha,
    existing_pr = final_existing and final_existing.number or nil,
    pr_number = final_existing and final_existing.number or created_pr_number,
    branch = bump_branch,
    merge_ready = merge_payload,
  }
end
end

return S
