local S = {}

function S.install(M)
local function bounded_framing(M, framing)
  if framing == nil then
    return nil
  end
  local value = tostring(framing)
  if #value > M._max_framing_len then
    value = value:sub(1, M._max_framing_len)
  end
  return value
end

local function board_digest_issue_list_cmd(M, repo)
  return "gh issue list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit 100"
    .. " --json number,title,labels"
end

local function board_digest_pr_list_cmd(M, repo)
  return "gh pr list"
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --state open"
    .. " --limit 100"
    .. " --json number,title,labels"
end

local function label_names(labels_json)
  local labels = {}
  for _, label in ipairs(labels_json or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
end

local function parse_board_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  if type(decoded) ~= "table" then
    return items
  end
  for _, item in ipairs(decoded) do
    if type(item) == "table" and tonumber(item.number) ~= nil then
      table.insert(items, {
        number = tonumber(item.number),
        title = tostring(item.title or ""),
        labels = label_names(item.labels),
      })
    end
  end
  return items
end

local function state_label(M, labels)
  for _, label in ipairs(labels or {}) do
    local text = tostring(label)
    if M._state_labels[text] then
      return text
    end
  end
  return "open"
end

local function first_chars(value, limit)
  local text = tostring(value or ""):gsub("[%s]+", " ")
  if #text > limit then
    return text:sub(1, limit)
  end
  return text
end

local function render_board_digest(M, issues, prs)
  local lines = {
    M._untrusted_issue_data_begin,
    "Open items snapshot:",
  }
  for _, item in ipairs(issues or {}) do
    if #lines >= 52 then
      break
    end
    table.insert(lines, "#" .. tostring(item.number)
      .. " [" .. state_label(M, item.labels) .. "] "
      .. first_chars(item.title, 60))
  end
  for _, item in ipairs(prs or {}) do
    if #lines >= 52 then
      break
    end
    table.insert(lines, "#" .. tostring(item.number)
      .. " [" .. state_label(M, item.labels) .. "] "
      .. first_chars(item.title, 60))
  end
  table.insert(lines, M._untrusted_issue_data_end)
  return table.concat(lines, "\n")
end

function M.board_digest_block(repo, tick)
  if tick == nil or tostring(tick) == "" then
    return ""
  end
  local key = "github-devloop/board-digest/" .. M.safe_repo(repo) .. "/" .. M.safe_updated_at(tick)
  local cached = cache_get(key)
  if cached ~= nil and cached ~= "" then
    return cached
  end

  local ok_issue, issue_result = pcall(exec_sync, { cmd = board_digest_issue_list_cmd(M, repo), timeout = 30 })
  local ok_pr, pr_result = pcall(exec_sync, { cmd = board_digest_pr_list_cmd(M, repo), timeout = 30 })
  if not ok_issue or not ok_pr
    or type(issue_result) ~= "table" or issue_result.exit_code ~= 0
    or type(pr_result) ~= "table" or pr_result.exit_code ~= 0 then
    return ""
  end

  local block = render_board_digest(M, parse_board_list(issue_result.stdout), parse_board_list(pr_result.stdout))
  cache_set(key, block)
  return block
end

function M.append_board_digest_to_proposal(proposal, repo, tick)
  local block = M.board_digest_block(repo, tick)
  if block == "" then
    return proposal
  end
  local body = tostring(proposal.body or "")
  local prefix = "\n\n"
  local neutralized = M.neutralize_untrusted_prompt_text(block)
  local remaining = M._max_body_len - #body - #prefix
  if remaining <= 0 then
    M.log_line("warn", "payloads", proposal.proposal_id, "BOARD_DIGEST", {
      "outcome=drop",
      "reason=body-budget-exhausted",
      "repo=" .. tostring(repo or ""),
      "tick=" .. tostring(tick or ""),
    })
    return proposal
  end
  if #neutralized > remaining then
    M.log_line("warn", "payloads", proposal.proposal_id, "BOARD_DIGEST", {
      "outcome=truncate",
      "reason=body-budget",
      "repo=" .. tostring(repo or ""),
      "tick=" .. tostring(tick or ""),
      "available=" .. tostring(remaining),
      "needed=" .. tostring(#neutralized),
    })
    neutralized = neutralized:sub(1, remaining)
  end
  proposal.body = body .. prefix .. neutralized
  if #proposal.body > M._max_body_len then
    error("github-devloop: proposal board digest exceeds bounded body")
  end
  return proposal
end

function M.build_devloop_ready_payload(source)
  local payload = {
    schema = "github-devloop.ready.v1",
    proposal_id = source.proposal_id,
    dedup_key = M._dedup_key({
      "ready",
      tostring(source.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source.source_ref),
  }
  local framing = bounded_framing(M, source.framing)
  if framing ~= nil then
    payload.framing = framing
  end
  return payload
end

function M.build_devloop_reviewing_payload(origin, pr_number, source_ref, version)
  local review_version = version or origin.impl_version
  return {
    schema = "github-devloop.reviewing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = review_version,
    dedup_key = M._dedup_key({
      "reviewing",
      tostring(origin.proposal_id),
      tostring(review_version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_fixing_payload(origin, pr_number, review_fact, source_ref)
  local version = origin.impl_version
  if review_fact.fix_version ~= nil then
    version = review_fact.fix_version
  end
  local payload = {
    schema = "github-devloop.fixing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact.review_proposal_id,
    review_dedup_key = review_fact.review_dedup_key,
    reviewed_head_sha = review_fact.reviewed_head_sha,
    dedup_key = M._dedup_key({
      "fixing",
      tostring(origin.proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact.review_dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
  local framing = bounded_framing(M, review_fact.framing or origin.framing)
  if framing ~= nil then
    payload.framing = framing
  end
  return payload
end

function M.build_devloop_review_meta_payload(unresolved, issue_proposal_id, issue_version, pr_number, n, source_ref)
  return {
    schema = "github-devloop.review-meta.v1",
    proposal_id = issue_proposal_id,
    review_proposal_id = unresolved.proposal_id,
    review_dedup_key = unresolved.dedup_key,
    version = issue_version,
    pr_number = pr_number,
    n = n,
    dedup_key = M._dedup_key({
      "review-meta",
      tostring(issue_proposal_id),
      tostring(issue_version),
      tostring(pr_number),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_devloop_merge_ready_payload(issue_proposal_id, pr_number, version, review_fact, source_ref)
  return {
    schema = "github-devloop.merge-ready.v1",
    proposal_id = issue_proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact and review_fact.review_proposal_id,
    review_dedup_key = review_fact and review_fact.review_dedup_key,
    reviewed_head_sha = review_fact and review_fact.reviewed_head_sha,
    dedup_key = M._dedup_key({
      "merge-ready",
      tostring(issue_proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact and review_fact.review_dedup_key or "review"),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_intake_candidate_payload(repo, issue_number, updated_at)
  local proposal_id = M.proposal_id(repo, issue_number)
  local source_ref = {
    kind = "external",
    ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
  }
  return {
    schema = "github-devloop.intake-candidate.v1",
    repo = repo,
    issue_number = issue_number,
    proposal_id = proposal_id,
    dedup_key = M.intake_dedup_key(proposal_id, updated_at),
    source_ref = source_ref,
  }
end

function M.issue_fetch_instruction(repo, issue_number)
  return "gh issue view " .. M._shell_single_quote(issue_number)
    .. " --repo " .. M._shell_single_quote(repo)
    .. " --json title,body,comments,labels,state"
end

local function pr_review_fetch_instruction(M, repo, pr_number, head_sha, issue_number)
  local lines = {
    "gh pr view " .. M._shell_single_quote(pr_number)
      .. " --repo " .. M._shell_single_quote(repo)
      .. " --json headRefOid,headRefName,baseRefName,state",
    "Confirm headRefOid equals reviewed head " .. tostring(head_sha) .. " before judging.",
    "gh pr diff " .. M._shell_single_quote(pr_number)
      .. " --repo " .. M._shell_single_quote(repo),
  }
  if issue_number ~= nil then
    table.insert(lines, "gh issue view " .. M._shell_single_quote(issue_number)
      .. " --repo " .. M._shell_single_quote(repo)
      .. " --json title,body,comments,labels,state")
  end
  return table.concat(lines, "\n")
end

function M.build_proposal(issue)
  local proposal_id = M.proposal_id(issue.repo, issue.number)
  local title = tostring(issue.title or "")
  if #title > M._max_title_len then
    title = title:sub(1, M._max_title_len)
  end
  local body = "Judge the current GitHub issue from the full source content."
    .. "\nIssue: " .. tostring(issue.repo) .. "#" .. tostring(issue.number)

  return {
    schema = "consensus.proposal.v1",
    verdict_mode = "converge",
    proposal_id = proposal_id,
    title = title,
    body = body,
    content_fetch = M.issue_fetch_instruction(issue.repo, issue.number),
    dedup_key = M.proposal_dedup_key(proposal_id, issue.updated_at),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_board_proposal(issue, tick)
  return M.append_board_digest_to_proposal(M.build_proposal(issue), issue.repo, tick)
end

-- Thread the meta-judge's narrowing onto a re-raised next-round proposal so the next
-- angles converge instead of blindly re-judging the same question. The next round sees
-- ONLY the bounded convergence_question + prior-round digests (verdict + short reply),
-- never prior peer full text, preserving angle peer-invisibility. The `/loop/N` dedup
-- shape stays unchanged so the existing round parsing + budget endpoint still work.
local function apply_converge_fields(proposal, n, converge)
  proposal.round = n
  if type(converge) ~= "table" then
    return proposal
  end
  if converge.narrowed_question ~= nil and converge.narrowed_question ~= "" then
    proposal.convergence_question = converge.narrowed_question
  end
  if type(converge.angle_digests) == "table" then
    proposal.prior_round_digests = converge.angle_digests
  end
  return proposal
end

function M.build_loop_proposal(repo, issue_number, current, source_ref, n, converge)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = source_ref,
  }
  local proposal = M.build_proposal(issue)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return apply_converge_fields(proposal, n, converge)
end

function M.build_board_loop_proposal(repo, issue_number, current, source_ref, n, converge, tick)
  return M.append_board_digest_to_proposal(M.build_loop_proposal(repo, issue_number, current, source_ref, n, converge), repo, tick)
end

function M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref)
  local review_id = M.pr_review_proposal_id(repo, pr_number, version, head_sha)
  local title = "Review PR #" .. tostring(pr_number)
  if issue_number ~= nil then
    title = title .. " for issue #" .. tostring(issue_number)
  end
  if type(current_issue) == "table" and tostring(current_issue.title or "") ~= "" then
    title = "Review PR #" .. tostring(pr_number) .. ": " .. tostring(current_issue.title)
  end
  if #title > M._max_title_len then
    title = title:sub(1, M._max_title_len)
  end

  local issue_title = type(current_issue) == "table" and tostring(current_issue.title or "") or ""
  if #issue_title > M._max_title_len then
    issue_title = issue_title:sub(1, M._max_title_len)
  end
  issue_title = M.neutralize_untrusted_prompt_text(M._neutralize_fkst_markers(issue_title))
  local body = "Review the PR diff and decide whether it should advance to merge-ready."
    .. "\nEntity proposal: " .. tostring(issue_number ~= nil and M.proposal_id(repo, issue_number) or M.pr_proposal_id(repo, pr_number))
    .. "\nReviewed PR head: " .. tostring(head_sha)
    .. "\nIssue title: " .. issue_title
    .. "\nFetch the current PR diff and backing issue content before judging."
  if #body > M._max_body_len then
    error("github-devloop: PR review proposal exceeds bounded body")
  end

  return {
    schema = "consensus.proposal.v1",
    verdict_mode = "gate",
    proposal_id = review_id,
    title = M.neutralize_untrusted_prompt_text(title),
    body = body,
    content_fetch = pr_review_fetch_instruction(M, repo, pr_number, head_sha, issue_number),
    dedup_key = M._dedup_key({
      review_id,
      "review",
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_board_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, tick)
  return M.append_board_digest_to_proposal(M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref), repo, tick)
end

function M.build_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge)
  local proposal = M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return apply_converge_fields(proposal, n, converge)
end

function M.build_board_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge, tick)
  return M.append_board_digest_to_proposal(M.build_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, source_ref, n, converge), repo, tick)
end
end

return S
