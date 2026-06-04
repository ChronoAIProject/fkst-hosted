local t = fkst.test

local function sh_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function run_cmd(cmd)
  local result = exec_sync({ cmd = cmd, timeout = 30 })
  t.eq(result.exit_code, 0)
  return result
end

local function make_temp_dir()
  local result = run_cmd("mktemp -d")
  return (result.stdout:gsub("%s+$", ""))
end

local function write_fake_gh(fakebin)
  local gh_path = fakebin .. "/gh"
  local script = [=[#!/usr/bin/env bash
set -euo pipefail
LOG="${FAKE_GH_LOG:?}"
printf 'ARGS:' >> "$LOG"
printf ' [%s]' "$@" >> "$LOG"
printf '\n' >> "$LOG"

if [[ "${1:-}" == "issue" && "${2:-}" == "list" ]]; then
  if [[ -n "${FAKE_GH_ISSUE_LIST_EXIT:-}" ]]; then
    printf 'forced issue list failure\n' >&2
    exit "$FAKE_GH_ISSUE_LIST_EXIT"
  fi
  updated_at="${FAKE_GH_ISSUE_UPDATED_AT:-2026-06-03T01:02:03Z}"
  state="${FAKE_GH_ISSUE_STATE:-OPEN}"
  printf '[{"number":42,"title":"Bridge issue","url":"https://github.example/owner/x/issues/42","updatedAt":"%s","state":"%s"}]\n' "$updated_at" "$state"
  exit 0
fi

if [[ "${1:-}" == "pr" && "${2:-}" == "list" ]]; then
  if [[ -n "${FAKE_GH_PR_LIST_EXIT:-}" ]]; then
    printf 'forced pr list failure\n' >&2
    exit "$FAKE_GH_PR_LIST_EXIT"
  fi
  updated_at="${FAKE_GH_PR_UPDATED_AT:-2026-06-03T02:03:04Z}"
  state="${FAKE_GH_PR_STATE:-OPEN}"
  printf '[{"number":7,"title":"Bridge PR","url":"https://github.example/owner/x/pull/7","updatedAt":"%s","state":"%s"}]\n' "$updated_at" "$state"
  exit 0
fi

if [[ "${1:-}" == "issue" && "${2:-}" == "view" ]]; then
  printf '{"comments":[{"body":"existing comment\n'
  if [[ -n "${FAKE_GH_STATE:-}" && -f "$FAKE_GH_STATE/comments" ]]; then
    cat "$FAKE_GH_STATE/comments"
  fi
  printf '"}]}\n'
  exit 0
fi

if [[ "${1:-}" == "issue" && "${2:-}" == "comment" ]]; then
  body_file=""
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--body-file" ]]; then
      body_file="$2"
      break
    fi
    shift
  done
  {
    printf 'BODY_BEGIN\n'
    cat "$body_file"
    printf 'BODY_END\n'
  } >> "$LOG"
  if [[ -n "${FAKE_GH_STATE:-}" ]]; then
    mkdir -p "$FAKE_GH_STATE"
    cat "$body_file" >> "$FAKE_GH_STATE/comments"
  fi
  exit 0
fi

printf 'unexpected gh invocation\n' >&2
exit 9
]=]
  run_cmd("printf %s " .. sh_quote(script) .. " > " .. sh_quote(gh_path))
  run_cmd("chmod +x " .. sh_quote(gh_path))
end

local function setup()
  local tmp = make_temp_dir()
  local fakebin = tmp .. "/bin"
  local runtime = tmp .. "/runtime"
  local gh_log = tmp .. "/gh.log"
  local gh_state = tmp .. "/gh-state"
  run_cmd(
    "mkdir -p "
      .. sh_quote(fakebin)
      .. " "
      .. sh_quote(runtime)
      .. " "
      .. sh_quote(gh_state)
      .. " && : > "
      .. sh_quote(gh_log)
  )
  write_fake_gh(fakebin)
  return {
    tmp = tmp,
    fakebin = fakebin,
    runtime = runtime,
    gh_log = gh_log,
    gh_state = gh_state,
  }
end

local function cleanup(ctx)
  run_cmd("rm -rf " .. sh_quote(ctx.tmp))
end

local function base_env(ctx)
  return {
    FKST_GITHUB_REPO = "owner/x",
    FKST_RUNTIME_ROOT = ctx.runtime,
    FAKE_GH_LOG = ctx.gh_log,
    FAKE_GH_STATE = ctx.gh_state,
  }
end

local function count_fixed(text, needle)
  local count = 0
  local start = 1
  while true do
    local found = text:find(needle, start, true)
    if found == nil then
      return count
    end
    count = count + 1
    start = found + #needle
  end
end

return {
  test_inbound_poll_raises_issue_and_pr_then_cache_hit = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local event = { queue = "github_poll_tick", payload = {} }
      local opts = {
        cwd = ctx.tmp,
        env = base_env(ctx),
        path_prepend = ctx.fakebin,
      }

      local first = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(first.exit_code, 0)
      t.eq(first.raises[1].queue, "github_entity_changed")
      t.eq(first.raises[1].payload.type, "issue")
      t.eq(first.raises[1].payload.repo, "owner/x")
      t.eq(first.raises[1].payload.number, 42)
      t.eq(first.raises[1].payload.title, "Bridge issue")
      t.eq(first.raises[1].payload.updated_at, "2026-06-03T01:02:03Z")
      t.eq(first.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-03T01:02:03Z")
      t.eq(first.raises[1].payload.source_ref.kind, "external")
      t.eq(first.raises[1].payload.source_ref.ref, "owner/x#issue/42")
      t.eq(first.raises[2].queue, "github_entity_changed")
      t.eq(first.raises[2].payload.type, "pr")
      t.eq(first.raises[2].payload.repo, "owner/x")
      t.eq(first.raises[2].payload.number, 7)
      t.eq(first.raises[2].payload.title, "Bridge PR")
      t.eq(first.raises[2].payload.url, "https://github.example/owner/x/pull/7")
      t.eq(first.raises[2].payload.state, "OPEN")
      t.eq(first.raises[2].payload.updated_at, "2026-06-03T02:03:04Z")
      t.eq(first.raises[2].payload.dedup_key, "owner/x#pr#7@2026-06-03T02:03:04Z")
      t.eq(first.raises[2].payload.source_ref.kind, "external")
      t.eq(first.raises[2].payload.source_ref.ref, "owner/x#pr/7")
      t.is_nil(first.raises[3])

      local second = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(second.exit_code, 0)
      t.eq(#second.raises, 0)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_inbound_poll_re_raises_when_updated_at_changes = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local event = { queue = "github_poll_tick", payload = {} }
      local env = base_env(ctx)
      local opts = {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      }

      local first = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(first.exit_code, 0)
      t.eq(#first.raises, 2)

      env.FAKE_GH_ISSUE_UPDATED_AT = "2026-06-04T05:06:07Z"
      env.FAKE_GH_PR_UPDATED_AT = "2026-06-04T06:07:08Z"
      local changed = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(changed.exit_code, 0)
      t.eq(#changed.raises, 2)
      t.eq(changed.raises[1].payload.type, "issue")
      t.eq(changed.raises[1].payload.updated_at, "2026-06-04T05:06:07Z")
      t.eq(changed.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-04T05:06:07Z")
      t.eq(changed.raises[2].payload.type, "pr")
      t.eq(changed.raises[2].payload.updated_at, "2026-06-04T06:07:08Z")
      t.eq(changed.raises[2].payload.dedup_key, "owner/x#pr#7@2026-06-04T06:07:08Z")
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_inbound_poll_re_raises_closed_lifecycle_state_when_updated_at_changes = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local event = { queue = "github_poll_tick", payload = {} }
      local env = base_env(ctx)
      local opts = {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      }

      local first = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(first.exit_code, 0)
      t.eq(#first.raises, 2)
      t.eq(first.raises[1].payload.type, "issue")
      t.eq(first.raises[1].payload.state, "OPEN")

      env.FAKE_GH_ISSUE_UPDATED_AT = "2026-06-04T09:10:11Z"
      env.FAKE_GH_ISSUE_STATE = "CLOSED"
      local closed = t.run_department("departments/github_poll/main.lua", event, opts)
      t.eq(closed.exit_code, 0)
      t.eq(#closed.raises, 1)
      t.eq(closed.raises[1].queue, "github_entity_changed")
      t.eq(closed.raises[1].payload.type, "issue")
      t.eq(closed.raises[1].payload.number, 42)
      t.eq(closed.raises[1].payload.updated_at, "2026-06-04T09:10:11Z")
      t.eq(closed.raises[1].payload.state, "CLOSED")
      t.eq(closed.raises[1].payload.dedup_key, "owner/x#issue#42@2026-06-04T09:10:11Z")
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_inbound_poll_continues_when_issue_list_fails = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local env = base_env(ctx)
      env.FAKE_GH_ISSUE_LIST_EXIT = "2"

      local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 1)
      t.eq(result.raises[1].queue, "github_entity_changed")
      t.eq(result.raises[1].payload.type, "pr")
      t.eq(file.read(ctx.gh_log):find("ARGS: [issue] [list]", 1, true) ~= nil, true)
      t.eq(file.read(ctx.gh_log):find("ARGS: [pr] [list]", 1, true) ~= nil, true)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_inbound_poll_continues_when_pr_list_fails = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local env = base_env(ctx)
      env.FAKE_GH_PR_LIST_EXIT = "2"

      local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 1)
      t.eq(result.raises[1].queue, "github_entity_changed")
      t.eq(result.raises[1].payload.type, "issue")
      t.eq(file.read(ctx.gh_log):find("ARGS: [issue] [list]", 1, true) ~= nil, true)
      t.eq(file.read(ctx.gh_log):find("ARGS: [pr] [list]", 1, true) ~= nil, true)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_inbound_poll_no_raise_without_repo_env = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local result = t.run_department("departments/github_poll/main.lua", { queue = "github_poll_tick", payload = {} }, {
        cwd = ctx.tmp,
        env = {
          FKST_GITHUB_REPO = "",
          FKST_RUNTIME_ROOT = ctx.runtime,
          FAKE_GH_LOG = ctx.gh_log,
          FAKE_GH_STATE = ctx.gh_state,
        },
        path_prepend = ctx.fakebin,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
      t.eq(file.read(ctx.gh_log):find("ARGS: [issue] [list]", 1, true), nil)
      t.eq(file.read(ctx.gh_log):find("ARGS: [pr] [list]", 1, true), nil)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_outbound_dry_run_write_and_marker_idempotency = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local event = {
        queue = "github_issue_comment_request",
        payload = {
          issue_number = 42,
          body = "fkst reply",
          dedup_key = "reply-42",
        },
      }

      local env = base_env(ctx)
      local dry = t.run_department("departments/github_comment/main.lua", event, {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })
      t.eq(dry.exit_code, 0)
      t.eq(file.read(ctx.gh_log):find("ARGS: [issue] [comment]", 1, true), nil)

      run_cmd(": > " .. sh_quote(ctx.gh_log) .. " && rm -rf " .. sh_quote(ctx.gh_state) .. " && mkdir -p " .. sh_quote(ctx.gh_state))
      env.FKST_GITHUB_WRITE = "1"
      local write = t.run_department("departments/github_comment/main.lua", event, {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })
      t.eq(write.exit_code, 0)

      local again = t.run_department("departments/github_comment/main.lua", event, {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })
      t.eq(again.exit_code, 0)

      local log_text = file.read(ctx.gh_log)
      t.eq(count_fixed(log_text, "ARGS: [issue] [comment]"), 1)
      t.is_true(log_text:find("<!-- fkst:github-proxy:comment:reply-42 -->", 1, true) ~= nil)
      t.eq(log_text:find("github.com", 1, true), nil)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,
}
