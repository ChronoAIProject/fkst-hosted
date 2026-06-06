local base = require("tests.devloop_base_helpers")
local t = base.t
local core = base.core
local action_label = base.action_label
local reason_label = base.reason_label
local json_string = base.json_string
local render_comment = base.render_comment
local function merge_comments(event, branch, impl_version)
  local version = event.version
  return {
    core.state_marker(event.proposal_id, "merge-ready", version),
    core.pr_link_marker(event.proposal_id, event.pr_number, branch or "devloop-owner-repo-42-01HY", impl_version or version),
    core.merge_ready_marker(event.proposal_id, event.pr_number, version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
  }
end

local function mock_pr_origin(comments, head, head_sha, state)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  t.mock_command("--json headRefName,headRefOid,state,comments", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","state":"%s","comments":[%s]}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      table.concat(rendered_comments, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function review_json(review_state, review_head)
  if review_state == "REVIEW_REQUIRED" then
    return "[]"
  end
  if tostring(review_state or ""):sub(1, 1) == "[" then
    return tostring(review_state)
  end
  return string.format(
    '[{"state":"%s","commit":{"oid":"%s"}}]',
    json_string(review_state or "APPROVED"),
    json_string(review_head or "def456")
  )
end

local function mock_pr_merge(comments, head, head_sha, state, head_repo, cross_repo, mergeable, merge_state, rollup_state, rollup_conclusion, merged_at, review_decision, review_head)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup,latestReviews", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","state":"%s","mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"name":"ci","state":"%s","conclusion":"%s"}],"latestReviews":%s}\n',
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
      json_string(rollup_conclusion or "SUCCESS"),
      review_json(review_decision, review_head or head_sha or "def456")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_merge_rollup(comments, rollup_json, head, head_sha, state, head_repo, cross_repo, mergeable, merge_state, merged_at, review_decision, review_head)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup,latestReviews", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","state":"%s","mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":%s,"latestReviews":%s}\n',
      json_string(head or "devloop-owner-repo-42-01HY"),
      json_string(head_sha or "def456"),
      json_string(state or "OPEN"),
      json_string(merged_at or ""),
      table.concat(rendered_comments, ","),
      json_string(head_repo or "owner/repo"),
      cross,
      json_string(mergeable or "MERGEABLE"),
      json_string(merge_state or "CLEAN"),
      rollup_json or '[{"name":"ci","state":"COMPLETED","conclusion":"SUCCESS"}]',
      review_json(review_decision, review_head or head_sha or "def456")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merging_comment(exit_code, stderr)
  t.mock_command("gh issue comment '42' --repo 'owner/repo' --body-file", {
    stdout = "commented\n",
    stderr = stderr or "",
    exit_code = exit_code or 0,
  })
end

local function mock_pr_merge_command(exit_code, stderr)
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
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local cross = "false"
  if cross_repo == true then
    cross = "true"
  end
  t.mock_command("--json headRefName,headRefOid,state,comments,headRepository,headRepositoryOwner,isCrossRepository", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","state":"%s","comments":[%s],"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s}\n',
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

local function mock_pr_origin_sequence(entries)
  for _, entry in ipairs(entries or {}) do
    mock_pr_origin(entry.comments or {}, entry.head, entry.head_sha, entry.state)
  end
end

local function mock_pr_head(head, state)
  t.mock_command("--json headRefName", {
    stdout = string.format('{"headRefName":"%s","state":"%s"}\n', json_string(head or "devloop-owner-repo-42-01HY"), json_string(state or "OPEN")),
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


return {
  merge_comments = merge_comments,
  mock_pr_origin = mock_pr_origin,
  review_json = review_json,
  mock_pr_merge = mock_pr_merge,
  mock_pr_merge_rollup = mock_pr_merge_rollup,
  mock_merging_comment = mock_merging_comment,
  mock_pr_merge_command = mock_pr_merge_command,
  has_call = has_call,
  mock_issue_close = mock_issue_close,
  merge_comments_with_merging = merge_comments_with_merging,
  mock_pr_fix = mock_pr_fix,
  mock_pr_origin_sequence = mock_pr_origin_sequence,
  mock_pr_head = mock_pr_head,
  mock_pr_diff = mock_pr_diff,
  mock_branch_exists = mock_branch_exists,
  mock_meta_codex = mock_meta_codex,
}
