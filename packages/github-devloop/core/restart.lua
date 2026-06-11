local S = {}

function S.install(M)

local audit = {
  {
    state = "thinking",
    marker_facts = "state:v1 thinking plus optional converge-round:v1",
    kickoff = "consensus.proposal",
    replay = "Initial thinking reuses the state version as proposal dedup; convergence replays the next /loop/N from the latest complete converge-round marker.",
  },
  {
    state = "ready",
    marker_facts = "state:v1 ready",
    kickoff = "devloop_ready",
    replay = "Raise ready/<version> after dependency gate re-derives satisfied blockers.",
  },
  {
    state = "implementing",
    marker_facts = "state:v1 implementing plus implementing:v1",
    kickoff = "github-proxy.github_entity_changed",
    replay = "Branch poll re-derives PR open or impl-failed from branch/worktree facts.",
  },
  {
    state = "pr-open",
    marker_facts = "state:v1 pr-open plus pr-link:v1",
    kickoff = "devloop_reviewing",
    replay = "Observe re-fetches the linked PR and raises review for the linked PR head.",
  },
  {
    state = "reviewing",
    marker_facts = "state:v1 reviewing plus PR head facts",
    kickoff = "devloop_reviewing",
    replay = "PR observe re-derives review kickoff from current PR head and issue version.",
  },
  {
    state = "review-converge",
    marker_facts = "state:v1 reviewing plus review-converge-round:v1",
    kickoff = "consensus.proposal",
    replay = "Review loop replays the next /review-loop/N from the latest complete review-converge marker.",
  },
  {
    state = "fixing",
    marker_facts = "state:v1 fixing plus review-result/review-meta/merge-gate feedback, or current PR head for deterministic renormalization",
    kickoff = "devloop_fixing or devloop_reviewing",
    replay = "Observe re-raises fix when a trusted feedback fact is parseable; otherwise it re-enters reviewing for the current head.",
  },
  {
    state = "review-meta",
    marker_facts = "state:v1 review-meta plus review proposal encoded in version/dedup",
    kickoff = "devloop_review_meta",
    replay = "Observe re-raises review-meta using the review proposal, PR number, issue version, and original dedup.",
  },
  {
    state = "merge-ready",
    marker_facts = "state:v1 merge-ready plus merge-ready:v1",
    kickoff = "devloop_merge_ready",
    replay = "PR observe or merge retry re-derives merge-ready from head-bound approval facts.",
  },
  {
    state = "merging",
    marker_facts = "state:v1 merging plus merging:v1",
    kickoff = "devloop_merge_ready",
    replay = "Merge retry re-derives completion or repair from PR mergeability and head facts.",
  },
}

local audit_by_state = {}
for _, row in ipairs(audit) do
  audit_by_state[row.state] = row
end

function M.restart_completeness_audit()
  local rows = {}
  for _, row in ipairs(audit) do
    table.insert(rows, {
      state = row.state,
      marker_facts = row.marker_facts,
      kickoff = row.kickoff,
      replay = row.replay,
    })
  end
  return rows
end

function M.restart_completeness_audit_for_state(state)
  return audit_by_state[state]
end

function M.latest_complete_converge_round(comments, proposal_id, base_version, source_ref)
  local sr_digest = M.source_ref_digest(source_ref)
  local latest = nil
  local facts = base_version ~= nil
    and M.converge_round_facts(comments, proposal_id, base_version, sr_digest)
    or M.converge_round_facts_for_source(comments, proposal_id, sr_digest)
  for _, fact in ipairs(facts) do
    if fact.narrowed_question ~= nil
      and fact.narrowed_question ~= ""
      and type(fact.angle_digests) == "table"
      and #fact.angle_digests > 0
      and (latest == nil or fact.round > latest.round) then
      latest = fact
    end
  end
  return latest
end

function M.review_meta_replay_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-converge%-round:v1.-%-%->"
  local best = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('issue_proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local review_proposal = marker:match('proposal="([^"]+)"')
      local consensus_dedup = marker:match('dedup="([^"]*)"')
      local round = tonumber(marker:match('round="(%d+)"'))
      local _, pr_number, review_version = M.parse_pr_review_proposal_id(review_proposal)
      local repo = M.parse_proposal_id(issue_proposal_id)
      if marker_issue == tostring(issue_proposal_id)
        and marker_version == tostring(issue_version)
        and review_version == M.safe_version_segment(issue_version)
        and repo ~= nil
        and M._is_positive_pr_number(pr_number)
        and M._is_path_safe_key(review_proposal, M._max_key_len)
        and M._is_bounded_string(consensus_dedup, M._max_dedup_len)
        and (best == nil or (round or 0) > (best.n or 0)) then
        best = {
          proposal_id = review_proposal,
          dedup_key = consensus_dedup,
          source_ref = M.pr_source_ref(repo, pr_number),
          pr_number = tonumber(pr_number),
          n = (round or 0) + 1,
        }
      end
    end
  end
  return best
end

function M.fixing_replay_feedback_fact(comments, issue_proposal_id, issue_version)
  local reject_fact = M.review_reject_fact(comments, issue_proposal_id, issue_version)
  if reject_fact ~= nil then
    return reject_fact
  end
  local meta_fix_fact = M.review_meta_fix_fact(comments, issue_proposal_id, issue_version)
  if meta_fix_fact ~= nil then
    return meta_fix_fact
  end
  return M.merge_gate_fix_fact(comments, issue_proposal_id, issue_version)
end

function M.fixing_version_matches_link(issue_version, link_version)
  local current = tostring(issue_version or "")
  local linked = tostring(link_version or "")
  return current == linked or M._strip_latest_fix_version_suffix(current) == linked
end

end

return S
