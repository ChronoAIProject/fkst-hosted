-- Filesystem effects for the health report: directory setup, ATOMIC publish, and
-- retention pruning. Every effect is confined to `<FKST_RUNTIME_ROOT>/health`.
--
-- WHY ATOMIC. The control-plane collector polls this directory roughly twice a second
-- and copies a file whenever its mtime or size changes. A report written in place
-- would therefore be collected and published half-written. So the body is written
-- under a temporary name in the SAME directory and renamed into place: rename(2)
-- within one filesystem is atomic, and a reader either sees the old name or the whole
-- new file, never a partial one.
--
-- The temporary name is a dotfile ending in `.partial`, which the control plane's
-- filename parser rejects twice over -- it refuses names beginning with `.` and
-- requires a `.md` suffix -- so an interrupted tick can never leave behind something
-- the collector will pick up.
local report = require("report")

local W = {}

-- A long-lived session ticks every ten minutes; without a cap its pod would
-- accumulate reports for as long as it lives. The newest 200 is over a day of
-- history at this cadence.
W.retention = 200
-- Where the engine writes one log per department invocation.
W.fault_log_directory = "logs/framework-child"
W.fault_reason_ceiling = 200
W.directory_leaf = "health"
W.partial_prefix = "."
W.partial_suffix = ".partial"

local process_timeout_seconds = 30

local function runner_of(handles)
  local runner = type(handles) == "table" and handles.exec_argv or nil
  if type(runner) == "function" then
    return runner
  end
  if type(exec_argv) == "function" then
    return exec_argv
  end
  return nil
end

local function files_of(handles)
  local files = type(handles) == "table" and handles.file or nil
  if type(files) == "table" then
    return files
  end
  if type(file) == "table" then
    return file
  end
  return nil
end

-- argv, never a shell string: the paths here are built from environment values, and a
-- shell would give a hostile one a quoting escape. This also keeps the module clear of
-- the gh/git adapter ratchet, which only governs those two commands.
local function run(runner, argv)
  if type(runner) ~= "function" then
    return false, "no process runner available"
  end
  local ok, result = pcall(runner, { argv = argv, timeout = process_timeout_seconds })
  if not ok then
    return false, tostring(result)
  end
  if type(result) ~= "table" then
    return false, "process runner returned no result"
  end
  local code = tonumber(result.exit_code)
  if code ~= 0 then
    return false, "exit_code=" .. tostring(result.exit_code)
  end
  return true, nil
end

local function basename(path)
  return tostring(path):match("([^/]+)$") or tostring(path)
end

--- Is this the name of a published report? Mirrors the control plane's parser closely
--- enough to decide what pruning owns: a `.md` file carrying the fixed marker and a
--- trailing `YYYYMMDD-HHMMSS` stamp. Anything else in the directory is left alone.
function W.report_stamp(name)
  local text = tostring(name)
  if text:sub(1, 1) == "." then
    return nil
  end
  if text:sub(-#report.filename_suffix) ~= report.filename_suffix then
    return nil
  end
  local id = text:sub(1, #text - #report.filename_suffix)
  -- The LAST occurrence of the marker is the real one: whatever follows it is the
  -- stamp, and a stamp cannot contain the marker.
  local at = nil
  local from = 1
  while true do
    local found = id:find(report.filename_marker, from, true)
    if found == nil then
      break
    end
    at = found
    from = found + 1
  end
  if at == nil or at == 1 then
    return nil
  end
  local stamp = id:sub(at + #report.filename_marker)
  if stamp:match("^%d%d%d%d%d%d%d%d%-%d%d%d%d%d%d$") == nil then
    return nil
  end
  return stamp
end

function W.new(handles)
  local self = {}
  local runner = runner_of(handles)
  local files = files_of(handles)

  function self.directory(runtime_root)
    return tostring(runtime_root):gsub("/+$", "") .. "/" .. W.directory_leaf
  end

  function self.ensure(path)
    return run(runner, { "mkdir", "-p", path })
  end

  --- The terminal error line for a failing department, read from its newest
  --- framework-child log.
  ---
  --- WHY THIS EXISTS. The engine's observe snapshot caps `error_excerpt` at 500
  --- characters, and a department's log preamble (timestamps, provenance, package
  --- versions) consumes all of it -- so the actual cause is ALWAYS past the cap and
  --- never appears in the snapshot. The only place it exists is the child log. A
  --- health report that cannot say WHY something failed is not worth writing, so this
  --- goes and gets it.
  ---
  --- Best-effort and bounded: any failure returns nil and the report degrades to
  --- naming the log instead of quoting it. `dept` is validated before it reaches a
  --- shell, even though it originates from the engine and not from a user.
  function self.terminal_error(runtime_root, dept)
    if type(runner) ~= "function" or type(dept) ~= "string" then
      return nil
    end
    if dept:find("^[%w%-%._]+$") == nil then
      return nil
    end
    local directory = tostring(runtime_root):gsub("/+$", "") .. "/" .. W.fault_log_directory
    local ok_list, listed = pcall(runner, {
      argv = { "sh", "-c", 'ls -t "$0"/"$1"-*.log 2>/dev/null | head -1', directory, dept },
      timeout = process_timeout_seconds,
    })
    if not ok_list or type(listed) ~= "table" or tonumber(listed.exit_code) ~= 0 then
      return nil
    end
    local path = tostring(listed.stdout or ""):gsub("%s+$", "")
    if path == "" then
      return nil
    end
    -- The LAST ERROR line is the terminal cause; a retry ladder prints
    -- "Reconnecting... n/5" ahead of it, which is noise.
    local ok_grep, found = pcall(runner, {
      argv = {
        "sh",
        "-c",
        'grep -a "ERROR:" "$0" 2>/dev/null | grep -v "Reconnecting" | tail -1',
        path,
      },
      timeout = process_timeout_seconds,
    })
    if not ok_grep or type(found) ~= "table" then
      return nil
    end
    local line = tostring(found.stdout or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if line == "" then
      return nil
    end
    return line:sub(1, W.fault_reason_ceiling), path
  end

  --- Write `text` to `<directory>/<name>` atomically. Returns ok, why.
  function self.publish(directory, name, text)
    if type(files) ~= "table" or type(files.write) ~= "function" then
      return false, "no file port available"
    end
    local final = directory .. "/" .. name
    local partial = directory .. "/" .. W.partial_prefix .. name .. W.partial_suffix
    local written, why = pcall(files.write, partial, text)
    if not written then
      return false, tostring(why)
    end
    local moved, move_why = run(runner, { "mv", "-f", partial, final })
    if not moved then
      -- Leave nothing half-published behind; a failed cleanup is not worth reporting
      -- over the rename failure that caused it.
      run(runner, { "rm", "-f", partial })
      return false, move_why
    end
    return true, nil
  end

  --- Delete all but the newest `W.retention` reports. Returns the number removed.
  function self.prune(directory)
    if type(files) ~= "table" or type(files.list) ~= "function" then
      return 0
    end
    local ok, listing = pcall(files.list, directory)
    if not ok or type(listing) ~= "table" then
      return 0
    end
    local reports = {}
    for _, path in ipairs(listing) do
      local stamp = W.report_stamp(basename(path))
      if stamp ~= nil then
        table.insert(reports, { path = path, stamp = stamp })
      end
    end
    if #reports <= W.retention then
      return 0
    end
    -- Sorted by the UTC stamp, not by path: the namespace/session prefix leads every
    -- filename, so path order is only chronological by accident.
    table.sort(reports, function(left, right)
      if left.stamp == right.stamp then
        return left.path < right.path
      end
      return left.stamp < right.stamp
    end)
    local removed = 0
    for index = 1, #reports - W.retention do
      if run(runner, { "rm", "-f", reports[index].path }) then
        removed = removed + 1
      end
    end
    return removed
  end

  --- Write the codex's evidence context. Best effort: a failure just means the judge
  --- narrates from the prompt's inline summary alone.
  function self.write_context(path, text)
    if type(files) ~= "table" or type(files.write) ~= "function" then
      return false
    end
    return pcall(files.write, path, text) == true
  end

  return self
end

return W
