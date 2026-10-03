{{/*
Chart name, truncated to the 63-character DNS label limit.
*/}}
{{- define "librefang.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified resource name. `helm install librefang …` yields `librefang`,
which is also the name the Kustomize reference uses.
*/}}
{{- define "librefang.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Name of the governing headless Service. Required by StatefulSet.spec.serviceName.
*/}}
{{- define "librefang.headlessName" -}}
{{- printf "%s-headless" (include "librefang.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "librefang.configMapName" -}}
{{- printf "%s-config" (include "librefang.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "librefang.agentsConfigMapName" -}}
{{- printf "%s-agents" (include "librefang.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "librefang.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "librefang.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "librefang.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Selector labels. Immutable once a StatefulSet exists: never add to this.
*/}}
{{- define "librefang.selectorLabels" -}}
app.kubernetes.io/name: {{ include "librefang.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "librefang.labels" -}}
{{- include "librefang.labelsFor" (dict "root" . "component" "daemon") -}}
{{- end }}

{{/*
Same labels for a non-daemon component. Usage:
  include "librefang.labelsFor" (dict "root" . "component" "config-validate")
*/}}
{{- define "librefang.labelsFor" -}}
helm.sh/chart: {{ include "librefang.chart" .root }}
{{ include "librefang.selectorLabels" .root }}
app.kubernetes.io/component: {{ .component }}
app.kubernetes.io/part-of: librefang
app.kubernetes.io/version: {{ .root.Values.image.tag | default .root.Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .root.Release.Service }}
{{- end }}

{{/*
Image reference. `image.digest`, when set, pins by content and the tag is kept
only as a human-readable hint (`repo:tag@sha256:…`).
*/}}
{{- define "librefang.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if .Values.image.digest -}}
{{- printf "%s:%s@%s" .Values.image.repository $tag .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end }}

{{/*
Hard stops that must hold even when the schema is bypassed with
`--skip-schema-validation`.
*/}}
{{- define "librefang.validate" -}}
{{- if ne (int .Values.replicas) 1 -}}
{{- fail (printf "replicas must be exactly 1 (got %v). LibreFang holds an exclusive flock on /data/daemon.lock and its cron, trigger dispatch, session ownership, budget enforcement and audit chain are process-local. See docs/architecture/multi-replica-rfc.md." .Values.replicas) -}}
{{- end -}}
{{- if not .Values.auth.existingSecret -}}
{{- fail "auth.existingSecret is required. The daemon binds 0.0.0.0 inside the pod and refuses to start on a non-loopback bind without configured authentication. The chart never creates Secrets; create one out of band (see deploy/helm/librefang/README.md)." -}}
{{- end -}}
{{- if and (not .Values.managedConfig.enabled) .Values.managedConfig.agents -}}
{{- fail "managedConfig.agents is set but managedConfig.enabled is false. Declarative agents are delivered by the managed-config ConfigMaps; enable managedConfig or remove the agents." -}}
{{- end -}}
{{- $reserved := list "LIBREFANG_LISTEN" "LIBREFANG_HOME" "LIBREFANG_API_KEY" "LIBREFANG_VAULT_KEY" "LIBREFANG_DASHBOARD_USER" "LIBREFANG_DASHBOARD_PASS" "LIBREFANG_STATE_SECRET" "LIBREFANG_CONFIG_PATH" "LIBREFANG_CONFIG_MODE" "LIBREFANG_PROVISIONING_PATH" "LIBREFANG_PROVISIONING_PRUNE" -}}
{{- $provider := dict -}}
{{- range .Values.providers.env -}}{{- $_ := set $provider .name true -}}{{- end -}}
{{- range .Values.extraEnv -}}
{{- if has .name $reserved -}}
{{- fail (printf "extraEnv %q collides with a variable the chart manages. Use the dedicated value instead (auth.*, managedConfig.*)." .name) -}}
{{- end -}}
{{- if hasKey $provider .name -}}
{{- fail (printf "extraEnv %q is also listed in providers.env; a duplicate env name is undefined behaviour in a pod spec." .name) -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Guards for `ingress` and `httpRoute`. Exposing the daemon is allowed, but never
half-configured: a route without hostnames also matches every other host on a
shared Gateway, and an Ingress without TLS would send the bearer token and the
dashboard password in clear text.
*/}}
{{- define "librefang.validateExposure" -}}
{{- if .Values.ingress.enabled -}}
{{- if not .Values.ingress.hosts -}}
{{- fail "ingress.enabled needs at least one entry in ingress.hosts." -}}
{{- end -}}
{{- if not .Values.ingress.allowWithoutTLS -}}
{{- if not .Values.ingress.tls -}}
{{- fail "ingress.enabled needs ingress.tls: the bearer token and dashboard password would cross the network in clear text. If TLS is terminated in front of the Ingress by something this chart cannot see, set ingress.allowWithoutTLS=true." -}}
{{- end -}}
{{- $covered := list -}}
{{- range .Values.ingress.tls -}}{{- range .hosts -}}{{- $covered = append $covered . -}}{{- end -}}{{- end -}}
{{- range .Values.ingress.hosts -}}
{{- if not (has .host $covered) -}}
{{- fail (printf "ingress host %q is not listed under any ingress.tls[].hosts entry, so it would be served without TLS." .host) -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- if .Values.httpRoute.enabled -}}
{{- if not .Values.httpRoute.parentRefs -}}
{{- fail "httpRoute.enabled needs httpRoute.parentRefs: the Gateway (and listener, via sectionName) to attach to." -}}
{{- end -}}
{{- if not .Values.httpRoute.hostnames -}}
{{- fail "httpRoute.enabled needs httpRoute.hostnames. A route without hostnames matches every host on the Gateway." -}}
{{- end -}}
{{- if .Values.httpRoute.httpRedirect.enabled -}}
{{- if not .Values.httpRoute.httpRedirect.parentRefs -}}
{{- fail "httpRoute.httpRedirect.enabled needs httpRoute.httpRedirect.parentRefs pointing at the Gateway's HTTP listener." -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Credential-shaped assignments in TOML shipped through a ConfigMap. A ConfigMap
is unencrypted in etcd and readable by anyone with `get configmaps`, so a
literal credential there is a leak. Mirrors SECRET_VALUE_KEYS in
scripts/check-k8s-manifests.py, which repeats the check over the rendered
output. `vault:` / `env:` values are indirections and are allowed; so are empty
values.

Usage: include "librefang.assertNoSecretValues" (dict "what" "…" "content" "…")
*/}}
{{- define "librefang.assertNoSecretValues" -}}
{{- $what := .what -}}
{{- range $line := regexFindAll `(?m)^[ \t]*(?:api_key|dashboard_pass|vault_key|state_secret|client_secret|password)[ \t]*=[ \t]*(?:"[^"]*"|'[^']*')` .content -1 -}}
{{- $field := regexReplaceAll `^[ \t]*([a-z_]+)[\s\S]*$` $line "${1}" -}}
{{- $value := regexReplaceAll `^[^=]*=[ \t]*["'](.*)["']$` $line "${1}" -}}
{{- if and $value (not (hasPrefix "vault:" $value)) (not (hasPrefix "env:" $value)) -}}
{{- fail (printf "%s assigns a literal value to %q. Credentials must not go through a ConfigMap; supply them from a Secret via auth.* / providers.* and reference the variable name with `*_env` in the config." $what $field) -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Reject TOML that does not parse. A StatefulSet terminates the running pod
before starting its replacement, so an unparseable config.toml would take the
working daemon down and leave a crash-looping one in its place. Catching it at
render time makes `helm upgrade` fail while the old pod is still serving.

Sprig's fromToml does not fail: it returns a map holding a single "Error" key.
A document whose only top-level key is literally `Error` is treated the same
way; that is the price of not shipping our own parser.

Usage: include "librefang.assertValidToml" (dict "what" "…" "content" "…")
*/}}
{{- define "librefang.assertValidToml" -}}
{{- $parsed := fromToml .content -}}
{{- if and (eq (len $parsed) 1) (hasKey $parsed "Error") -}}
{{- fail (printf "%s is not valid TOML: %s. The daemon would fail to boot and, because a StatefulSet stops the old pod first, take the running instance down with it." .what (index $parsed "Error")) -}}
{{- end -}}
{{- end }}

{{/*
The config.toml the ConfigMap carries: `managedConfig.config`, or the file
shipped in the chart. Normalised to exactly one trailing newline, because a
YAML `|` block scalar always ends that way and the checksum must be taken over
the bytes the daemon will actually read.
*/}}
{{- define "librefang.config" -}}
{{- $c := .Values.managedConfig.config | default (.Files.Get "files/config.toml") -}}
{{- if not (trim $c) -}}
{{- fail "managed config resolved to an empty config.toml: managedConfig.config is empty and files/config.toml is missing or empty in the chart. An empty file makes the daemon boot on compiled defaults (weaker exec_policy, no [reload] mode, no config_version)." -}}
{{- end -}}
{{- printf "%s\n" (regexReplaceAll `\s+$` $c "") -}}
{{- end }}

{{- define "librefang.agentContent" -}}
{{- printf "%s\n" (regexReplaceAll `\s+$` . "") -}}
{{- end }}

{{/*
Digest of the managed config, in the exact form the daemon reports as
`checksum` from GET /api/config/status: sha256 over the file's raw bytes.
*/}}
{{- define "librefang.configChecksum" -}}
{{- printf "sha256:%s" (sha256sum (include "librefang.config" .)) -}}
{{- end }}

{{/*
Digest of the provisioning tree: sha256 over `sha256sum` output for the agent
files in sorted key order. Matches check_provisioning_checksum in
scripts/check-k8s-manifests.py.
*/}}
{{- define "librefang.provisioningChecksum" -}}
{{- $manifest := "" -}}
{{- $agents := .Values.managedConfig.agents -}}
{{- range $key := (keys $agents | sortAlpha) -}}
{{- $manifest = printf "%s%s  %s\n" $manifest (sha256sum (include "librefang.agentContent" (index $agents $key))) $key -}}
{{- end -}}
{{- printf "sha256:%s" (sha256sum $manifest) -}}
{{- end }}

{{/*
Managed configuration and declarative provisioning landed in 2026.8.30. An
older image ignores LIBREFANG_CONFIG_PATH, runs `librefang init` against the
read-only mount and crash-loops. Only tags of the form YYYY.M.D are compared;
a digest-only or custom tag is the operator's responsibility.
*/}}
{{- define "librefang.validateManagedImage" -}}
{{- if .Values.managedConfig.enabled -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if regexMatch `^[0-9]+\.[0-9]+\.[0-9]+$` $tag -}}
{{- if not (semverCompare ">=2026.8.30" $tag) -}}
{{- fail (printf "managedConfig.enabled needs a LibreFang image >= 2026.8.30, but image.tag resolves to %s. Set image.tag to a newer release." $tag) -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end }}
