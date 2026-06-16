return {
  family = "converge-round",
  resolver = "converge-round",
  surface = "issue-comment-stream",
  version_form = "raw",
  producer = "departments/loop/main.lua",
  queue = "github-proxy.github_issue_comment_request",
  marker_source = "core/convergence.lua",
  marker_builder = "converge_round_marker",
  request_builder = "build_converge_round_comment_request",
}
