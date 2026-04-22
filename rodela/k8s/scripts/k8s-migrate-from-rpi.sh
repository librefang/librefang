#!/usr/bin/env bash
# =============================================================================
# k8s-migrate-from-rpi.sh
#
# Full migration from RPi user-systemd deployment to Kubernetes.
# Run from the worktree root. Requires: ssh access to the RPi,
# kubectl configured for the target cluster, python3.
#
# Usage:
#   ./deploy/k8s/scripts/k8s-migrate-from-rpi.sh [--dry-run] [--skip-ssh]
#
# Flags:
#   --dry-run   Print steps without executing destructive operations.
#   --skip-ssh  Skip RPi SSH steps (WAL checkpoint + tar). Assumes the
#               tarball already exists at $TARBALL or you will supply it.
#
# Note: run `chmod +x deploy/k8s/scripts/k8s-migrate-from-rpi.sh` once
#       after cloning / pulling this file.
# =============================================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
RPI_HOST="blitz@192.168.1.144"
RPI_DATA="/home/blitz/.librefang"
TARBALL="/tmp/librefang-rpi-$(date +%Y%m%dT%H%M%S).tar.gz"
SCRIPT_DIR="$REPO_ROOT/deploy/k8s/scripts"
K8S_DIR="$REPO_ROOT/deploy/k8s"

# ---------------------------------------------------------------------------
# Parse flags
# ---------------------------------------------------------------------------
DRY_RUN=false
SKIP_SSH=false
for arg in "$@"; do
  case $arg in
    --dry-run) DRY_RUN=true ;;
    --skip-ssh) SKIP_SSH=true ;;
  esac
done

echo "============================================================"
echo "  LibreFang: RPi -> Kubernetes migration"
echo "  REPO_ROOT : $REPO_ROOT"
echo "  RPI_HOST  : $RPI_HOST"
echo "  TARBALL   : $TARBALL"
echo "  DRY_RUN   : $DRY_RUN"
echo "  SKIP_SSH  : $SKIP_SSH"
echo "============================================================"
echo ""

# ---------------------------------------------------------------------------
# Step 1: WAL checkpoint
# ---------------------------------------------------------------------------
if ! $SKIP_SSH && ! $DRY_RUN; then
  echo "==> Step 1: Checkpointing SQLite WAL files on RPi..."
  ssh "$RPI_HOST" "
    for db in $RPI_DATA/data/*.db; do
      [ -f \"\$db\" ] || continue
      echo \"Checkpointing \$db\"
      sqlite3 \"\$db\" 'PRAGMA wal_checkpoint(TRUNCATE);'
    done
  "
else
  echo "==> Step 1: Skipped (SKIP_SSH=$SKIP_SSH DRY_RUN=$DRY_RUN)"
fi

# ---------------------------------------------------------------------------
# Step 2: Tar from RPi
# ---------------------------------------------------------------------------
if ! $SKIP_SSH && ! $DRY_RUN; then
  echo "==> Step 2: Archiving RPi data tree..."
  ssh "$RPI_HOST" "tar -C '$RPI_DATA' -cf - \
    config.toml aliases.toml channels providers integrations plugins \
    skills workflows data workspaces cron_jobs.json hand_state.json \
    sessions.json workflow_runs.json message_journal.jsonl vault.enc \
    .env secrets.env .omc 2>/dev/null || true" \
    | gzip -9 > "$TARBALL"
  echo "    Saved: $TARBALL ($(du -sh "$TARBALL" | cut -f1))"
else
  echo "==> Step 2: Skipped (SKIP_SSH=$SKIP_SSH DRY_RUN=$DRY_RUN)"
fi

# ---------------------------------------------------------------------------
# Step 3: Extract secrets
# ---------------------------------------------------------------------------
echo "==> Step 3: Extracting secrets to $K8S_DIR/secrets.env ..."
if ! $DRY_RUN; then
  python3 "$SCRIPT_DIR/k8s-extract-secrets.py" "$TARBALL" > "$K8S_DIR/secrets.env"
  chmod 600 "$K8S_DIR/secrets.env"
  echo "    secrets.env written ($(wc -l < "$K8S_DIR/secrets.env") keys)"
else
  echo "    [dry-run] Would run: python3 k8s-extract-secrets.py $TARBALL > $K8S_DIR/secrets.env"
fi

# ---------------------------------------------------------------------------
# Step 4: Sanitize config
# ---------------------------------------------------------------------------
echo "==> Step 4: Extracting sanitized config to $K8S_DIR/config/ ..."
if ! $DRY_RUN; then
  python3 "$SCRIPT_DIR/k8s-sanitize-config.py" "$TARBALL" "$K8S_DIR/config/"
  echo "    Config files written:"
  find "$K8S_DIR/config" -type f | sort | sed 's/^/      /'
else
  echo "    [dry-run] Would run: python3 k8s-sanitize-config.py $TARBALL $K8S_DIR/config/"
fi

# ---------------------------------------------------------------------------
# Step 5: Git diff (always run — safe to show in dry-run too)
# ---------------------------------------------------------------------------
echo "==> Step 5: Review config changes (safe to commit):"
cd "$REPO_ROOT"
git diff --stat deploy/k8s/config/ 2>/dev/null || true
git status --short deploy/k8s/config/ 2>/dev/null || true
echo ""
echo "    Review the diff above. secrets.env is gitignored and must NOT be committed."

# ---------------------------------------------------------------------------
# Step 6: Dry-run apply
# ---------------------------------------------------------------------------
echo "==> Step 6: kubectl dry-run..."
if ! $DRY_RUN; then
  kubectl apply -k "$K8S_DIR" --dry-run=client
else
  echo "    [dry-run] Would run: kubectl apply -k $K8S_DIR --dry-run=client"
fi

# ---------------------------------------------------------------------------
# Step 7: Apply manifests, seed PVC, scale back up
# ---------------------------------------------------------------------------
echo "==> Step 7: Seeding data into PVC..."
if ! $DRY_RUN; then
  echo "    Applying manifests..."
  kubectl apply -k "$K8S_DIR"

  echo "    Waiting for PVC to bind..."
  kubectl -n librefang wait --for=jsonpath='{.status.phase}'=Bound \
    pvc/librefang-data-librefang-0 --timeout=120s 2>/dev/null || \
    kubectl -n librefang get pvc librefang-data-librefang-0

  echo "    Scaling StatefulSet to 0 to free PVC for seeding..."
  kubectl -n librefang scale statefulset librefang --replicas=0
  kubectl -n librefang wait --for=delete pod/librefang-0 --timeout=60s 2>/dev/null || true

  echo "    Seeding PVC from tarball..."
  kubectl run "librefang-seed-$$" \
    --rm -i --restart=Never \
    --image=ubuntu:24.04 \
    --namespace=librefang \
    --overrides="{
      \"spec\":{
        \"volumes\":[{\"name\":\"data\",\"persistentVolumeClaim\":{\"claimName\":\"librefang-data-librefang-0\"}}],
        \"containers\":[{
          \"name\":\"seed\",
          \"image\":\"ubuntu:24.04\",
          \"stdin\":true,
          \"tty\":false,
          \"volumeMounts\":[{\"name\":\"data\",\"mountPath\":\"/data\"}],
          \"command\":[\"/bin/bash\",\"-c\",\"tar -C /data -xzf -\"]
        }]
      }
    }" < "$TARBALL"

  echo "    Scaling StatefulSet back to 1..."
  kubectl -n librefang scale statefulset librefang --replicas=1
else
  echo "    [dry-run] Would apply manifests, scale down, seed PVC, scale back up."
fi

# ---------------------------------------------------------------------------
# Step 8: Wait for pod ready
# ---------------------------------------------------------------------------
echo "==> Step 8: Waiting for pod to be ready..."
if ! $DRY_RUN; then
  kubectl -n librefang wait --for=condition=ready pod/librefang-0 --timeout=180s
  echo ""
  echo "==> Migration complete. Verify:"
  echo "    kubectl -n librefang port-forward svc/librefang 4545:4545 &"
  echo "    curl http://127.0.0.1:4545/api/health"
  echo "    curl http://127.0.0.1:4545/api/agents | jq '.[].id'"
else
  echo "    [dry-run] Would wait for pod/librefang-0 condition=ready."
  echo ""
  echo "==> Dry-run complete. Re-run without --dry-run to execute."
fi
