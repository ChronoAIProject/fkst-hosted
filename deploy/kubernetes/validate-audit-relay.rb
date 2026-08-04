#!/usr/bin/env ruby
# frozen_string_literal: true

# Structural and security policy for the durable audit relay composition.
#
# It is a separate validator from `validate-render.rb` for the same reason the
# relay is a separate Kustomization: the relay is the one FKST workload that
# holds persistent state and the deployment's whole recent audit trail, and the
# properties that matter for it — a single writer, a bound volume, no Kubernetes
# API reach, no Ingress, credentials only ever by `secretKeyRef` — are not
# properties of the stateless base at all.
#
# Three renders are checked together:
#
#   RELAY_YAML     `audit-relay/`               the workload itself
#   REQUIRED_YAML  `overlays/required-audit/`   the composed production shape
#   BASE_YAML      `base/`                      the non-production default
#
# The cross-render checks are the ones that catch real mistakes: a grace that
# disagrees between the two ConfigMaps, a base that quietly selects required
# delivery, a relay volume that leaked into the control plane, or a PostHog
# credential that reached the frontend.

require "set"
require "yaml"

abort "usage: #{$PROGRAM_NAME} RELAY_YAML REQUIRED_YAML BASE_YAML" unless ARGV.length == 3

load_objects = lambda do |path|
  YAML.load_stream(File.read(path)).compact.map do |object|
    abort "#{path}: rendered document is not a mapping" unless object.is_a?(Hash)
    object
  end
end

identity = lambda do |object|
  [object.fetch("kind"), object.dig("metadata", "namespace").to_s,
   object.dig("metadata", "name").to_s]
end

relay = load_objects.call(ARGV.fetch(0))
required = load_objects.call(ARGV.fetch(1))
base = load_objects.call(ARGV.fetch(2))

fetch = lambda do |objects, kind, name, label|
  object = objects.find { |item| identity.call(item) == [kind, "chronoai-fkst", name] }
  abort "#{label} render is missing #{kind}/#{name}" unless object
  object
end

# The four values that must never appear outside a Secret record.
SECRET_VARS = %w[
  FKST_POSTHOG_PROJECT_TOKEN
  FKST_POSTHOG_QUERY_API_KEY
  FKST_AUDIT_RELAY_WRITE_TOKEN
  FKST_AUDIT_RELAY_READ_TOKEN
].freeze

# ---------------------------------------------------------------- relay render

expected = Set[
  ["ServiceAccount", "chronoai-fkst", "fkst-audit-relay"],
  ["ConfigMap", "chronoai-fkst", "fkst-audit-relay-config"],
  ["ExternalSecret", "chronoai-fkst", "fkst-audit-relay"],
  ["PersistentVolumeClaim", "chronoai-fkst", "fkst-audit-relay-data"],
  ["Deployment", "chronoai-fkst", "fkst-audit-relay"],
  ["Service", "chronoai-fkst", "fkst-audit-relay"],
  ["PodDisruptionBudget", "chronoai-fkst", "fkst-audit-relay"],
  ["NetworkPolicy", "chronoai-fkst", "fkst-audit-relay"]
]
actual = relay.map(&identity).to_set
abort "audit-relay render must contain exactly the reviewed resources" unless actual == expected
abort "audit-relay render must not contain Secret objects" if relay.any? { |object| object["kind"] == "Secret" }
if relay.any? { |object| object["kind"] == "Ingress" }
  abort "the audit relay must never be published through an Ingress"
end
rbac_kinds = Set["Role", "RoleBinding", "ClusterRole", "ClusterRoleBinding"]
if relay.any? { |object| rbac_kinds.include?(object["kind"]) }
  abort "the audit relay must hold no Kubernetes RBAC"
end

account = fetch.call(relay, "ServiceAccount", "fkst-audit-relay", "audit-relay")
unless account["automountServiceAccountToken"] == false
  abort "the relay ServiceAccount must not mount an API token"
end

config = fetch.call(relay, "ConfigMap", "fkst-audit-relay-config", "audit-relay")
config_data = config["data"] || {}
SECRET_VARS.each do |var|
  abort "#{var} must never be rendered into a ConfigMap" if config_data.key?(var)
end
abort "the relay ConfigMap must select a database path" if config_data["FKST_AUDIT_RELAY_DB_PATH"].to_s.empty?
relay_grace = config_data["FKST_AUDIT_INCOMPLETE_GRACE_SECS"].to_s
abort "the relay ConfigMap must pin the shared incomplete grace" if relay_grace.empty?

# The delivery host is a ConfigMap value, so a mistake in it is committed, and
# the relay is the process that carries the project capture token on every
# batch: `http://` ships that credential in cleartext and `https://user:token@…`
# parks it in this very object. The relay refuses both at startup; this refuses
# them at review time, where the fix is free. Rendering-time checks cannot know
# FKST_DEPLOYMENT_ENVIRONMENT's runtime value, so plaintext is judged against
# the environment the SAME ConfigMap declares.
check_posthog_host = lambda do |data, label|
  host = data["FKST_POSTHOG_HOST"].to_s
  next if host.empty?
  unless host.start_with?("http://", "https://")
    abort "#{label} FKST_POSTHOG_HOST must be an http(s) URL"
  end
  authority = host.split("://", 2).fetch(1).split(%r{[/?#]}, 2).fetch(0)
  abort "#{label} FKST_POSTHOG_HOST must not embed userinfo credentials" if authority.include?("@")
  next unless host.start_with?("http://")
  environment = data["FKST_DEPLOYMENT_ENVIRONMENT"].to_s.downcase
  unless %w[test local].include?(environment)
    abort "#{label} FKST_POSTHOG_HOST may only be plaintext in a test/local deployment"
  end
end
check_posthog_host.call(config_data, "the relay ConfigMap's")

external_secret = fetch.call(relay, "ExternalSecret", "fkst-audit-relay", "audit-relay")
target = external_secret.dig("spec", "target") || {}
unless target["creationPolicy"] == "Orphan" && target["deletionPolicy"] == "Retain"
  abort "the relay ExternalSecret must retain materialized data"
end
abort "the relay ExternalSecret embeds target data" if target.dig("template", "data") || target.dig("template", "stringData")
bound_keys = (external_secret.dig("spec", "data") || []).map { |entry| entry["secretKey"] }
unless bound_keys.to_set == SECRET_VARS.to_set
  abort "the relay ExternalSecret must bind exactly the four reviewed credentials"
end
if external_secret.dig("spec", "dataFrom")
  abort "the relay ExternalSecret must bind properties explicitly, not extract a whole record"
end

claim = fetch.call(relay, "PersistentVolumeClaim", "fkst-audit-relay-data", "audit-relay")
unless claim.dig("spec", "accessModes") == ["ReadWriteOnce"]
  abort "the audit outbox must be ReadWriteOnce; SQLite has one writer"
end
abort "the audit outbox claim must name a storage class explicitly" if claim.dig("spec", "storageClassName").to_s.empty?
abort "the audit outbox claim must request an explicit size" if claim.dig("spec", "resources", "requests", "storage").to_s.empty?

deployment = fetch.call(relay, "Deployment", "fkst-audit-relay", "audit-relay")
abort "the audit relay must run exactly one replica" unless deployment.dig("spec", "replicas") == 1
unless deployment.dig("spec", "strategy", "type") == "Recreate"
  abort "the audit relay must use Recreate; a surge replica would open the same database"
end
pod = deployment.dig("spec", "template", "spec") || {}
abort "the relay Pod must use its own ServiceAccount" unless pod["serviceAccountName"] == "fkst-audit-relay"
abort "the relay Pod must not mount an API token" unless pod["automountServiceAccountToken"] == false
grace = pod["terminationGracePeriodSeconds"].to_i
unless grace >= 60
  abort "the relay Pod needs a grace long enough for a SQLite checkpoint and worker shutdown"
end
pod_security = pod["securityContext"] || {}
unless pod_security["runAsNonRoot"] == true && pod_security["runAsUser"].to_i.positive? &&
       pod_security["runAsGroup"].to_i.positive? && pod_security["fsGroup"].to_i.positive? &&
       pod_security.dig("seccompProfile", "type") == "RuntimeDefault"
  abort "the relay Pod security context drifted from the restricted profile"
end

containers = pod["containers"] || []
abort "the relay Pod must run exactly one container" unless containers.length == 1
container = containers.first
security = container["securityContext"] || {}
unless security["allowPrivilegeEscalation"] == false &&
       security.dig("capabilities", "drop") == ["ALL"] &&
       security["readOnlyRootFilesystem"] == true
  abort "the relay container security context drifted from the restricted profile"
end
%w[requests limits].each do |half|
  %w[cpu memory].each do |resource|
    if container.dig("resources", half, resource).to_s.empty?
      abort "the relay container must declare #{half}.#{resource}"
    end
  end
end

# Probes must reach the relay's own unauthenticated ops endpoints. A credential
# in a probe URL would be readable in every Pod spec, every event, and every
# `kubectl describe`.
%w[startupProbe readinessProbe livenessProbe].each do |probe_name|
  probe = container[probe_name]
  abort "the relay container lacks #{probe_name}" unless probe.is_a?(Hash)
  http = probe["httpGet"]
  abort "#{probe_name} must probe the relay's own HTTP ops surface" unless http.is_a?(Hash)
  path = http["path"].to_s
  unless %w[/health /ready].include?(path)
    abort "#{probe_name} must use /health or /ready, not #{path}"
  end
  abort "#{probe_name} must not carry credentials in its URL" if path.include?("?") || path.include?("@")
  abort "#{probe_name} must not send request headers" if http["httpHeaders"]
end
unless container.dig("readinessProbe", "httpGet", "path") == "/ready"
  abort "relay readiness must track durable ingress, not process liveness"
end
unless container.dig("livenessProbe", "httpGet", "path") == "/health"
  abort "relay liveness must be dependency-free, or a PostHog outage restarts the outbox"
end

# Credentials arrive only as a whole-Secret reference; nothing is inlined.
env_from = container["envFrom"] || []
sources = env_from.map { |entry| entry.keys.first }.to_set
unless sources == Set["configMapRef", "secretRef"]
  abort "the relay must take configuration from one ConfigMap and one Secret"
end
(container["env"] || []).each do |entry|
  name = entry["name"].to_s
  if SECRET_VARS.include?(name) && !entry.dig("valueFrom", "secretKeyRef")
    abort "#{name} must arrive by secretKeyRef, never as an inline value"
  end
end
mounts = (container["volumeMounts"] || []).to_h { |mount| [mount["name"], mount["mountPath"]] }
unless mounts["audit-data"] == "/var/lib/fkst-audit"
  abort "the outbox volume must be mounted at /var/lib/fkst-audit"
end
volumes = (pod["volumes"] || []).to_h { |volume| [volume["name"], volume] }
unless volumes.dig("audit-data", "persistentVolumeClaim", "claimName") == "fkst-audit-relay-data"
  abort "the outbox volume must be backed by the reviewed claim"
end
if volumes.values.any? { |volume| volume.key?("secret") }
  abort "the relay must not project a Secret as a file volume"
end

service = fetch.call(relay, "Service", "fkst-audit-relay", "audit-relay")
abort "the relay Service must stay ClusterIP" unless service.dig("spec", "type") == "ClusterIP"
unless service.dig("spec", "selector") == { "app.kubernetes.io/name" => "fkst-audit-relay" }
  abort "the relay Service selector drifted"
end

budget = fetch.call(relay, "PodDisruptionBudget", "fkst-audit-relay", "audit-relay")
unless budget.dig("spec", "minAvailable") == 1
  abort "the relay PDB must prevent voluntary zero availability"
end

policy = fetch.call(relay, "NetworkPolicy", "fkst-audit-relay", "audit-relay")
unless policy.dig("spec", "podSelector", "matchLabels") == { "app.kubernetes.io/name" => "fkst-audit-relay" }
  abort "the relay NetworkPolicy selector drifted"
end
unless policy.dig("spec", "policyTypes").to_a.to_set == Set["Ingress", "Egress"]
  abort "the relay NetworkPolicy must cage both directions"
end
ingress_peers = (policy.dig("spec", "ingress") || []).flat_map { |rule| rule["from"] || [] }
unless ingress_peers.any? { |peer| peer.dig("podSelector", "matchLabels", "app.kubernetes.io/name") == "fkst-control-plane" }
  abort "the relay must admit the control plane"
end
unless ingress_peers.any? { |peer| peer.dig("namespaceSelector", "matchLabels", "fkst.chronoai.io/metrics-scraper") == "true" }
  abort "the relay must admit a labelled Prometheus namespace"
end
unless ingress_peers.length == 2
  abort "the relay must admit exactly the control plane and the metrics scraper"
end
egress_rules = policy.dig("spec", "egress") || []
dns = egress_rules.find do |rule|
  (rule["to"] || []).any? { |peer| peer.dig("namespaceSelector", "matchLabels", "kubernetes.io/metadata.name") == "kube-system" }
end
abort "the relay must be allowed cluster DNS" unless dns
blocked = egress_rules.flat_map { |rule| rule["to"] || [] }
                      .filter_map { |peer| peer.dig("ipBlock", "except") }
                      .flatten.to_set
%w[169.254.169.254/32 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16].each do |cidr|
  abort "relay egress must exclude #{cidr}" unless blocked.include?(cidr)
end

# ------------------------------------------------------- composed and base shape

base_config = fetch.call(base, "ConfigMap", "fkst-control-plane-config", "base")
base_mode = (base_config["data"] || {})["FKST_AUDIT_DELIVERY_MODE"].to_s
unless %w[disabled best_effort].include?(base_mode)
  abort "base must default to a non-production delivery mode, got #{base_mode.inspect}"
end

required_config = fetch.call(required, "ConfigMap", "fkst-control-plane-config", "required-audit")
required_data = required_config["data"] || {}
unless required_data["FKST_AUDIT_DELIVERY_MODE"] == "required"
  abort "the required-audit overlay must select required delivery"
end
unless required_data["FKST_AUDIT_RELAY_URL"].to_s.start_with?("http://", "https://")
  abort "required delivery needs a relay URL"
end
if required_data["FKST_AUDIT_RELAY_URL"].to_s.include?("@")
  abort "the relay URL must not embed userinfo credentials"
end
unless required_data["FKST_AUDIT_INCOMPLETE_GRACE_SECS"].to_s == relay_grace
  abort "the control plane and the relay must agree on FKST_AUDIT_INCOMPLETE_GRACE_SECS"
end
SECRET_VARS.each do |var|
  abort "#{var} must never be rendered into the control-plane ConfigMap" if required_data.key?(var)
end

# Two capture writers into one project. Asserted here as well as at startup
# because an overlay that turns both on also legitimises putting
# FKST_POSTHOG_PROJECT_TOKEN back into the control-plane record, which is the
# credential boundary the whole relay exists to draw (epic `OPS-02`).
if required_data["FKST_POSTHOG_ENABLED"].to_s == "true"
  abort "FKST_POSTHOG_ENABLED must stay false where the relay captures; two writers, one project"
end

# The activity READ path needs the host as well as the project id, and this is
# the composition where forgetting it is invisible: the control plane here does
# not capture, so nothing else would want a host. Without it /operations answers
# 503 forever.
if required_data["FKST_POSTHOG_PROJECT_ID"].to_s.empty?
  abort "the required-audit overlay must select a PostHog project id for the activity query"
end
if required_data["FKST_POSTHOG_HOST"].to_s.empty?
  abort "the required-audit overlay must set FKST_POSTHOG_HOST for the control plane's activity query"
end
check_posthog_host.call(required_data, "the control-plane ConfigMap's")

# The COMPOSED relay ConfigMap is where an environment overlay actually supplies
# the delivery host, so it is judged again after the patch — the standalone relay
# render legitimately carries no host at all.
required_relay_data = fetch.call(required, "ConfigMap", "fkst-audit-relay-config", "required-audit")["data"] || {}
if required_relay_data["FKST_POSTHOG_HOST"].to_s.empty?
  abort "the required-audit overlay must give the relay a PostHog delivery host"
end
check_posthog_host.call(required_relay_data, "the composed relay ConfigMap's")
SECRET_VARS.each do |var|
  abort "#{var} must never be rendered into the composed relay ConfigMap" if required_relay_data.key?(var)
end

# The control plane stays stateless: no claim, no outbox mount, no relay volume.
control_plane = fetch.call(required, "Deployment", "fkst-control-plane", "required-audit")
control_pod = control_plane.dig("spec", "template", "spec") || {}
if (control_pod["volumes"] || []).any? { |volume| volume.key?("persistentVolumeClaim") }
  abort "the control plane must not mount a persistent volume"
end
(control_pod["containers"] || []).each do |item|
  if (item["volumeMounts"] || []).any? { |mount| mount["mountPath"].to_s.start_with?("/var/lib/fkst-audit") }
    abort "the control plane must not mount the audit outbox"
  end
end

# The frontend is a static bundle served by nginx. It must receive no
# configuration at all, which is the strongest possible form of "no PostHog or
# relay credential reaches a browser" (epic `OPS-02`).
frontend = fetch.call(required, "Deployment", "fkst-frontend", "required-audit")
(frontend.dig("spec", "template", "spec", "containers") || []).each do |item|
  abort "the frontend must not consume configuration by envFrom" unless (item["envFrom"] || []).empty?
  (item["env"] || []).each do |entry|
    name = entry["name"].to_s
    if name.include?("POSTHOG") || name.include?("AUDIT_RELAY")
      abort "the frontend must never receive #{name}"
    end
  end
end

# Finally, a literal scan: no rendered document may carry a value that looks like
# one of the credentials, in any field, including annotations and arguments.
[ARGV.fetch(0), ARGV.fetch(1)].each do |path|
  text = File.read(path)
  text.each_line do |line|
    next unless SECRET_VARS.any? { |var| line.include?(var) }
    # A reference is fine (`secretKey:`, `property:`, a comment). A `KEY: value`
    # assignment carrying anything but an empty string is not.
    next unless line =~ /^\s+(#{SECRET_VARS.join('|')}):\s*(\S.*)$/
    abort "#{File.basename(path)} assigns a value to #{Regexp.last_match(1)}"
  end
end

puts "validated #{relay.length} audit-relay objects (no Ingress, no RBAC, no secret values)"
