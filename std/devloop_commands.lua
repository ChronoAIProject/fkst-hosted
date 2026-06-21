local S = {}

local modules = {
  "std.devloop_commands.support",
  "std.devloop_commands.validators",
  "std.devloop_commands.issue_reads",
  "std.devloop_commands.observe_lists",
  "std.devloop_commands.dashboard",
  "std.devloop_commands.labels",
  "std.devloop_commands.prs",
  "std.devloop_commands.git_ops",
}

function S.install(M)
  for _, module_name in ipairs(modules) do
    require(module_name).install(M)
  end
end

return S
