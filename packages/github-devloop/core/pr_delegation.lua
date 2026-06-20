local S = {}

function S.install(M)
local function issue_fields(issue, impl_version)
  if type(issue) ~= "table" then
    error("github-devloop: invalid delegation issue")
  end
  local repo = issue.repo
  local issue_number = issue.number or issue.issue_number
  local proposal_id = issue.proposal_id or M.proposal_id(repo, issue_number)
  if M.parse_proposal_id(proposal_id) == nil then
    error("github-devloop: invalid delegation issue proposal")
  end
  if not M._is_bounded_string(impl_version, M._max_dedup_len) then
    error("github-devloop: invalid delegation implementation version")
  end
  return repo, issue_number, proposal_id
end

local function delegation_key(proposal_id, impl_version, generation)
  local value = "g" .. tostring(generation or 1)
  value = value:gsub(":", "-")
  if not M._is_path_safe_key(value, M._max_dedup_len) then
    error("github-devloop: invalid delegation generation")
  end
  return value
end

local function branch_for(repo, issue_number, impl_version)
  return M.implement_branch(repo, issue_number, M.implementation_base_version(impl_version))
end

local function parse_open_prs_for_branch(stdout, branch, base_branch)
  local found = {}
  for _, pr in ipairs(M.parse_pr_list_head_base(stdout)) do
    if tostring(pr.head_ref_name or "") == tostring(branch)
      and (base_branch == nil or tostring(pr.base_ref_name or "") == tostring(base_branch))
      and tostring(pr.state or ""):lower() ~= "closed" then
      table.insert(found, pr)
    end
  end
  table.sort(found, function(a, b) return tonumber(a.number or 0) < tonumber(b.number or 0) end)
  return found
end

local function find_pr(repo, branch, base_branch)
  local listed = M.gh_pr_list_head_base(repo, branch, base_branch, 30)
  if listed.exit_code ~= 0 then
    error("github-devloop: pr-delegation PR list failed: " .. tostring(listed.stderr))
  end
  local prs = parse_open_prs_for_branch(listed.stdout, branch, base_branch)
  if #prs == 0 then
    return nil
  end
  return prs[1]
end

local function create_pr(repo, issue_number, branch, base_branch, title, body)
  local effective_title = tostring(title or "")
  if effective_title == "" then
    effective_title = "github-devloop implementation for #" .. tostring(issue_number)
  end
  if #effective_title > M._max_pr_title_len then
    effective_title = M.truncate_utf8(effective_title, M._max_pr_title_len)
  end
  local created = M.gh_pr_create_body(repo, branch, base_branch, effective_title, body, 60)
  if created.exit_code ~= 0 then
    error("github-devloop: pr-delegation PR create failed: " .. tostring(created.stderr))
  end
end

local function require_head_sha(branch, expected_head)
  local head = tostring(expected_head or "")
  if M.is_safe_head_sha(head) then
    return head
  end
  local fetched = M.current_branch_head_sha(branch)
  if not M.is_safe_head_sha(fetched) then
    error("github-devloop: pr-delegation branch head is missing")
  end
  return fetched
end

local function build_pr_open_comment_request(repo, pr_number, pr_proposal_id, issue_proposal_id, issue_number, impl_version, branch, base_branch, head_sha, source_ref, delegation)
  if not M.is_safe_head_sha(head_sha) then
    error("github-devloop: invalid pr-delegation head sha")
  end
  local body = "github-devloop PR child open"
    .. "\n\n" .. M.pr_origin_marker(pr_proposal_id, pr_number, branch, impl_version, base_branch)
    .. "\n" .. M.state_marker(pr_proposal_id, "pr-open", impl_version)
  local request = M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, body, M._dedup_key({
    "pr-delegation",
    "pr-open",
    tostring(issue_proposal_id),
    tostring(delegation),
  }), source_ref)
  request.handoff = {
    kind = "github-devloop.pr-open",
    issue_proposal_id = issue_proposal_id,
    proposal_id = pr_proposal_id,
    pr_number = pr_number,
    version = impl_version,
    delegation_generation = delegation,
    branch = branch,
    base_branch = base_branch,
    head_sha = head_sha,
    source_ref = M.normalize_source_ref(source_ref),
  }
  return request
end

local function build_issue_delegation_comment_request(repo, issue_number, issue_proposal_id, pr_proposal_id, pr_number, impl_version, delegation, source_ref)
  local marker = M.pr_delegation_marker(issue_proposal_id, pr_proposal_id, pr_number, impl_version, delegation)
  return M.build_entity_comment_request({
    kind = "issue",
    repo = repo,
    number = issue_number,
  }, "github-devloop delegated implementation to PR #" .. tostring(pr_number)
    .. "\n\n" .. marker, M._dedup_key({
    "pr-delegation",
    "issue",
    tostring(issue_proposal_id),
    tostring(impl_version),
    tostring(pr_number),
    tostring(delegation),
  }), source_ref)
end

local function existing_delegation(issue, issue_proposal_id, delegation)
  local fact = M.pr_delegation_fact(issue and issue.comments, issue_proposal_id, nil, delegation)
  if fact == nil then
    return nil
  end
  return {
    number = fact.pr_number,
    pr_number = fact.pr_number,
    head_ref_name = issue.branch or (issue.implementation and issue.implementation.branch),
    base_ref_name = issue.base_branch or (issue.implementation and issue.implementation.base_branch),
  }
end

function M.build_devloop_pr_open_payload(issue_proposal_id, pr_proposal_id, pr_number, version, source_ref, delegation_generation, branch, base_branch, head_sha)
  return {
    schema = "github-devloop.pr-open.v1",
    proposal_id = pr_proposal_id,
    issue_proposal_id = issue_proposal_id,
    pr_number = pr_number,
    version = version,
    delegation_generation = delegation_generation,
    branch = branch,
    base_branch = base_branch,
    head_sha = head_sha,
    dedup_key = M._dedup_key({
      "pr-open",
      tostring(issue_proposal_id),
      tostring(delegation_generation),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_pr_delegation_open_comment_request(repo, pr_number, issue_proposal_id, pr_proposal_id, issue_number, impl_version, branch, base_branch, head_sha, source_ref, delegation)
  return build_pr_open_comment_request(repo, pr_number, pr_proposal_id, issue_proposal_id, issue_number, impl_version, branch, base_branch, head_sha, source_ref, delegation)
end

function M.ensure_pr_child(issue, impl_version, generation)
  local repo, issue_number, issue_proposal_id = issue_fields(issue, impl_version)
  local base_branch = issue.base_branch or (issue.implementation and issue.implementation.base_branch) or M.branch_config().integration
  local branch = issue.branch or (issue.implementation and issue.implementation.branch) or branch_for(repo, issue_number, impl_version)
  local delegation = delegation_key(issue_proposal_id, impl_version, generation or 1)
  local pr = existing_delegation(issue, issue_proposal_id, delegation)
  if pr == nil then
    pr = find_pr(repo, branch, base_branch)
  end
  if pr == nil then
    local head_sha = require_head_sha(branch, issue.head_sha or (issue.implementation and issue.implementation.head_sha))
    local body = "github-devloop implementation PR for issue #" .. tostring(issue_number)
    create_pr(repo, issue_number, branch, base_branch, issue.title, body)
    pr = find_pr(repo, branch, base_branch)
    if pr == nil then
      error("github-devloop: pr-delegation PR create did not yield an adoptable branch PR")
    end
    if pr.head_sha ~= nil and tostring(pr.head_sha):lower() ~= tostring(head_sha):lower() then
      error("github-devloop: pr-delegation created PR head mismatch")
    end
  end
  if not M.is_safe_pr_number(pr.number) then
    error("github-devloop: pr-delegation adopted invalid PR")
  end
  local pr_number = tonumber(pr.number)
  local pr_source_ref = M.pr_source_ref(repo, pr_number)
  local pr_proposal_id = M.pr_proposal_id(repo, pr_number)
  local head_sha = pr.head_sha or issue.head_sha or (issue.implementation and issue.implementation.head_sha)
  local effects = {}
  local pr_origin = M.pr_origin_fact(issue.pr_comments or {})
  local pr_state = M.current_entity_state(issue.pr_comments or {}, pr_proposal_id)
  if pr_origin == nil or tostring(pr_origin.proposal_id or "") ~= pr_proposal_id or pr_state.state ~= "pr-open" or tostring(pr_state.version or "") ~= tostring(impl_version) then
    table.insert(effects, {
      queue = "github-proxy.github_pr_comment_request",
      payload = build_pr_open_comment_request(repo, pr_number, pr_proposal_id, issue_proposal_id, issue_number, impl_version, branch, base_branch, head_sha, pr_source_ref, delegation),
    })
  end
  local delegation_fact = existing_delegation(issue, issue_proposal_id, delegation)
  if delegation_fact == nil or tonumber(delegation_fact.pr_number) ~= pr_number then
    table.insert(effects, {
      queue = "github-proxy.github_issue_comment_request",
      payload = build_issue_delegation_comment_request(repo, issue_number, issue_proposal_id, pr_proposal_id, pr_number, impl_version, delegation, M.issue_source_ref(repo, issue_number)),
    })
  end
  return {
    issue_proposal_id = issue_proposal_id,
    pr_proposal_id = pr_proposal_id,
    pr_number = pr_number,
    pr_source_ref = pr_source_ref,
    branch = branch,
    base_branch = base_branch,
    head_sha = head_sha,
    delegation_generation = delegation,
    effects = effects,
  }
end
end

return S
