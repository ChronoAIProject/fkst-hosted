local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local S = {}

function S.install(M)
  local function label_colors_for(add_labels)
    local colors = {}
    local has_color = false
    for _, label in ipairs(add_labels or {}) do
      local color = M._label_colors and M._label_colors[tostring(label)]
      if color ~= nil then
        colors[tostring(label)] = color
        has_color = true
      end
    end
    return has_color and colors or nil
  end

  local function state_marker_guard(proposal_id, state, version)
    return {
      namespace = "github-devloop",
      marker = "state",
      version = "v1",
      match = {
        proposal = tostring(proposal_id),
      },
      expected = {
        state = tostring(state),
        version = tostring(version),
      },
      order_by = {
        "marker_order_key",
        "version_order_key",
        "stage_rank",
      },
    }
  end

function M.build_pr_state_label_request(repo, issue_number, pr_number, proposal_id, to_state, version, dedup_key_value, source_ref, current_labels)
  local add_labels, remove_labels
  if current_labels ~= nil then
    add_labels, remove_labels = M.state_label_reconcile_changes(current_labels, to_state)
  else
    add_labels, remove_labels = M.state_label_changes(to_state)
  end
  return m_claims.attach_issue_claim({
    schema = "github-proxy.label.v1",
    repo = repo,
    target_kind = "pr",
    target_number = pr_number,
    pr_number = pr_number,
    issue_number = issue_number,
    require_marker_guard = true,
    expected_proposal_id = proposal_id,
    expected_state = to_state,
    expected_version = version,
    marker_guard = state_marker_guard(proposal_id, to_state, version),
    add_labels = add_labels,
    remove_labels = remove_labels,
    label_colors = label_colors_for(add_labels),
    dedup_key = dedup_key_value,
    source_ref = base_ids.normalize_source_ref(source_ref),
  }, issue_number ~= nil and entity_lib.issue_source_ref(repo, issue_number) or nil)
end

function M.build_reconcile_pr_state_label_request(repo, issue_number, pr_number, proposal_id, state, version, source_ref, current_labels)
  return M.build_pr_state_label_request(
    repo,
    issue_number,
    pr_number,
    proposal_id,
    state,
    version,
    base_ids.dedup_key({
      "reconcile",
      "pr-label",
      tostring(proposal_id),
      tostring(state),
      tostring(version or "unversioned"),
      tostring(pr_number),
    }),
    source_ref,
    current_labels
  )
end

function M.pr_state_label_request_guard_visible(comments, label_request)
  if type(label_request) ~= "table" then
    return false
  end
  if label_request.target_kind ~= "pr" then
    return true
  end
  if label_request.expected_proposal_id == nil
    or label_request.expected_state == nil
    or label_request.expected_version == nil then
    return false
  end
  return M.has_state_marker(
    comments,
    label_request.expected_proposal_id,
    label_request.expected_state,
    label_request.expected_version
  )
end

function M.build_pr_reviewing_label_request(repo, issue_number, origin, pr_number, source_ref)
  return M.build_pr_state_label_request(
    repo,
    issue_number,
    pr_number,
    origin.proposal_id,
    "reviewing",
    origin.impl_version,
    base_ids.dedup_key({
      "observe-pr",
      "pr-label",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref
  )
end

end

return S
