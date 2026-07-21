#!/usr/bin/env ruby
# frozen_string_literal: true

require "fileutils"
require "json"
require "optparse"
require "set"
require "time"

options = {}
OptionParser.new do |parser|
  parser.banner = "usage: #{$PROGRAM_NAME} --output-dir PATH --result RESULT --failure-phase PHASE --started-at TIME --completed-at TIME --context-class CLASS --target-namespace NAMESPACE [recovery facts]"
  parser.on("--output-dir PATH") { |value| options[:output_dir] = value }
  parser.on("--result RESULT") { |value| options[:result] = value }
  parser.on("--failure-phase PHASE") { |value| options[:failure_phase] = value }
  parser.on("--started-at TIME") { |value| options[:started_at] = value }
  parser.on("--completed-at TIME") { |value| options[:completed_at] = value }
  parser.on("--context-class CLASS") { |value| options[:context_class] = value }
  parser.on("--target-namespace NAMESPACE") { |value| options[:target_namespace] = value }
  parser.on("--rto-seconds SECONDS") { |value| options[:rto_seconds] = value }
  parser.on("--repository-sha256 SHA256") { |value| options[:repository_sha256] = value }
  parser.on("--pre-session-count COUNT") { |value| options[:pre_session_count] = value }
  parser.on("--post-session-count COUNT") { |value| options[:post_session_count] = value }
  parser.on("--pre-session-set-sha256 SHA256") { |value| options[:pre_session_set_sha256] = value }
  parser.on("--post-session-set-sha256 SHA256") { |value| options[:post_session_set_sha256] = value }
  parser.on("--environment-content-hash SHA256") { |value| options[:environment_content_hash] = value }
  parser.on("--environment-secret-key-count COUNT") { |value| options[:environment_secret_key_count] = value }
  parser.on("--environment-secret-keys-sha256 SHA256") { |value| options[:environment_secret_keys_sha256] = value }
  parser.on("--lease-transitions-before COUNT") { |value| options[:lease_transitions_before] = value }
  parser.on("--lease-transitions-after COUNT") { |value| options[:lease_transitions_after] = value }
end.parse!
abort "unexpected positional evidence input" unless ARGV.empty?

required = %i[
  output_dir result failure_phase started_at completed_at context_class target_namespace
]
missing = required.select { |key| options[key].to_s.empty? }
abort "missing required evidence fields: #{missing.join(', ')}" unless missing.empty?

results = Set["passed", "failed"]
phases = Set[
  "none",
  "preflight",
  "runtime_inventory",
  "environment_inventory",
  "restore_preflight",
  "namespace_delete",
  "namespace_restore",
  "runtime_reconstruction",
  "post_verify"
]
abort "invalid evidence result" unless results.include?(options[:result])
abort "invalid evidence failure phase" unless phases.include?(options[:failure_phase])
if options[:result] == "passed"
  abort "passed evidence must have failure_phase=none" unless options[:failure_phase] == "none"
elsif options[:failure_phase] == "none"
  abort "failed evidence must identify a bounded failure phase"
end
abort "unreviewed context class" unless options[:context_class] == "kind_disposable"
abort "unreviewed target namespace" unless options[:target_namespace] == "chronoai-fkst"

timestamp = lambda do |key|
  parsed = Time.iso8601(options.fetch(key))
  abort "#{key} must be a UTC timestamp" unless parsed.utc_offset.zero?
  parsed.utc.iso8601
rescue ArgumentError
  abort "#{key} must be an ISO-8601 timestamp"
end

nullable_integer = lambda do |key|
  value = options[key].to_s
  next nil if value.empty?
  parsed = Integer(value, exception: false)
  abort "#{key} must be a non-negative integer" unless parsed && parsed >= 0
  parsed
end

nullable_hash = lambda do |key|
  value = options[key].to_s
  next nil if value.empty?
  abort "#{key} must be a lowercase SHA-256 value" unless value.match?(/\A[0-9a-f]{64}\z/)
  value
end

document = {
  "evidence_version" => 1,
  "result" => options.fetch(:result),
  "failure_phase" => options.fetch(:failure_phase),
  "started_at" => timestamp.call(:started_at),
  "completed_at" => timestamp.call(:completed_at),
  "rto_seconds" => nullable_integer.call(:rto_seconds),
  "context_class" => options.fetch(:context_class),
  "target_namespace" => options.fetch(:target_namespace),
  "repository_sha256" => nullable_hash.call(:repository_sha256),
  "pre_session_count" => nullable_integer.call(:pre_session_count),
  "post_session_count" => nullable_integer.call(:post_session_count),
  "pre_session_set_sha256" => nullable_hash.call(:pre_session_set_sha256),
  "post_session_set_sha256" => nullable_hash.call(:post_session_set_sha256),
  "environment_content_hash" => nullable_hash.call(:environment_content_hash),
  "environment_secret_key_count" => nullable_integer.call(:environment_secret_key_count),
  "environment_secret_keys_sha256" => nullable_hash.call(:environment_secret_keys_sha256),
  "lease_transitions_before" => nullable_integer.call(:lease_transitions_before),
  "lease_transitions_after" => nullable_integer.call(:lease_transitions_after),
  "github_mutations" => 0
}

if document["result"] == "passed"
  required_success = document.reject { |key, _value| %w[failure_phase].include?(key) }
  missing_success = required_success.select { |_key, value| value.nil? }.keys
  abort "passed evidence is incomplete: #{missing_success.join(', ')}" unless missing_success.empty?
  abort "passed evidence requires a prepared runtime" unless document["pre_session_count"].positive?
  unless document["pre_session_count"] == document["post_session_count"] &&
         document["pre_session_set_sha256"] == document["post_session_set_sha256"]
    abort "passed evidence requires the same deterministic session set"
  end
end

FileUtils.mkdir_p(options.fetch(:output_dir))
json_path = File.join(options.fetch(:output_dir), "recovery-evidence.json")
markdown_path = File.join(options.fetch(:output_dir), "recovery-evidence.md")

write_atomic = lambda do |path, contents|
  temporary = "#{path}.#{Process.pid}.tmp"
  File.open(temporary, File::WRONLY | File::CREAT | File::EXCL, 0o644) do |file|
    file.write(contents)
  end
  File.rename(temporary, path)
ensure
  File.unlink(temporary) if temporary && File.exist?(temporary)
end

write_atomic.call(json_path, "#{JSON.pretty_generate(document)}\n")

rows = document.map do |key, value|
  rendered = value.nil? ? "not captured" : value.to_s
  "| `#{key}` | `#{rendered}` |"
end
markdown = <<~MARKDOWN
  # FKST namespace recovery evidence

  This artifact contains only bounded outcomes, counts, timestamps, and SHA-256
  projections. Repository and session identities, environment values, credentials,
  issue content, raw cluster context, and command logs are intentionally absent.

  | Field | Value |
  |---|---|
  #{rows.join("\n")}
MARKDOWN
write_atomic.call(markdown_path, markdown)

puts "wrote redacted recovery evidence to #{options.fetch(:output_dir)}"
