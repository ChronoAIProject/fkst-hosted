local core = require("core")
local fixtures = require("tests.production_fixture_helpers")

M = {}

M.spec = {
  consumes = { "context_bundle_probe" },
  produces = { "context_bundle_probe_result" },
}

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function exec_with_env(root, fixtures)
  local state = fixtures or {}
  state.calls = state.calls or {}
  state.issue_outputs = state.issue_outputs or {
    '{"title":"Bundle issue","body":"Full issue body","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
  }
  state.pr_output = state.pr_output or '{"title":"Bundle PR","body":"PR body","headRefName":"devloop-owner-repo-42","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-04T01:02:03Z","comments":[],"labels":[]}\n'
  state.diff_output = state.diff_output or "diff --git a/file.lua b/file.lua\n+return true\n"
  return function(cmd)
    local rendered = type(cmd) == "table" and cmd.cmd or tostring(cmd)
    table.insert(state.calls, rendered)
    if rendered == core.read_runtime_root_cmd() then
      return { stdout = root, stderr = "", exit_code = 0 }
    end
    if rendered:find("gh issue view", 1, true) ~= nil then
      local output = table.remove(state.issue_outputs, 1) or state.last_issue_output or ""
      state.last_issue_output = output
      return { stdout = output, stderr = "", exit_code = 0 }
    end
    if rendered:find("gh pr view", 1, true) ~= nil then
      return { stdout = state.pr_output, stderr = "", exit_code = 0 }
    end
    if rendered:find("gh pr diff", 1, true) ~= nil then
      return { stdout = state.diff_output, stderr = "", exit_code = 0 }
    end
    local with_env = "FKST_RUNTIME_ROOT=" .. shell_single_quote(root) .. " " .. rendered
    local handle = io.popen(with_env .. " 2>&1")
    local stdout = handle:read("*a")
    local ok, _, status = handle:close()
    return {
      stdout = stdout or "",
      stderr = ok and "" or (stdout or ""),
      exit_code = ok and 0 or (status or 1),
    }
  end
end

local function build_args(root, fixtures, extra)
  local fields = extra or {}
  return {
    repo = "owner/repo",
    issue_number = fields.issue_number or 42,
    pr_number = fields.pr_number,
    proposal_id = fields.proposal_id or "github-devloop/issue/owner/repo/42",
    version = fields.version or "2026-06-03T01-02-03Z",
    tick = fields.tick or "2026-06-10T01:02:03Z",
    exec = exec_with_env(root, fixtures),
  }
end

local function manifest_paths(manifest)
  local paths = {}
  for line in (tostring(manifest or "") .. "\n"):gmatch("([^\n]*)\n") do
    local path = line:match(":%s*(/.+)%s*$")
    if path ~= nil then
      table.insert(paths, path)
    end
  end
  return paths
end

local function has_path_suffix(paths, suffix)
  for _, path in ipairs(paths or {}) do
    if tostring(path):sub(-#suffix) == suffix then
      return true
    end
  end
  return false
end

local function read_file(path)
  local handle = assert(io.open(path, "r"))
  local content = handle:read("*a")
  handle:close()
  return content
end

local function write_file(path, content)
  local handle = assert(io.open(path, "w"))
  handle:write(content)
  handle:close()
end

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_single_quote(path))
  if not (ok == true or ok == 0) then
    error("mkdir failed")
  end
end

local function count_calls(calls, needle)
  local count = 0
  for _, rendered in ipairs(calls or {}) do
    if rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function assert_readable_from_cwd(path, cwd)
  local cmd = "cd " .. shell_single_quote(cwd)
    .. " && test -r " .. shell_single_quote(path)
    .. " && cat " .. shell_single_quote(path) .. " >/dev/null"
  local ok = os.execute(cmd)
  if not (ok == true or ok == 0) then
    error("cross-cwd read failed")
  end
end

local function run_round_trip(root)
  local fixtures = {}
  local bundle = core.build_context_bundle(build_args(root, fixtures, { pr_number = 7 }))
  local paths = manifest_paths(core.context_bundle_manifest(bundle))
  local scratch = root .. "/isolated-scratch"
  mkdir_p(scratch)
  local contents = {}
  for _, path in ipairs(paths) do
    assert_readable_from_cwd(path, scratch)
    table.insert(contents, read_file(path))
  end
  return {
    paths = paths,
    contents = contents,
    manifest = core.context_bundle_manifest(bundle),
    issue_content = read_file(bundle.issue_path),
    notice_content = read_file(bundle.notice_path),
  }
end

local function run_deleted_file(root)
  local fixtures = {
    issue_outputs = {
      '{"title":"First issue","body":"first","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
      '{"title":"Second issue","body":"second","updatedAt":"2026-06-03T01:02:04Z","state":"OPEN","labels":[],"comments":[]}\n',
    },
  }
  local args = build_args(root, fixtures)
  local first = core.build_context_bundle(args)
  os.remove(first.issue_path)
  local second = core.build_context_bundle(args)
  return {
    first_dir = first.dir,
    second_dir = second.dir,
    issue_content = read_file(second.issue_path),
    issue_fetch_count = count_calls(fixtures.calls, "gh issue view"),
  }
end

local function run_preexisting(root)
  local fixtures = {}
  local dir = root .. "/context/github-devloop-issue-owner-repo-42/2026-06-03T01-02-03Z"
  mkdir_p(dir)
  write_file(dir .. "/UNTRUSTED-NOTICE.txt", "BEGIN UNTRUSTED BUNDLE DATA\npreexisting notice\nEND UNTRUSTED BUNDLE DATA\n")
  write_file(dir .. "/issue.json", "preexisting issue\n")
  write_file(dir .. "/board.txt", "preexisting board\n")
  local bundle = core.build_context_bundle(build_args(root, fixtures))
  return {
    dir = bundle.dir,
    expected_dir = dir,
    issue_content = read_file(bundle.issue_path),
    manifest = core.context_bundle_manifest(bundle),
    issue_fetch_count = count_calls(fixtures.calls, "gh issue view"),
  }
end

local function run_publish_reuse(root)
  local fixtures = {
    issue_outputs = {
      '{"title":"First publish","body":"first","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
      '{"title":"Second publish","body":"second","updatedAt":"2026-06-03T01:02:04Z","state":"OPEN","labels":[],"comments":[]}\n',
    },
  }
  local args = build_args(root, fixtures)
  local first = core.build_context_bundle(args)
  local before_notice = read_file(first.notice_path)
  local before_issue = read_file(first.issue_path)
  local before_board = read_file(first.board_path)
  local fetches_after_first = count_calls(fixtures.calls, "gh issue view")
  local second = core.build_context_bundle(args)
  return {
    first_dir = first.dir,
    second_dir = second.dir,
    fetches_after_first = fetches_after_first,
    fetches_after_second = count_calls(fixtures.calls, "gh issue view"),
    notice_unchanged = before_notice == read_file(first.notice_path),
    issue_unchanged = before_issue == read_file(first.issue_path),
    board_unchanged = before_board == read_file(first.board_path),
  }
end

local function run_publish_unique_on_invalid(root)
  local fixtures = {
    issue_outputs = {
      '{"title":"First publish","body":"first","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
      '{"title":"Rebuilt issue","body":"rebuilt","updatedAt":"2026-06-03T01:02:04Z","state":"OPEN","labels":[],"comments":[]}\n',
    },
  }
  local args = build_args(root, fixtures)
  local first = core.build_context_bundle(args)
  os.remove(first.notice_path)
  write_file(first.issue_path, "invalid first issue remains\n")
  local before_issue = read_file(first.issue_path)
  local before_board = read_file(first.board_path)
  local second = core.build_context_bundle(args)
  return {
    dir = second.dir,
    original_dir = first.dir,
    issue_fetch_count = count_calls(fixtures.calls, "gh issue view"),
    original_notice_absent = io.open(first.notice_path, "r") == nil,
    original_issue_unchanged = before_issue == read_file(first.issue_path),
    original_board_unchanged = before_board == read_file(first.board_path),
    rebuilt_issue = read_file(second.issue_path),
    manifest = core.context_bundle_manifest(second),
    has_notice = has_path_suffix(manifest_paths(core.context_bundle_manifest(second)), "/UNTRUSTED-NOTICE.txt"),
  }
end

local function run_utf8_truncation(root)
  local fixture_data = {
    issue_outputs = {
      string.rep("a", core._max_bundle_file_len - 1) .. fixtures.cjk_char() .. "tail",
    },
  }
  local bundle = core.build_context_bundle(build_args(root, fixture_data, { tick = nil }))
  return {
    issue_content = read_file(bundle.issue_path),
    issue_bytes = bundle.issue_bytes,
  }
end

function pipeline(event)
  local payload = event.payload or {}
  local root = payload.root
  if payload.mode == "round_trip" then
    raise("context_bundle_probe_result", run_round_trip(root))
  elseif payload.mode == "deleted_file" then
    raise("context_bundle_probe_result", run_deleted_file(root))
  elseif payload.mode == "preexisting" then
    raise("context_bundle_probe_result", run_preexisting(root))
  elseif payload.mode == "publish_reuse" then
    raise("context_bundle_probe_result", run_publish_reuse(root))
  elseif payload.mode == "publish_unique_on_invalid" then
    raise("context_bundle_probe_result", run_publish_unique_on_invalid(root))
  elseif payload.mode == "utf8_truncation" then
    raise("context_bundle_probe_result", run_utf8_truncation(root))
  else
    error("unknown context bundle probe mode")
  end
end
