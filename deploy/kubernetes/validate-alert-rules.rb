#!/usr/bin/env ruby
# frozen_string_literal: true

# Evaluates every alert expression in the rendered PrometheusRules against the
# synthetic fixtures in `monitoring/alert-fixtures.yaml`, and fails when a rule
# cannot be made to fire, cannot be made to clear, or disagrees with a fixture's
# stated expectation.
#
# usage: validate-alert-rules.rb MONITORING_YAML FIXTURES_YAML
#
# ## Why an evaluator lives here
#
# `promtool test rules` is the obvious tool, and this repository deliberately does
# NOT use it — there is no promtool invocation here or anywhere else in the tree.
# It is not a dependency that can be required: it ships inside the Prometheus
# release tarball, and the deployment validators must run on a machine with
# nothing but Ruby and kubectl. An unverifiable fixture file would be worse than
# none, so the fixtures are executable HERE, deterministically, by the evaluator
# below.
#
# The trade-off is stated plainly because it bounds what a green run means: this
# evaluator understands only the grammar the checked-in rules use (below), and
# proves each rule fires and clears against the fixtures. It is not a
# general-purpose PromQL implementation, and a rule written outside the grammar
# is a hard error rather than an unverified pass.
#
# ## The grammar this evaluator supports
#
# Deliberately only the subset the checked-in rules use. Anything outside it is a
# hard error rather than a silent pass, so an expression written in a richer
# dialect fails review instead of going unverified:
#
#   expr     := operand (('and' | 'or') operand)*
#   operand  := term (('>' | '<' | '==') NUMBER)?
#   term     := factor (('/' | '-') factor)?
#   factor   := 'time()' | NUMBER | 'absent(' selector ')'
#             | ('max' | 'sum') '(' inner ')'
#   inner    := selector | 'increase(' selector '[' duration '] )'
#   selector := NAME '{' matcher (',' matcher)* '}'
#   matcher  := LABEL ('=' | '=~') '"' VALUE '"'
#
# Values model Prometheus's instant-vector semantics in the only shape these
# rules produce: `nil` is the empty vector (no alert), a Float is a one-element
# vector. `and` keeps the left side when the right side is non-empty; `or` takes
# the left side when it is non-empty. An alert fires iff the expression evaluates
# to non-nil.

require "set"
require "yaml"

abort "usage: #{$PROGRAM_NAME} MONITORING_YAML FIXTURES_YAML" unless ARGV.length == 2

MONITORING_YAML, FIXTURES_YAML = ARGV

problems = []

# ------------------------------------------------------------------- expression

# A single metric sample: name, labels, instantaneous value, windowed increase.
Sample = Struct.new(:metric, :labels, :value, :increase)

# Parse `name{label="value",other=~"a|b"}` into a matcher predicate.
def parse_selector(text)
  name, rest = text.split("{", 2)
  name = name.strip
  matchers = []
  if rest
    body = rest.sub(/\}\s*\z/, "")
    body.scan(/([a-zA-Z_][a-zA-Z0-9_]*)\s*(=~|=)\s*"([^"]*)"/) do |label, operator, value|
      matchers << [label, operator, value]
    end
  end
  [name, matchers]
end

def sample_matches?(sample, name, matchers)
  return false unless sample.metric == name

  matchers.all? do |label, operator, value|
    actual = sample.labels[label].to_s
    operator == "=~" ? actual.match?(/\A(?:#{value})\z/) : actual == value
  end
end

def select_samples(samples, text)
  name, matchers = parse_selector(text)
  samples.select { |sample| sample_matches?(sample, name, matchers) }
end

# Split on a top-level infix keyword, ignoring anything inside parentheses.
def split_top_level(text, keyword)
  depth = 0
  index = 0
  parts = []
  current = +""
  needle = " #{keyword} "
  while index < text.length
    char = text[index]
    depth += 1 if char == "("
    depth -= 1 if char == ")"
    if depth.zero? && text[index, needle.length] == needle
      parts << current
      current = +""
      index += needle.length
      next
    end
    current << char
    index += 1
  end
  parts << current
  parts
end

def evaluate(expression, samples, now)
  text = expression.strip
  %w[or and].each do |keyword|
    parts = split_top_level(text, keyword)
    next if parts.length < 2

    values = parts.map { |part| evaluate(part, samples, now) }
    return keyword == "or" ? values.find { |value| !value.nil? } : combine_and(values)
  end
  evaluate_operand(text, samples, now)
end

# `a and b and c` keeps a's value only when every other side is non-empty.
def combine_and(values)
  return nil if values.any?(&:nil?)

  values.first
end

def evaluate_operand(text, samples, now)
  if (match = text.match(/\A(.*?)(>=|<=|==|>|<)\s*([0-9.eE+-]+)\s*\z/))
    left = evaluate_term(match[1].strip, samples, now)
    return nil if left.nil?

    bound = Float(match[3])
    kept = case match[2]
           when ">" then left > bound
           when "<" then left < bound
           when ">=" then left >= bound
           when "<=" then left <= bound
           when "==" then left == bound
           end
    return kept ? left : nil
  end
  evaluate_term(text, samples, now)
end

def evaluate_term(text, samples, now)
  text = text.strip
  text = text[1..-2].strip while text.start_with?("(") && balanced_wrapper?(text)

  ["/", "-"].each do |operator|
    parts = split_top_level(text, operator)
    next if parts.length < 2

    left = evaluate_term(parts[0], samples, now)
    right = evaluate_term(parts[1], samples, now)
    return nil if left.nil? || right.nil?
    return operator == "/" ? (right.zero? ? nil : left / right) : left - right
  end
  evaluate_factor(text, samples, now)
end

def balanced_wrapper?(text)
  depth = 0
  text.each_char.with_index do |char, index|
    depth += 1 if char == "("
    depth -= 1 if char == ")"
    return index == text.length - 1 if depth.zero?
  end
  false
end

def evaluate_factor(text, samples, now)
  text = text.strip
  return now.to_f if text == "time()"
  return Float(text) if text.match?(/\A[0-9.eE+-]+\z/)

  if (match = text.match(/\Aabsent\((.*)\)\z/m))
    return select_samples(samples, match[1]).empty? ? 1.0 : nil
  end
  if (match = text.match(/\A(max|sum)\((.*)\)\z/m))
    values = inner_values(match[2], samples)
    return nil if values.empty?

    return match[1] == "max" ? values.max : values.sum
  end
  raise "unsupported alert expression fragment: #{text.inspect}"
end

def inner_values(text, samples)
  text = text.strip
  if (match = text.match(/\Aincrease\((.*)\[[0-9]+[smhdw]\]\)\z/m))
    return select_samples(samples, match[1]).map { |sample| sample.increase.to_f }
  end

  select_samples(samples, text).map { |sample| sample.value.to_f }
end

# ---------------------------------------------------------------------- fixtures

fixtures = YAML.safe_load(File.read(FIXTURES_YAML))
abort "#{FIXTURES_YAML}: not a mapping" unless fixtures.is_a?(Hash)
now = fixtures.fetch("now")
label_sets = fixtures.fetch("label_sets", {})
cases_by_alert = fixtures.fetch("alerts")

def build_samples(case_entry, label_sets, alert, problems)
  case_entry.fetch("samples", []).map do |raw|
    labels = {}
    if (reference = raw["labels_ref"])
      unless label_sets.key?(reference)
        problems << "#{alert}: unknown labels_ref #{reference.inspect}"
        next nil
      end
      labels.merge!(label_sets.fetch(reference))
    end
    labels.merge!(raw.fetch("labels", {}))
    Sample.new(raw.fetch("metric"), labels, raw["value"], raw["increase"])
  end.compact
end

# ------------------------------------------------------------------------- rules

documents = YAML.load_stream(File.read(MONITORING_YAML)).compact
rules = documents
        .select { |object| object.is_a?(Hash) && object["kind"] == "PrometheusRule" }
        .flat_map { |object| object.dig("spec", "groups") || [] }
        .flat_map { |group| group.fetch("rules", []) }
        .select { |rule| rule.key?("alert") }

problems << "no alerting rules were found in #{MONITORING_YAML}" if rules.empty?

covered = Set.new
rules.each_with_index do |rule, index|
  alert = rule.fetch("alert")
  covered << alert
  cases = cases_by_alert[alert]
  if cases.nil? || cases.empty?
    problems << "#{alert}: no fixtures; every alert needs a firing and a clearing case"
    next
  end

  fired = []
  cleared = []
  cases.each do |case_entry|
    samples = build_samples(case_entry, label_sets, alert, problems)
    begin
      value = evaluate(rule.fetch("expr"), samples, now)
    rescue StandardError => e
      problems << "#{alert} [#{case_entry.fetch('name')}]: #{e.message}"
      next
    end
    (value.nil? ? cleared : fired) << case_entry.fetch("name")
  end

  # Per RULE INSTANCE: a ladder shares one alert name across two thresholds, and
  # a tier that can never fire is exactly the bug this file exists to catch.
  identity = "#{alert} (rule ##{index + 1})"
  problems << "#{identity}: no fixture makes it fire" if fired.empty?
  problems << "#{identity}: no fixture leaves it clear" if cleared.empty?
end

# Per ALERT NAME: the stated expectation must match the union across its rules.
cases_by_alert.each do |alert, cases|
  instances = rules.select { |rule| rule.fetch("alert") == alert }
  if instances.empty?
    problems << "#{alert}: fixtures exist for an alert no rule declares"
    next
  end
  cases.each do |case_entry|
    samples = build_samples(case_entry, label_sets, alert, problems)
    any_fired = instances.any? do |rule|
      !evaluate(rule.fetch("expr"), samples, now).nil?
    rescue StandardError
      false
    end
    expected = case_entry.fetch("expect")
    unless %w[fire clear].include?(expected)
      problems << "#{alert} [#{case_entry.fetch('name')}]: expect must be fire or clear"
      next
    end
    if expected == "fire" && !any_fired
      problems << "#{alert} [#{case_entry.fetch('name')}]: expected to fire, but no rule did"
    elsif expected == "clear" && any_fired
      problems << "#{alert} [#{case_entry.fetch('name')}]: expected to stay clear, but a rule fired"
    end
  end
end

missing = covered - cases_by_alert.keys
problems << "these alerts have no fixtures: #{missing.to_a.sort.join(', ')}" unless missing.empty?

if problems.empty?
  puts "alert rules: #{rules.length} rules across #{covered.length} alerts fire and clear on their fixtures"
  exit 0
end

warn "alert-rule fixture policy violations:"
problems.each { |problem| warn "  - #{problem}" }
exit 1
