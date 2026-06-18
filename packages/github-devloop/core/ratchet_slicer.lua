local S = {}

function S.install(M)
local strings = require("std.strings")
local slice_schema = "fkst.ratchet-slice.v1"
local supported_ratchets = {
  "saga-handler",
  "code-dedup",
}
local max_site_lines = 10

local function repo_root()
  local result = exec_argv({ argv = { "pwd" }, timeout = 5 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop: ratchet slicer repo-root probe failed")
  end
  local root = tostring(result.stdout or ""):gsub("%s+$", "")
  if root == "" then
    error("github-devloop: ratchet slicer repo-root probe returned empty path")
  end
  return root
end

local function load_slice_doc(ratchet)
  local result = exec_argv({
    argv = {
      "python3",
      "-B",
      "scripts/ratchet_migration_slicer.py",
      tostring(ratchet),
      "--repo-root",
      repo_root(),
      "--slice-size",
      "3",
      "--json",
    },
    timeout = 30,
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop: ratchet slicer failed: " .. tostring(result and result.stderr or ""))
  end
  local ok, decoded = pcall(json.decode, result.stdout or "{}")
  if not ok or type(decoded) ~= "table" or decoded.schema ~= slice_schema then
    error("github-devloop: ratchet slicer returned invalid schema")
  end
  return decoded
end

local function bounded_doc_string(doc, field, limit)
  local value = doc[field]
  if type(value) ~= "string" or value == "" or #value > limit then
    error("github-devloop: ratchet slicer invalid " .. tostring(field))
  end
  return value
end

local function validate_doc(doc, ratchet)
  local selected = tonumber(doc.selected_count)
  local current = tonumber(doc.current_count)
  if doc.ratchet ~= ratchet
    or type(doc.parent_issue) ~= "number"
    or doc.parent_issue < 1
    or selected == nil
    or current == nil
    or selected < 0
    or selected > 3
    or current < selected
    or doc.target_count ~= 0
    or type(doc.sites) ~= "table" then
    error("github-devloop: ratchet slicer document failed validation")
  end
  bounded_doc_string(doc, "allowlist_path", 200)
  bounded_doc_string(doc, "sites_fingerprint", 80)
  bounded_doc_string(doc, "dedup_key", 200)
  if doc.dedup_key ~= tostring(ratchet) .. "/slice/" .. tostring(doc.sites_fingerprint) then
    error("github-devloop: ratchet slicer dedup_key mismatch")
  end
  if #doc.sites ~= selected then
    error("github-devloop: ratchet slicer selected_count mismatch")
  end
  return selected, current
end

local function code(value)
  return "`" .. tostring(value or ""):gsub("`", "\\`") .. "`"
end

local function site_line(site)
  if type(site) ~= "table" then
    error("github-devloop: ratchet slicer invalid site")
  end
  local site_ref = tostring(site.site_ref or "")
  local detail = tostring(site.detail or "")
  if site_ref == "" or #site_ref > 400 or detail == "" or #detail > 300 then
    error("github-devloop: ratchet slicer invalid site fields")
  end
  return "- " .. code(site_ref) .. " (" .. code(detail) .. ")"
end

local function render_slice_body(doc)
  local lines = {
    "# " .. M.neutralize_untrusted_comment_text(doc.title or "ratchet migration slice"),
    "",
    "Machine-readable ratchet slice:",
    "- schema: " .. code(slice_schema),
    "- ratchet: " .. code(doc.ratchet),
    "- parent_issue: #" .. tostring(doc.parent_issue),
    "- migration_kind: " .. code(doc.migration_kind or "allowlist"),
    "- allowlist_path: " .. code(doc.allowlist_path),
    "- current_count: " .. tostring(doc.current_count),
    "- target_count: " .. tostring(doc.target_count),
    "- slice_size: " .. tostring(doc.slice_size),
    "- selected_count: " .. tostring(doc.selected_count),
    "- sites_fingerprint: " .. code(doc.sites_fingerprint),
    "- dedup_key: " .. code(doc.dedup_key),
    "",
    "Reference shape:",
    M.neutralize_untrusted_comment_text(doc.reference_shape or ""),
    "",
    "Exact sites:",
  }
  if #doc.sites == 0 then
    table.insert(lines, "- none")
  else
    for index, site in ipairs(doc.sites) do
      if index > max_site_lines then
        error("github-devloop: ratchet slicer emitted too many sites")
      end
      table.insert(lines, site_line(site))
    end
  end
  table.insert(lines, "")
  table.insert(lines, "Acceptance:")
  table.insert(lines, "- Migrate only the exact sites listed above.")
  table.insert(lines, "- Remove only those migrated entries from " .. code(doc.allowlist_path) .. ".")
  table.insert(lines, "- The allowlist count decreases by exactly " .. tostring(doc.selected_count) .. ".")
  table.insert(lines, "- Behavior is preserved.")
  table.insert(lines, "- `scripts/run.sh test` exits 0.")
  table.insert(lines, "- No broad cleanup, opportunistic refactors, or unrelated migration work.")
  local body = table.concat(lines, "\n")
  if #body > M._max_body_len then
    body = M.truncate_utf8(body, M._max_body_len)
  end
  return body
end

function M.ratchet_slice_marker(ratchet, sites_fingerprint)
  local safe_ratchet = strings.sanitize_key(ratchet or "", 120):gsub("/", "-")
  local safe_fingerprint = strings.sanitize_key(sites_fingerprint or "", 120):gsub("/", "-")
  return '<!-- fkst:github-devloop:ratchet-slice:v1 ratchet="' .. safe_ratchet
    .. '" sites_fingerprint="' .. safe_fingerprint
    .. '" -->'
end

function M.ratchet_slice_search_query(ratchet)
  return "fkst:github-devloop:ratchet-slice:v1 ratchet=\"" .. tostring(ratchet) .. "\""
end

function M.parse_ratchet_slice_issue_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local issues = {}
  if type(decoded) ~= "table" then
    return issues
  end
  for _, issue in ipairs(decoded) do
    if type(issue) == "table" and tonumber(issue.number) ~= nil then
      table.insert(issues, {
        number = tonumber(issue.number),
        state = tostring(issue.state or ""),
        body = tostring(issue.body or ""),
        title = tostring(issue.title or ""),
      })
    end
  end
  return issues
end

function M.has_inflight_ratchet_slice(issues, ratchet)
  local marker_prefix = '<!-- fkst:github-devloop:ratchet-slice:v1 ratchet="' .. tostring(ratchet) .. '"'
  for _, issue in ipairs(issues or {}) do
    if tostring(issue.state or ""):upper() == "OPEN"
      and tostring(issue.body or ""):find(marker_prefix, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.build_ratchet_slice_issue_create_request(repo, doc)
  local title = tostring(doc.title or "ratchet allowlist migration slice")
  if #title > M._max_title_len then
    title = M.truncate_utf8(title, M._max_title_len)
  end
  local body = render_slice_body(doc)
    .. "\n\n" .. M.ratchet_slice_marker(doc.ratchet, doc.sites_fingerprint)
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = tostring(doc.dedup_key),
    parent_comment_target = {
      repo = tostring(repo),
      issue_number = tonumber(doc.parent_issue),
    },
    post_create_blocked_by = {
      blocked_issue_number = tonumber(doc.parent_issue),
      dedup_key = tostring(doc.dedup_key) .. "/blocked-by",
      external_effect_saga = "ratchet-slicer",
      external_effect_step = "block-parent",
    },
    source_ref = M.issue_source_ref(repo, doc.parent_issue),
  }
end

local function search_existing_slices(repo, ratchet)
  local result = M.gh_issue_list_ratchet_slices(repo, ratchet, 30)
  if result.exit_code ~= 0 then
    error("github-devloop: ratchet slice issue search failed: " .. tostring(result.stderr))
  end
  return M.parse_ratchet_slice_issue_list(result.stdout)
end

local function close_parent_if_empty(repo, doc, ratchet)
  local mode = M.write_mode()
  if mode ~= "real" then
    M.log_line("info", "ratchet_slicer", "ratchet/" .. tostring(ratchet), "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(repo),
      "parent_issue=" .. tostring(doc.parent_issue),
      "reason=empty-ratchet-parent-close-requires-FKST_GITHUB_WRITE=1",
    })
    return
  end
  local result = M.gh_issue_close(repo, doc.parent_issue, 30)
  if result.exit_code ~= 0 then
    error("github-devloop: ratchet parent close failed: " .. tostring(result.stderr))
  end
end

function M.reconcile_ratchet_slices(repo)
  local raised_count = 0
  local closed_count = 0
  for _, ratchet in ipairs(supported_ratchets) do
    with_lock("github-devloop/ratchet-slicer/" .. strings.sanitize_key(ratchet, 120), function()
      local doc = load_slice_doc(ratchet)
      local selected, current = validate_doc(doc, ratchet)
      local existing = search_existing_slices(repo, ratchet)
      if current == 0 then
        close_parent_if_empty(repo, doc, ratchet)
        closed_count = closed_count + 1
        return
      end
      if selected == 0 or M.has_inflight_ratchet_slice(existing, ratchet) then
        return
      end
      local request = M.build_ratchet_slice_issue_create_request(repo, doc)
      M.log_raise("ratchet_slicer", tostring(ratchet), "github-proxy.github_issue_create_request", request)
      raised_count = raised_count + 1
    end)
  end
  return {
    raised_count = raised_count,
    closed_count = closed_count,
  }
end

end

return S
