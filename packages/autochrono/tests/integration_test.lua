local t = fkst.test

local draft_body = "Thanks for opening this. I will review the details and follow up with the next concrete step."

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

local function write_fake_codex(fakebin)
  local codex_path = fakebin .. "/codex"
  local script = [=[#!/usr/bin/env bash
set -euo pipefail
while IFS= read -r _line; do
  :
done
if [[ -n "${FAKE_CODEX_EXIT:-}" ]]; then
  exit "$FAKE_CODEX_EXIT"
fi
if [[ -n "${FAKE_CODEX_EMPTY:-}" ]]; then
  exit 0
fi
printf '%s\n' "${FAKE_CODEX_BODY:?}"
]=]
  run_cmd("printf %s " .. sh_quote(script) .. " > " .. sh_quote(codex_path))
  run_cmd("chmod +x " .. sh_quote(codex_path))
end

local function setup()
  local tmp = make_temp_dir()
  local fakebin = tmp .. "/bin"
  local runtime = tmp .. "/runtime"
  run_cmd("mkdir -p " .. sh_quote(fakebin) .. " " .. sh_quote(runtime))
  write_fake_codex(fakebin)
  return {
    tmp = tmp,
    fakebin = fakebin,
    runtime = runtime,
  }
end

local function cleanup(ctx)
  run_cmd("rm -rf " .. sh_quote(ctx.tmp))
end

local function base_env(ctx)
  return {
    FKST_RUNTIME_ROOT = ctx.runtime,
    FAKE_CODEX_BODY = draft_body,
  }
end

local function issue(extra)
  local value = {
    schema = "autochrono.issue.v1",
    repo = "owner/repo",
    issue_number = 42,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function run_reply(event_payload, opts)
  return t.run_department("departments/reply/main.lua", {
    queue = "issue",
    payload = event_payload,
  }, opts)
end

return {
  test_reply_raises_once_then_cache_hit_skips = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local opts = {
        cwd = ctx.tmp,
        env = base_env(ctx),
        path_prepend = ctx.fakebin,
      }

      local first = run_reply(issue(), opts)
      t.eq(first.exit_code, 0)
      t.eq(#first.raises, 1)
      t.eq(first.raises[1].queue, "reply")
      t.eq(first.raises[1].payload.schema, "autochrono.reply.v1")
      t.eq(first.raises[1].payload.repo, "owner/repo")
      t.eq(first.raises[1].payload.issue_number, 42)
      t.eq(first.raises[1].payload.body, draft_body)
      t.eq(first.raises[1].payload.dedup_key, "autochrono:owner/repo#issue/42")
      t.eq(first.raises[1].payload.source_ref.kind, "external")
      t.eq(first.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

      local second = run_reply(issue({ updated_at = "2026-06-04T05:06:07Z" }), opts)
      t.eq(second.exit_code, 0)
      t.eq(#second.raises, 0)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_reply_degrades_when_codex_fails = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local env = base_env(ctx)
      env.FAKE_CODEX_EXIT = "7"
      local result = run_reply(issue(), {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,

  test_reply_degrades_when_codex_stdout_is_empty = function()
    local ctx = setup()
    local ok, err = pcall(function()
      local env = base_env(ctx)
      env.FAKE_CODEX_EMPTY = "1"
      local result = run_reply(issue(), {
        cwd = ctx.tmp,
        env = env,
        path_prepend = ctx.fakebin,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
    end)
    cleanup(ctx)
    if not ok then
      error(err)
    end
  end,
}
