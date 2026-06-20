local core = require("core")
local strings = require("std.strings")

local M = {}

M.spec = {
  consumes = { "github_pr_open_request" },
  produces = { "github_entity_changed", "github_pr_opened" },
  stall_window = "2m",
}

local MAX_RUNTIME_ID_LEN = 180

local function runtime_identity(repo, branch)
  local id = "pr-open-" .. strings.runtime_safe_segment(repo) .. "-" .. strings.runtime_safe_segment(branch)
  if #id > MAX_RUNTIME_ID_LEN then
    return id:sub(1, MAX_RUNTIME_ID_LEN)
  end
  return id
end

local function temp_body_file(repo, branch, kind)
  return "/tmp/fkst-github-proxy-" .. runtime_identity(repo, branch) .. "-" .. kind .. ".md"
end

local function lock_name(repo, branch)
  return "github-proxy/" .. runtime_identity(repo, branch)
end

local function normalize_labels(value)
  local labels = {}
  if type(value) ~= "table" then
    return labels
  end
  for _, label in ipairs(value) do
    if label ~= nil and tostring(label) ~= "" then
      table.insert(labels, tostring(label))
    end
  end
  return labels
end

local function render_pr_number_template(value, pr_number)
  return tostring(value or ""):gsub("{{pr_number}}", tostring(pr_number))
end

local function verify_pr_remote_head(repo, pr_number, expected_head_sha, expected_base_branch)
  local pr_head = core.gh_exec(function(timeout)
    return core.github_pr_view_head_oid(repo, pr_number, timeout)
  end, 30, "gh PR REST head repository/headRefOid/state")
  local remote_pr = core.parse_pr_view_head_state(pr_head.stdout, repo)
  if remote_pr == nil then
    error("github-proxy: gh PR REST head repository/headRefOid/state did not return a valid open PR fact")
  end
  if tostring(remote_pr.state):lower() ~= "open" then
    error("github-proxy: PR is not open")
  end
  if remote_pr.is_cross_repository ~= false or not remote_pr.is_target_repository then
    error("github-proxy: PR head repository is not the target repository")
  end
  if remote_pr.head_ref_oid ~= tostring(expected_head_sha):lower() then
    error("github-proxy: PR headRefOid does not match implementing head_sha")
  end
  if expected_base_branch ~= nil and tostring(remote_pr.base_ref_name or "") ~= tostring(expected_base_branch) then
    error("github-proxy: PR baseRefName does not match requested base_branch")
  end
end

local function guard_pr_open_write(repo, payload, bot_login)
  if payload.proposal_id == nil
    or payload.impl_version == nil
    or payload.expected_state == nil
    or payload.expected_version == nil
    or payload.head_sha == nil
    or payload.base_branch == nil then
    log.warn("github-proxy: PR open request missing write-time guard facts")
    return nil
  end
  if payload.expected_state ~= "implementing" or tostring(payload.expected_version) ~= tostring(payload.impl_version) then
    log.warn("github-proxy: PR open request has inconsistent expected state")
    return nil
  end
  if not core.is_safe_head_sha(payload.head_sha) then
    log.warn("github-proxy: PR open request has unsafe head_sha")
    return nil
  end
  if not core.is_safe_branch(payload.base_branch) then
    log.warn("github-proxy: PR open request has unsafe base_branch")
    return nil
  end

  local view = core.fetch_issue_view(repo, payload.issue_number, nil, { fresh = true, marker_bearing = true })
  if view.exit_code ~= 0 then
    error("github-proxy: gh issue REST guard failed")
  end

  local issue = core.parse_issue_state(view.stdout)
  if not core.verify_issue_claim_in_issue(issue, payload, repo, payload.issue_number, "github_pr_open") then
    return nil
  end
  local state = core.current_devloop_state(issue.comments, payload.proposal_id, bot_login)
  local has_pr_open_marker = core.has_devloop_pr_open_marker(issue.comments, payload.proposal_id, payload.impl_version, bot_login)
  if has_pr_open_marker then
    return {
      can_advance = false,
      pr_open_visible = true,
      state = state,
      labels = issue.labels,
    }
  end

  if state.state ~= payload.expected_state or tostring(state.version or "") ~= tostring(payload.expected_version) then
    log.warn("github-proxy: PR open skipped because current issue state is not implementing at requested version")
    return nil
  end
  local fact = core.devloop_implementing_fact(issue.comments, payload.proposal_id, payload.impl_version, bot_login)
  if fact == nil then
    log.warn("github-proxy: PR open skipped because implementing fact marker is not visible")
    return nil
  end
  if tostring(fact.branch) ~= tostring(payload.branch)
    or tostring(fact.base_branch) ~= tostring(payload.base_branch) then
    log.warn("github-proxy: PR open skipped because request does not match implementing fact marker")
    return nil
  end

  local branch_ref = core.git_show_ref_branch(payload.branch, 30)
  if branch_ref.exit_code ~= 0 then
    error("github-proxy: implementing branch ref missing before PR open: " .. tostring(branch_ref.stderr))
  end
  local current_head = core.parse_git_show_ref_head(branch_ref.stdout, payload.branch)
  if current_head == nil then
    log.warn("github-proxy: PR open skipped because branch ref output is invalid")
    return nil
  end
  if current_head ~= tostring(payload.head_sha):lower() then
    log.warn("github-proxy: PR open skipped because request head does not match current branch head")
    return nil
  end
  if current_head ~= tostring(fact.head_sha):lower() then
    local ancestry = core.git_is_ancestor(fact.head_sha, current_head, 30)
    if ancestry.exit_code ~= 0 then
      log.warn("github-proxy: PR open skipped because branch head is not descended from implementing fact")
      return nil
    end
  end

  return {
    can_advance = true,
    pr_open_visible = false,
    state = state,
    labels = issue.labels,
  }
end

local function can_apply_pr_open_labels(state, impl_version)
  if type(state) ~= "table" then
    return false
  end
  if tostring(state.version or "") ~= tostring(impl_version) then
    return false
  end
  return state.state == "implementing" or state.state == "pr-open"
end

local function raise_pr_entity_changed(repo, pr, payload)
  local updated_at = tostring(payload.dedup_key or "") .. "/pr/" .. tostring(pr.number)
  local entity_payload = {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = pr.number,
    title = tostring(payload.title or "PR #" .. tostring(pr.number)),
    url = pr.url,
    state = pr.state or "OPEN",
    updated_at = updated_at,
    dedup_key = core.entity_dedup_key(repo, "pr", pr.number, updated_at),
    source = "github_pr_open",
    source_ref = core.entity_source_ref(repo, "pr", pr.number),
  }
  raise("github_entity_changed", {
    schema = entity_payload.schema,
    type = entity_payload.type,
    repo = entity_payload.repo,
    number = entity_payload.number,
    title = entity_payload.title,
    url = entity_payload.url,
    state = entity_payload.state,
    updated_at = entity_payload.updated_at,
    dedup_key = entity_payload.dedup_key,
    source = entity_payload.source,
    source_ref = entity_payload.source_ref,
  })
  raise("github_pr_opened", {
    schema = "github-proxy.pr-opened.v1",
    repo = repo,
    issue_number = payload.issue_number,
    proposal_id = payload.proposal_id,
    impl_version = payload.impl_version,
    pr_number = pr.number,
    branch = payload.branch,
    head_sha = payload.head_sha,
    base_branch = payload.base_branch,
    dedup_key = tostring(payload.dedup_key or "") .. "/opened/" .. tostring(pr.number),
    source_ref = entity_payload.source_ref,
  })
end

local function current_issue_state_for_label_edit(repo, payload, bot_login)
  local view = core.fetch_issue_view(repo, payload.issue_number, nil, { fresh = true, marker_bearing = true })
  if view.exit_code ~= 0 then
    error("github-proxy: gh issue REST label guard failed")
  end
  local issue = core.parse_issue_state(view.stdout)
  return core.current_devloop_state(issue.comments, payload.proposal_id, bot_login)
end

function pipeline(event)
  local payload = event.payload or {}
  if payload.schema ~= "github-proxy.pr-open.v1" then
    log.warn("github-proxy: unsupported PR open request schema")
    return
  end

  local repo = payload.repo or core.read_env("FKST_GITHUB_REPO")
  if repo == nil or repo == "" then
    log.warn("github-proxy: PR open request missing repo")
    return
  end
  if payload.issue_number == nil or payload.dedup_key == nil then
    log.warn("github-proxy: PR open request missing issue_number or dedup_key")
    return
  end
  if not core.is_safe_branch(payload.branch) then
    log.warn("github-proxy: PR open request has unsafe branch")
    return
  end
  if payload.title == nil or payload.body == nil or payload.issue_comment_body_template == nil then
    log.warn("github-proxy: PR open request missing title/body/comment template")
    return
  end

  if core.read_env("FKST_GITHUB_WRITE") ~= "1" then
    log.info("github-proxy dry-run: would push/create PR for " .. tostring(repo) .. " branch " .. tostring(payload.branch))
    return
  end
  local bot_login = core.assert_trusted_bot_configured()

  with_lock(lock_name(repo, payload.branch), function()
    local guard = guard_pr_open_write(repo, payload, bot_login)
    if guard == nil then
      return
    end

    local existing = core.gh_exec(function(timeout)
      return core.github_pr_list_head(repo, payload.branch, payload.base_branch, timeout)
    end, 30, "gh pr list --head")
    local pr = core.parse_pr_list_for_head(existing.stdout, payload.branch)

    if pr == nil then
      if not guard.can_advance then
        error("github-proxy: pr-open marker exists but no matching open PR was found for branch")
      end

      local push = core.git_push_branch(payload.branch, 120)
      if push.exit_code ~= 0 then
        error("github-proxy: git push failed: " .. tostring(push.stderr))
      end

      local pr_body_path = temp_body_file(repo, payload.branch, "pr-body")
      local pr_create_body = core.with_github_debug_stamp(tostring(payload.body), {
        emitter = "github-proxy.pr-open",
        target = "pr:" .. tostring(repo) .. "#new",
        dedup_key = payload.dedup_key,
        context = payload.head_sha,
      })
      file.write(pr_body_path, pr_create_body)
      local created = core.gh_exec(function(timeout)
        return core.github_pr_create(repo, payload.branch, payload.base_branch, payload.title, pr_body_path, timeout)
      end, 60, "gh pr create")
      pr = core.parse_pr_create(created.stdout)
      if pr == nil then
        local listed = core.gh_exec(function(timeout)
          return core.github_pr_list_head(repo, payload.branch, payload.base_branch, timeout)
        end, 30, "gh pr list --head after create")
        pr = core.parse_pr_list_for_head(listed.stdout, payload.branch)
      end
      if pr == nil then
        error("github-proxy: gh pr create/list did not return a valid PR number")
      end
      verify_pr_remote_head(repo, pr.number, payload.head_sha, payload.base_branch)
      core.invalidate_entity_after_write(repo, "pr", pr.number)
    else
      verify_pr_remote_head(repo, pr.number, payload.head_sha, payload.base_branch)
      log.info("github-proxy: PR for head branch already exists; reusing #" .. tostring(pr.number))
    end

    if not guard.pr_open_visible then
      local issue_view = core.gh_exec(
        core.gh_issue_view_comments_cmd(repo, payload.issue_number),
        30,
        "gh issue REST comments after PR open"
      )
      if core.has_trusted_marker(core.parse_issue_comments(issue_view.stdout), payload.dedup_key, bot_login) then
        guard.pr_open_visible = true
      end
    end
    if not guard.pr_open_visible then
      local issue_body = render_pr_number_template(payload.issue_comment_body_template, pr.number)
        .. "\n\n" .. core.comment_marker(payload.dedup_key)
        .. "\n"
      issue_body = core.with_github_debug_stamp(issue_body, {
        emitter = "github-proxy.pr-open.issue-comment",
        target = "issue:" .. tostring(repo) .. "#" .. tostring(payload.issue_number),
        dedup_key = payload.dedup_key,
        context = pr.number,
      })
      local issue_body_path = temp_body_file(repo, payload.branch, "issue-comment")
      file.write(issue_body_path, issue_body)
      core.gh_exec(
        core.gh_issue_comment_cmd(repo, payload.issue_number, issue_body_path),
        30,
        "gh issue comment after PR open"
      )
      core.invalidate_entity_after_write(repo, "issue", payload.issue_number)
    end

    local pr_view = core.gh_exec(
      core.gh_pr_view_comments_cmd(repo, pr.number),
      30,
      "gh PR REST comments after PR open"
    )
    if not core.has_trusted_comment_fragment(core.parse_issue_comments(pr_view.stdout), tostring(payload.body), bot_login) then
      local pr_body = tostring(payload.body) .. "\n\n" .. core.comment_marker(payload.dedup_key) .. "\n"
      pr_body = core.with_github_debug_stamp(pr_body, {
        emitter = "github-proxy.pr-open.pr-comment",
        target = "pr:" .. tostring(repo) .. "#" .. tostring(pr.number),
        dedup_key = payload.dedup_key,
        context = payload.head_sha,
      })
      local pr_body_path = temp_body_file(repo, payload.branch, "pr-comment")
      file.write(pr_body_path, pr_body)
      core.gh_exec(
        core.gh_pr_comment_cmd(repo, pr.number, pr_body_path),
        30,
        "gh pr comment"
      )
      core.invalidate_entity_after_write(repo, "pr", pr.number)
    end

    local add_labels = normalize_labels(payload.issue_label_add)
    local remove_labels = normalize_labels(payload.issue_label_remove)
    if #add_labels > 0 or #remove_labels > 0 then
      with_lock(core.issue_label_lock_key(repo, payload.issue_number), function()
        local current_state = current_issue_state_for_label_edit(repo, payload, bot_login)
        if not can_apply_pr_open_labels(current_state, payload.impl_version) or current_state.state ~= "pr-open" then
          log.warn("github-proxy: PR open label update skipped because current issue state advanced past pr-open")
          return
        end
        core.apply_issue_labels(repo, payload.issue_number, add_labels, remove_labels)
      end)
    end

    raise_pr_entity_changed(repo, pr, payload)
  end)
end

pipeline = core.wrap_pipeline_failure("github_pr_open", pipeline)

return M
