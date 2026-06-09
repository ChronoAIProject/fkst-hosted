local base = require("tests.devloop_base_helpers")
local t = base.t
local core = base.core
local action_label = base.action_label
local reason_label = base.reason_label
local json_string = base.json_string
local render_comment = base.render_comment
local last_merge_comments = nil
local function review_result_approve_marker(event)
  return core.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key)
end

local function append_merged_pr_merging_fact(comments, pr_state)
  if tostring(pr_state or "OPEN") ~= "MERGED" then
    return comments
  end
  local has_merging = false
  local proposal_id = nil
  local version = nil
  local head_sha = nil
  for _, comment in ipairs(comments or {}) do
    local body = type(comment) == "table" and comment.body or comment
    if tostring(body or ""):find("fkst:github-devloop:merging:v1", 1, true) ~= nil then
      has_merging = true
    end
    for marker in tostring(body or ""):gmatch("<!%-%- fkst:github%-devloop:merge%-ready:v1.-%-%->") do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if core.parse_entity_proposal_id(marker_proposal) ~= nil and core.is_safe_head_sha(marker_head_sha) then
        proposal_id = marker_proposal
        version = marker_version
        head_sha = marker_head_sha
      end
    end
  end
  if has_merging or proposal_id == nil then
    return comments
  end
  local merged = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(merged, comment)
  end
  table.insert(merged, core.state_marker(proposal_id, "merging", version))
  table.insert(merged, core.merging_marker(proposal_id, 7, version, head_sha or "def456"))
  return merged
end

local function merge_comments(event, branch, impl_version, include_review_result)
  local version = event.version
  local comments = {
    core.pr_origin_marker(event.proposal_id, 42, branch or "devloop-owner-repo-42-01HY", impl_version or version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", version),
    core.merge_ready_marker(event.proposal_id, event.pr_number, version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
  }
  if include_review_result ~= false then
    table.insert(comments, review_result_approve_marker(event))
  end
  return comments
end

local function pr_native_comments(event, include_review_result)
  local comments = {
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    core.merge_ready_marker(event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
  }
  if include_review_result ~= false then
    table.insert(comments, review_result_approve_marker(event))
  end
  return comments
end

local function mock_pr_origin(comments, head, head_sha, state, base_branch)
  local input_comments = comments
  local cached = base.take_pr_phase_comments()
  local has_state_marker = false
  for _, comment in ipairs(input_comments or {}) do
    if tostring(type(comment) == "table" and comment.body or comment):find("fkst:github-devloop:state:v1", 1, true) ~= nil then
      has_state_marker = true
    end
  end
  if cached == nil and input_comments ~= nil and #input_comments > 0 and not has_state_marker then
    base.set_pending_pr_origin({
      repo = "owner/repo",
      pr_number = 7,
      comments = input_comments,
      head = head or "devloop-owner-repo-42-01HY",
      head_sha = head_sha or "def456",
      state = state or "OPEN",
      base_branch = base_branch or "dev",
    })
    return
  end
  if input_comments == nil or #input_comments == 0 then
    input_comments = cached or {}
  elseif cached ~= nil then
    local merged = {}
    for _, comment in ipairs(input_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(cached) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  if tostring(state or "OPEN") == "MERGED" and last_merge_comments ~= nil then
    local merged = {}
    for _, comment in ipairs(last_merge_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(input_comments or {}) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  input_comments = append_merged_pr_merging_fact(input_comments, state)
  if tostring(state or "OPEN") ~= "MERGED" then
    last_merge_comments = input_comments
  end
  local rendered_comments = {}
  for _, comment in ipairs(input_comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"%s","state":"%s","updatedAt":"2026-06-03T02:03:04Z","comments":[%s]}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(base_branch or "dev"),
      json_string(state or "OPEN"),
      table.concat(rendered_comments, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_merge(comments, head, head_sha, state, head_repo, cross_repo, mergeable, merge_state, rollup_state, rollup_conclusion, merged_at)
  local input_comments = comments
  local cached = base.take_pr_phase_comments()
  if input_comments == nil or #input_comments == 0 then
    input_comments = cached or last_merge_comments or {}
  elseif cached ~= nil then
    local merged = {}
    for _, comment in ipairs(input_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(cached) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  if cached == nil
    and last_merge_comments ~= nil
    and tostring(state or "OPEN") == "OPEN"
    and (comments == nil or #comments == 0) then
    local merged = {}
    for _, comment in ipairs(last_merge_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(input_comments or {}) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  input_comments = append_merged_pr_merging_fact(input_comments, state)
  local rendered_comments = {}
  for _, comment in ipairs(input_comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"%s","mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"name":"ci","state":"%s","conclusion":"%s"}]}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      json_string(merged_at or ""),
      table.concat(rendered_comments, ","),
      json_string(head_repo or "owner/repo"),
      cross,
      json_string(mergeable or "MERGEABLE"),
      json_string(merge_state or "CLEAN"),
      json_string(rollup_state or "COMPLETED"),
      json_string(rollup_conclusion or "SUCCESS")
    ),
    stderr = "",
    exit_code = 0,
  })
  if tostring(state or "OPEN") ~= "MERGED" then
    last_merge_comments = input_comments
  end
end

local function mock_pr_merge_rollup(comments, rollup_json, head, head_sha, state, head_repo, cross_repo, mergeable, merge_state, merged_at)
  local input_comments = comments
  local cached = base.take_pr_phase_comments()
  if input_comments == nil or #input_comments == 0 then
    input_comments = cached or last_merge_comments or {}
  elseif cached ~= nil then
    local merged = {}
    for _, comment in ipairs(input_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(cached) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  if tostring(state or "OPEN") == "MERGED" and last_merge_comments ~= nil then
    local merged = {}
    for _, comment in ipairs(last_merge_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(input_comments or {}) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  input_comments = append_merged_pr_merging_fact(input_comments, state)
  if tostring(state or "OPEN") ~= "MERGED" then
    last_merge_comments = input_comments
  end
  local rendered_comments = {}
  for _, comment in ipairs(input_comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"%s","mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":%s}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      json_string(merged_at or ""),
      table.concat(rendered_comments, ","),
      json_string(head_repo or "owner/repo"),
      cross,
      json_string(mergeable or "MERGEABLE"),
      json_string(merge_state or "CLEAN"),
      rollup_json or '[{"name":"ci","state":"COMPLETED","conclusion":"SUCCESS"}]'
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merging_comment(exit_code, stderr)
  t.mock_command("gh pr comment '7' --repo 'owner/repo' --body-file", {
    stdout = "commented\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_pr_merge_command(exit_code, stderr)
  mock_pr_merge(nil, "devloop-owner-repo-42-01HY", "def456", "OPEN")
  t.mock_command("gh pr merge '7' --repo 'owner/repo' --merge --match-head-commit 'def456'", {
    stdout = "merged\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function has_call(needle)
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function mock_issue_close(exit_code, stderr)
  t.mock_command("gh issue close", {
    stdout = "closed\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function merge_comments_with_merging(event, branch, impl_version)
  local comments = merge_comments(event, branch, impl_version)
  table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
  table.insert(comments, core.merging_marker(event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha))
  return comments
end

local function mock_pr_fix(comments, head, head_sha, state, head_repo, cross_repo)
  local rendered_comments = {}
  local cached = base.take_pr_phase_comments()
  local with_origin = {
    core.pr_origin_marker("github-devloop/issue/owner/repo/42", 42, head or "devloop-owner-repo-42-01HY", base.reviewing().version, "dev"),
  }
  local input_comments = comments
  if input_comments == nil or #input_comments == 0 then
    input_comments = cached or {}
  end
  for _, comment in ipairs(input_comments or {}) do
    table.insert(with_origin, comment)
  end
  for _, comment in ipairs(cached or {}) do
    table.insert(with_origin, comment)
  end
  for _, comment in ipairs(with_origin) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,comments,headRepository,headRepositoryOwner,isCrossRepository", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      table.concat(rendered_comments, ","),
      json_string(head_repo or "owner/repo"),
      cross
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_native_fix(comments, head, head_sha, state, head_repo, cross_repo)
  local rendered_comments = {}
  local cached = base.take_pr_phase_comments()
  local input_comments = comments
  if input_comments == nil or #input_comments == 0 then
    input_comments = cached or {}
  elseif cached ~= nil then
    local merged = {}
    for _, comment in ipairs(input_comments) do
      table.insert(merged, comment)
    end
    for _, comment in ipairs(cached) do
      table.insert(merged, comment)
    end
    input_comments = merged
  end
  for _, comment in ipairs(input_comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,comments,headRepository,headRepositoryOwner,isCrossRepository", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s}\n',
      json_string(head or "pr-native-branch"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      table.concat(rendered_comments, ","),
      json_string(head_repo or "owner/repo"),
      cross
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_origin_sequence(entries)
  for _, entry in ipairs(entries or {}) do
    local cached = base.take_pr_phase_comments()
    local comments = entry.comments
    if comments == nil then
      comments = {
        core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", entry.head or "devloop-owner-repo-42-01HY", base.reviewing().version, "dev"),
      }
    end
    if cached ~= nil then
      local merged = {}
      for _, comment in ipairs(comments) do
        table.insert(merged, comment)
      end
      for _, comment in ipairs(cached) do
        table.insert(merged, comment)
      end
      comments = merged
    end
    local rendered_comments = {}
    for _, comment in ipairs(comments or {}) do
      table.insert(rendered_comments, render_comment(comment))
    end
    t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
      stdout = string.format(
        '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","state":"%s","updatedAt":"2026-06-03T02:03:04Z","comments":[%s]}\n',
        json_string(entry.head or "devloop-owner-repo-42-01HY"),
        json_string(entry.head_sha or "def456"),
        json_string(entry.state or "OPEN"),
        table.concat(rendered_comments, ",")
      ),
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_head(head, state)
  t.mock_command("--json headRefName", {
    stdout = string.format('{"headRefName":"%s","baseRefName":"dev","state":"%s"}\n', json_string(head or "devloop-owner-repo-42-01HY"), json_string(state or "OPEN")),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_diff(diff, exit_code, stderr)
  t.mock_command("gh pr diff", {
    stdout = diff or "diff --git a/file.lua b/file.lua\n+return true\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_branch_exists(branch, head)
  t.mock_command("show-ref --verify --quiet", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("rev-parse --verify", {
    stdout = (head or "abc123") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_meta_codex(action, reason, exit_code)
  local stdout = ""
  if action ~= nil then
    stdout = action_label .. " " .. tostring(action) .. "\n" .. reason_label .. " " .. tostring(reason or "Reason.")
  end
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function reset_pr_helper_state()
  last_merge_comments = nil
end


return {
  merge_comments = merge_comments,
  pr_native_comments = pr_native_comments,
  review_result_approve_marker = review_result_approve_marker,
  mock_pr_origin = mock_pr_origin,
  mock_pr_merge = mock_pr_merge,
  mock_pr_merge_rollup = mock_pr_merge_rollup,
  mock_merging_comment = mock_merging_comment,
  mock_pr_merge_command = mock_pr_merge_command,
  has_call = has_call,
  mock_issue_close = mock_issue_close,
  merge_comments_with_merging = merge_comments_with_merging,
  mock_pr_fix = mock_pr_fix,
  mock_pr_native_fix = mock_pr_native_fix,
  mock_pr_origin_sequence = mock_pr_origin_sequence,
  mock_pr_head = mock_pr_head,
  mock_pr_diff = mock_pr_diff,
  mock_branch_exists = mock_branch_exists,
  mock_meta_codex = mock_meta_codex,
  reset_pr_helper_state = reset_pr_helper_state,
}
