local base_ids = require("devloop.base_ids")
local strings = require("contract.strings")
local C = {}
local forge_validators = require("devloop.forge_validators")
local env = require("workflow.env")

local seconds_per_minute = 60
local liveness_poll_interval_minutes = 5

local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_AUTHORIZED_LOGINS = true,
  FKST_GITHUB_CLAIM_MODE = true,
  FKST_GITHUB_REPO = true,
  FKST_SESSION_CREATOR = true,
  FKST_SESSION_WORK_LABEL = true,
  FKST_SESSION_WORK_LABEL_MAP_JSON = true,
  FKST_WORK_LABEL_NAMESPACE = true,
  FKST_GITHUB_WRITE = true,
  FKST_DEVLOOP_UPSTREAM_BRANCH = true,
  FKST_DEVLOOP_INTEGRATION_BRANCH = true,
  FKST_DEVLOOP_FORK_GRACE_HOURS = true,
  FKST_DEVLOOP_MAX_INFLIGHT = true,
  FKST_DEVLOOP_MANAGED_SIBLING_REPOS = true,
  FKST_DEVLOOP_MANAGED_BOT_LOGINS = true,
  FKST_DEVLOOP_ROLLUP_MERGE = true,
  FKST_DEVLOOP_ROLLUP_AUTOFIX = true,
  FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES = true,
  FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES = true,
  FKST_DEVLOOP_RELEASE_NOTES_FALLBACK = true,
  FKST_DEVLOOP_CONFLICT_LOG_CMD = true,
  FKST_DEVLOOP_BOARD_CMD = true,
  FKST_DEVLOOP_TEST_COMMAND = true,
  FKST_DEVLOOP_DELIVERY_GRANTS = true,
  FKST_OUTPUT_LANG = true,
  FKST_DEBUG_STAMP = true,
}

local allowed_presence_env = {
  GH_TOKEN = true,
  GITHUB_TOKEN = true,
  FKST_GITHUB_READ_TOKEN = true,
  FKST_GITHUB_WRITE_TOKEN = true,
  FKST_GITHUB_MERGE_TOKEN = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-devloop: env name is not allowed")
  end
  return 'printf %s "$' .. name .. '"'
end

local function env_present_command(name)
  if not allowed_presence_env[name] then
    error("github-devloop: env name is not allowed")
  end
  return 'if [ -n "${' .. name .. ':-}" ]; then printf present; fi'
end

local read_env = env.read_env(read_env_command)

function C.read_env_command(name)
  return read_env_command(name)
end

function C.read_env(name, exec)
  return read_env(name, exec)
end

function C.env_present_command(name)
  return env_present_command(name)
end

function C.env_present(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    return false
  end
  local ok, out = pcall(run, env_present_command(name))
  return ok and type(out) == "table" and out.exit_code == 0 and out.stdout ~= ""
end

function C.write_mode(exec)
  return C.read_env("FKST_GITHUB_WRITE", exec) == "1" and "real" or "dry-run"
end

-- Claim mode is opt-in and additive: the default (unset/empty/unknown) is
-- "assignee", which is byte-for-byte today's behavior. "label" opts into
-- holding ownership via the fkst-dev:claimed label, which a GitHub App can set
-- even though an App cannot be an issue assignee.
function C.claim_mode(exec)
  local raw = C.read_env("FKST_GITHUB_CLAIM_MODE", exec)
  raw = strings.trim(raw or "")
  if raw == "label" then
    return "label"
  end
  return "assignee"
end

function C.parse_session_work_labels(value)
  local labels = {}
  local seen = {}
  for raw in tostring(value or ""):gmatch("[^,]+") do
    local label = strings.trim(raw)
    if label ~= "" and not seen[label] then
      seen[label] = true
      table.insert(labels, label)
    end
  end
  return labels
end

local function raw_session_work_labels(exec)
  return C.parse_session_work_labels(C.read_env("FKST_SESSION_WORK_LABEL", exec))
end

local github_label_name_max_chars = 50

local function valid_utf8(value)
  if type(utf8) ~= "table" or type(utf8.len) ~= "function" then
    return false, nil
  end
  local ok, length = pcall(utf8.len, value)
  return ok and length ~= nil, length
end

local function contains_control_character(value)
  for _, codepoint in utf8.codes(value) do
    if codepoint <= 31 or (codepoint >= 127 and codepoint <= 159) then
      return true
    end
  end
  return false
end

local function validate_work_label_map_entry(kind, value)
  if type(value) ~= "string" or value == "" then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: " .. kind .. " must be a non-empty string")
  end
  if strings.trim(value) ~= value then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: " .. kind .. " cannot have surrounding whitespace")
  end
  if value:find(",", 1, true) ~= nil then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: " .. kind .. " cannot contain a comma")
  end
  local is_valid_utf8, length = valid_utf8(value)
  if not is_valid_utf8 then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: " .. kind .. " must be valid UTF-8")
  end
  if contains_control_character(value) then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: " .. kind .. " cannot contain control characters")
  end
  return length
end

function C.parse_work_label_map_json(raw)
  local source = strings.trim(tostring(raw or ""))
  if source == "" then
    return {}
  end
  if source:sub(1, 1) ~= "{" or source:sub(-1) ~= "}" then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: expected a JSON object")
  end

  local ok, decoded = pcall(json.decode, source)
  if not ok or type(decoded) ~= "table" then
    error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: malformed JSON object")
  end

  local map = {}
  local owner_by_effective_label = {}
  for logical, effective in pairs(decoded) do
    validate_work_label_map_entry("logical label", logical)
    local effective_length = validate_work_label_map_entry("effective label", effective)
    if effective_length > github_label_name_max_chars then
      error(
        "github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: effective label exceeds GitHub's 50-character limit"
      )
    end
    local folded = effective:lower()
    local owner = owner_by_effective_label[folded]
    if owner ~= nil and owner ~= logical then
      error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: effective labels collide case-insensitively")
    end
    owner_by_effective_label[folded] = logical
    map[logical] = effective
  end
  return map
end

function C.parse_work_label_namespace(raw)
  local namespace = strings.trim(tostring(raw or ""))
  if namespace == "" then
    return nil
  end
  if #namespace > github_label_name_max_chars - 2 then
    error("github-devloop: invalid FKST_WORK_LABEL_NAMESPACE: must be at most 48 characters")
  end
  if namespace:find("^[a-z0-9-]+$") == nil
    or namespace:sub(1, 1) == "-"
    or namespace:sub(-1) == "-"
    or namespace:find("--", 1, true) ~= nil then
    error(
      "github-devloop: invalid FKST_WORK_LABEL_NAMESPACE: must be a lowercase ASCII slug with single interior hyphens"
    )
  end
  return namespace
end

function C.work_label_namespace(exec)
  return C.parse_work_label_namespace(C.read_env("FKST_WORK_LABEL_NAMESPACE", exec))
end

function C.work_label_family_isolation_active(exec)
  local explicit_map = strings.trim(tostring(C.read_env("FKST_SESSION_WORK_LABEL_MAP_JSON", exec) or ""))
  if explicit_map ~= "" then
    return true
  end
  return C.work_label_namespace(exec) ~= nil
end

local function validate_namespaced_mapping(logical, effective, namespace)
  validate_work_label_map_entry("logical label", logical)
  local effective_length = validate_work_label_map_entry("effective label", effective)
  local expected = logical .. "-" .. namespace
  if effective ~= expected then
    error(
      "github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: effective label does not match FKST_WORK_LABEL_NAMESPACE"
    )
  end
  if effective_length > github_label_name_max_chars then
    error(
      "github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: effective label exceeds GitHub's 50-character limit"
    )
  end
end

local function derive_namespaced_mapping(configured_label, namespace)
  local label = tostring(configured_label or "")
  validate_work_label_map_entry("session work label", label)
  local namespace_suffix = "-" .. namespace
  if #label > #namespace_suffix and label:sub(-#namespace_suffix) == namespace_suffix then
    return label:sub(1, #label - #namespace_suffix), label
  end
  return label, label .. namespace_suffix
end

-- The explicit map is useful for diagnostics and removes any ambiguity about
-- logical identities. The namespace remains the authority: when the map is
-- absent it is reconstructed from FKST_SESSION_WORK_LABEL, and when both are
-- present every entry must agree exactly. This prevents a malformed runtime
-- injection from silently falling back to a generic or foreign provider family.
function C.work_label_map(exec, configured_labels)
  local map = C.parse_work_label_map_json(C.read_env("FKST_SESSION_WORK_LABEL_MAP_JSON", exec))
  local namespace = C.work_label_namespace(exec)
  if namespace == nil then
    return map
  end

  local owner_by_effective_label = {}
  for logical, effective in pairs(map) do
    validate_namespaced_mapping(logical, effective, namespace)
    owner_by_effective_label[effective:lower()] = logical
  end

  local labels = configured_labels or raw_session_work_labels(exec)
  for _, configured_label in ipairs(labels) do
    local logical, effective = derive_namespaced_mapping(configured_label, namespace)
    validate_namespaced_mapping(logical, effective, namespace)
    if map[logical] ~= nil and map[logical] ~= effective then
      error(
        "github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: session work label conflicts with explicit mapping"
      )
    end
    local folded = effective:lower()
    local owner = owner_by_effective_label[folded]
    if owner ~= nil and owner ~= logical then
      error("github-devloop: invalid FKST_SESSION_WORK_LABEL_MAP_JSON: effective labels collide case-insensitively")
    end
    owner_by_effective_label[folded] = logical
    map[logical] = effective
  end
  return map
end

local function utf8_prefix(value, character_count)
  if character_count <= 0 then
    return ""
  end
  local next_byte = utf8.offset(value, character_count + 1)
  if next_byte == nil then
    return value
  end
  return value:sub(1, next_byte - 1)
end

local function compact_derived_label(root, suffix, expanded)
  local _, expanded_length = valid_utf8(expanded)
  if expanded_length <= github_label_name_max_chars then
    return expanded
  end

  -- Preserve the complete effective work-label root. Only an overlong derived
  -- suffix is compacted, and its checksum keeps the result deterministic. This
  -- matters for provider namespaces whose readable root fits GitHub while a long
  -- lifecycle suffix (for example blocked-on-dependency) does not.
  local _, root_length = valid_utf8(root)
  local checksum = strings.decimal_checksum(expanded)
  local readable_suffix_chars = github_label_name_max_chars
    - root_length
    - 1 -- colon
    - 1 -- checksum separator
    - #checksum
  if readable_suffix_chars < 1 then
    error(
      "github-devloop: derived effective label exceeds GitHub's 50-character limit and its work-label root leaves no room for deterministic compaction"
    )
  end
  return root .. ":" .. utf8_prefix(suffix, readable_suffix_chars) .. "-" .. checksum
end

local function sorted_logical_roots(map)
  local roots = {}
  for logical in pairs(type(map) == "table" and map or {}) do
    roots[#roots + 1] = logical
  end
  table.sort(roots, function(left, right)
    local left_length = select(2, valid_utf8(left)) or #left
    local right_length = select(2, valid_utf8(right)) or #right
    if left_length ~= right_length then
      return left_length > right_length
    end
    return left < right
  end)
  return roots
end

-- Translate one logical GitHub label through the declared work-label FAMILY.
-- A root maps exactly, and root:suffix maps to effective-root:suffix. Unrelated
-- labels are untouched. Longest-root matching makes overlapping declarations
-- deterministic (for example `fkst` and `fkst-dev`).
function C.apply_work_label_map_to_label(logical, map)
  local label = tostring(logical or "")
  for _, root in ipairs(sorted_logical_roots(map)) do
    if label == root then
      return map[root]
    end
    local prefix = root .. ":"
    if label:sub(1, #prefix) == prefix then
      local suffix = label:sub(#prefix + 1)
      local effective_root = map[root]
      return compact_derived_label(effective_root, suffix, effective_root .. ":" .. suffix)
    end
  end
  return label
end

function C.effective_work_label(logical, exec)
  return C.apply_work_label_map_to_label(logical, C.work_label_map(exec))
end

function C.apply_work_label_map(labels, map)
  local effective = {}
  local seen = {}
  for _, logical in ipairs(labels or {}) do
    local label = tostring(logical or "")
    local translated = C.apply_work_label_map_to_label(label, map)
    if translated ~= "" and not seen[translated] then
      seen[translated] = true
      effective[#effective + 1] = translated
    end
  end
  return effective
end

function C.effective_work_labels(labels, exec)
  return C.apply_work_label_map(labels, C.work_label_map(exec))
end

function C.session_work_labels(exec)
  local labels = raw_session_work_labels(exec)
  return C.apply_work_label_map(labels, C.work_label_map(exec, labels))
end

function C.effective_label_colors(label_colors, map)
  if type(label_colors) ~= "table" then
    return label_colors
  end
  local effective = {}
  for logical, color in pairs(label_colors) do
    effective[C.apply_work_label_map_to_label(logical, map)] = color
  end
  return effective
end

-- Incoming GitHub labels remain in their effective form. Package lifecycle
-- code supplies logical expected labels, so compare only against the one
-- effective identity for this session. Deliberately do not accept both forms:
-- doing so would let a namespaced cloud session adopt a local session's issue.
function C.label_matches_effective(actual, logical, exec, map)
  local effective_map = map or C.work_label_map(exec)
  local expected = C.apply_work_label_map_to_label(logical, effective_map)
  return tostring(actual or "") == expected
end

function C.has_effective_label(labels, logical, exec, map)
  if type(labels) ~= "table" then
    return false
  end
  local effective_map = map or C.work_label_map(exec)
  local expected = C.apply_work_label_map_to_label(logical, effective_map)
  for _, label in ipairs(labels) do
    local name = type(label) == "table" and label.name or label
    if tostring(name or "") == expected then
      return true
    end
  end
  return false
end

-- Set by fkst-hosted for creator-routed sessions. Nil preserves the legacy
-- single-operator contract used by standalone package deployments.
function C.session_creator(exec)
  local creator = strings.trim(C.read_env("FKST_SESSION_CREATOR", exec) or "")
  if creator == "" then
    return nil
  end
  return creator
end

function C.matches_session_work_label(issue_labels, exec, configured_labels)
  local configured = configured_labels or C.session_work_labels(exec)
  if #configured == 0 then
    return false, "FKST_SESSION_WORK_LABEL is empty"
  end

  local allowed = {}
  for _, label in ipairs(configured) do
    allowed[label] = true
  end
  for _, label in ipairs(issue_labels or {}) do
    local name = type(label) == "table" and label.name or label
    if allowed[tostring(name or "")] then
      return true, nil
    end
  end
  return false, "issue has no exact configured session work label"
end

-- Rollup auto-fix is opt-in and additive: default (unset/anything-but-"1") is
-- off, which is byte-for-byte today's behavior (the rollup-health watchdog only
-- files a passive issue). When "1", the watchdog issue is created already
-- fkst-dev:enabled + fkst-class:expedite so the loop claims and fixes the red
-- rollup ahead of new issues (expedite class + inflight cap = priority).
function C.rollup_autofix_enabled(exec)
  return strings.trim(C.read_env("FKST_DEVLOOP_ROLLUP_AUTOFIX", exec) or "") == "1"
end

function C.rollup_runtime_soak_minutes(exec)
  local raw = strings.trim(C.read_env("FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES", exec) or "")
  if raw == "" then
    return 30
  end
  local parsed = tonumber(raw)
  if parsed == nil or parsed ~= math.floor(parsed) or parsed < 1 or parsed > 1440 then
    error("github-devloop: invalid FKST_DEVLOOP_ROLLUP_RUNTIME_SOAK_MINUTES")
  end
  return parsed
end

function C.max_inflight(exec)
  local value = C.read_env("FKST_DEVLOOP_MAX_INFLIGHT", exec)
  if value == nil then
    return nil
  end
  value = strings.trim(value)
  if value == "" then
    return nil
  end
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) or parsed < 1 or parsed > 100 then
    error("github-devloop: invalid FKST_DEVLOOP_MAX_INFLIGHT")
  end
  return parsed
end

function C.managed_sibling_repos(exec)
  local raw = C.read_env("FKST_DEVLOOP_MANAGED_SIBLING_REPOS", exec)
  local repos = {}
  if raw == nil then
    return repos
  end
  for entry in tostring(raw):gmatch("[^,%s]+") do
    local repo = tostring(entry)
    if base_ids.issue_ref_round_trips(repo, 1) then
      repos[repo] = true
    end
  end
  return repos
end

function C.max_fix_rounds()
  return 12
end

function C.liveness_poll_interval()
  return tostring(liveness_poll_interval_minutes) .. "m"
end

function C.liveness_poll_cadence_seconds()
  return liveness_poll_interval_minutes * seconds_per_minute
end

function C.default_test_command()
  return "scripts/run.sh test"
end

function C.test_command(exec)
  local command = C.read_env("FKST_DEVLOOP_TEST_COMMAND", exec)
  if command == nil then
    return C.default_test_command()
  end
  return command
end

function C.local_iteration_test_command(_exec)
  return "scripts/run.sh test-affected"
end

local function current_checkout_branch(exec)
  local run = exec or exec_argv
  if type(run) ~= "function" then
    error("github-devloop: branch config requires exec_argv")
  end
  local git = require("forge.git").new(run)
  local ok, out = pcall(function()
    return git.current_branch(30)
  end)
  if not ok or type(out) ~= "table" or out.exit_code ~= 0 then
    error("github-devloop: current checkout branch read failed")
  end
  local branch = strings.trim(out.stdout)
  if branch == "HEAD" or not forge_validators.is_git_ref_safe(branch) then
    error("github-devloop: invalid current checkout branch")
  end
  return branch
end

local function validated_branch(name, branch)
  branch = strings.trim(branch)
  if not forge_validators.is_git_ref_safe(branch) then
    error("github-devloop: invalid " .. name)
  end
  return branch
end

function C.branch_config(exec)
  local upstream_env = C.read_env("FKST_DEVLOOP_UPSTREAM_BRANCH", exec)
  local upstream = upstream_env
  if upstream == nil then
    upstream = current_checkout_branch(exec)
  end
  upstream = validated_branch("FKST_DEVLOOP_UPSTREAM_BRANCH", upstream)
  local integration = C.read_env("FKST_DEVLOOP_INTEGRATION_BRANCH", exec)
  if integration == nil then
    integration = upstream
  end
  integration = validated_branch("FKST_DEVLOOP_INTEGRATION_BRANCH", integration)
  return {
    upstream = upstream,
    integration = integration,
  }
end

function C.devloop_config(exec)
  local branches = C.branch_config(exec)
  local rollup_merge = C.read_env("FKST_DEVLOOP_ROLLUP_MERGE", exec) or "auto"
  rollup_merge = strings.trim(rollup_merge)
  if rollup_merge ~= "auto" and rollup_merge ~= "manual" then
    error("github-devloop: invalid FKST_DEVLOOP_ROLLUP_MERGE")
  end
  return {
    repo = C.read_env("FKST_GITHUB_REPO", exec),
    bot_login = C.read_env("FKST_GITHUB_BOT_LOGIN", exec),
    write_mode = C.write_mode(exec),
    upstream_branch = branches.upstream,
    integration_branch = branches.integration,
    rollup_merge = rollup_merge,
    allow_release_notes_fallback = C.read_env("FKST_DEVLOOP_RELEASE_NOTES_FALLBACK", exec) == "1",
  }
end

return C
