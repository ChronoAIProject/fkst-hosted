return {
  family = "child-state",
  marker_family = "state",
  resolver = "child-state",
  surface = "pr-comment-stream",
  version_form = "raw",
  producer = "github-proxy.github_entity_changed",
  queue = "devloop_pr_terminal",
  marker_source = "core/state.lua",
  request_source = "departments/observe_pr/main.lua",
  marker_builder = "state_marker",
  request_builder = "devloop_pr_terminal",
  observe_only = true,
}
