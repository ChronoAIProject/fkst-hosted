local S = {}

function S.install(M)
local max_decompose_issues = 3
local max_decompose_depth = 1
local fallback_title = "Rework blocked PR with a smaller or alternative approach"

local function bounded_text(value, limit, fallback)
  local text = tostring(value or "")
  if text == "" then
    text = fallback or "(empty)"
  end
  if #text > limit then
    text = M._utf8_safe_truncate(text, limit)
  end
  return text
end

local function issue_fingerprint(decompose, index)
  return M._decimal_checksum(table.concat({
    tostring(decompose.proposal_id or ""),
    tostring(decompose.version or ""),
    tostring(decompose.pr_number or ""),
    tostring(decompose.round or ""),
    tostring(index or ""),
  }, "\n"))
end

function M.build_devloop_decompose_payload(fix_reconcile)
  return {
    schema = "github-devloop.decompose.v1",
    proposal_id = fix_reconcile.proposal_id,
    pr_number = fix_reconcile.pr_number,
    version = fix_reconcile.issue_version,
    review_proposal_id = fix_reconcile.review_proposal_id,
    review_dedup_key = fix_reconcile.review_dedup_key,
    head_sha = fix_reconcile.head_sha,
    round = fix_reconcile.round,
    dedup_key = M._dedup_key({
      "decompose",
      tostring(fix_reconcile.proposal_id),
      tostring(fix_reconcile.issue_version),
    }),
    source_ref = M.normalize_source_ref(fix_reconcile.source_ref),
  }
end

function M.is_supported_decompose(payload)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return payload.schema == "github-devloop.decompose.v1"
    and repo ~= nil
    and issue_number ~= nil
    and M._is_path_safe_key(payload.proposal_id, M._max_key_len)
    and M._is_positive_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M._is_path_safe_key(payload.review_proposal_id, M._max_key_len)
    and M._is_bounded_string(payload.review_dedup_key, M._max_dedup_len)
    and M._is_git_sha(payload.head_sha)
    and tonumber(payload.round) ~= nil
    and tonumber(payload.round) == M.version_fix_round(payload.version)
    and M._is_path_safe_key(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == M._dedup_key({
      "decompose",
      tostring(payload.proposal_id),
      tostring(payload.version),
    })
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.decomposed_marker(proposal_id, version, pr_number, count)
  local issue_count = tonumber(count)
  if issue_count == nil or issue_count < 1 or issue_count > max_decompose_issues or issue_count % 1 ~= 0 then
    error("github-devloop: invalid decomposed count")
  end
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid decomposed pr number")
  end
  return '<!-- fkst:github-devloop:decomposed:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" pr="' .. tostring(pr_number)
    .. '" count="' .. tostring(issue_count)
    .. '" -->'
end

function M.has_decomposed_marker(comments, proposal_id, version, pr_number)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:decomposed:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == tostring(proposal_id)
        and marker:match('version="([^"]*)"') == tostring(version)
        and tostring(marker:match('pr="([^"]+)"')) == tostring(pr_number) then
        return true
      end
    end
  end
  return false
end

function M.decompose_child_marker(proposal_id, version, pr_number, index)
  return '<!-- fkst:github-devloop:decompose-child:v1 parent="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" pr="' .. tostring(pr_number)
    .. '" index="' .. tostring(index)
    .. '" -->'
end

function M.decompose_lineage_marker(root_proposal_id, depth)
  local n = tonumber(depth)
  if n == nil or n < 0 or n % 1 ~= 0 then
    error("github-devloop: invalid decompose lineage depth")
  end
  return '<!-- fkst:github-devloop:decompose-lineage:v1 root="' .. tostring(root_proposal_id)
    .. '" depth="' .. tostring(n)
    .. '" -->'
end

function M.decompose_lineage_depth(body)
  local text = tostring(body or "")
  local marker_pattern = "<!%-%- fkst:github%-devloop:decompose%-lineage:v1.-%-%->"
  local max_depth = 0
  for marker in text:gmatch(marker_pattern) do
    local depth = tonumber(marker:match('depth="(%d+)"'))
    if depth ~= nil and depth > max_depth then
      max_depth = depth
    end
  end
  return max_depth
end

function M.parse_decompose_plan(stdout)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" or type(decoded.issues) ~= "table" then
    return nil
  end
  local issues = {}
  for _, issue in ipairs(decoded.issues) do
    if #issues >= max_decompose_issues then
      break
    end
    if type(issue) ~= "table"
      or not M._is_bounded_string(issue.title, M._max_title_len)
      or not M._is_bounded_string(issue.body, M._max_body_len) then
      return nil
    end
    table.insert(issues, {
      title = bounded_text(issue.title, M._max_title_len, fallback_title),
      body = bounded_text(issue.body, M._max_body_len, "Define a smaller independently-completable follow-up."),
    })
  end
  if #issues < 1 then
    return nil
  end
  return issues
end

function M.fallback_decompose_plan(decompose)
  return {
    {
      title = "Rework blocked PR #" .. tostring(decompose.pr_number) .. " with a smaller or alternative approach",
      body = "The parent PR repeatedly failed review after " .. tostring(decompose.round)
        .. " fix rounds. Rework it as a smaller independently-completable issue, or choose an alternative implementation approach that avoids repeating the same failed fix path.",
    },
  }
end

function M.decomposed_comment_body(decompose, count)
  return "github-devloop decomposed blocked PR into " .. tostring(count) .. " follow-up issue(s)"
    .. "\n\n" .. M.decomposed_marker(decompose.proposal_id, decompose.version, decompose.pr_number, count)
end

function M.build_issue_create_request(repo, decompose, issue, index)
  local safe_title = bounded_text(issue.title, M._max_title_len, fallback_title)
  local parent_summary = "Parent issue: #" .. tostring(select(2, M.parse_proposal_id(decompose.proposal_id)) or "unknown")
    .. "\nParent PR: #" .. tostring(decompose.pr_number)
    .. "\nBlocked reason: fix loop reached " .. tostring(decompose.round) .. " rounds and was reconciled to blocked."
  local body = parent_summary
    .. "\n\nSmaller scope / alternative approach:\n" .. M.neutralize_untrusted_comment_text(issue.body)
    .. "\n\nNon-goals:\n- Do not repeat the same high-round fix path without reducing scope or changing approach."
    .. "\n\nAcceptance:\n- The work is independently reviewable."
    .. "\n- The implementation can pass the normal intake, consensus, implementation, and review pipeline."
    .. "\n\n" .. M.decompose_lineage_marker(decompose.proposal_id, M.decompose_lineage_depth(decompose.current_issue_body) + 1)
    .. "\n\n" .. M.decompose_child_marker(decompose.proposal_id, decompose.version, decompose.pr_number, index)
  if #body > M._max_body_len then
    body = M._utf8_safe_truncate(body, M._max_body_len)
  end
  local fingerprint = issue_fingerprint(decompose, index)
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = safe_title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = M._dedup_key({
      "decompose",
      tostring(decompose.proposal_id),
      tostring(decompose.version),
      tostring(index),
      fingerprint,
    }),
    parent_comment_target = {
      repo = repo,
      pr_number = decompose.pr_number,
    },
    source_ref = M.normalize_source_ref(decompose.source_ref),
  }
end

function M.max_decompose_issues()
  return max_decompose_issues
end

function M.max_decompose_depth()
  return max_decompose_depth
end
end

return S
