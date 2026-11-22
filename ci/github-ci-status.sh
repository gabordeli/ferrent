#!/usr/bin/env bash
# Reports one GitHub Actions run of the mirror: the conclusion, the job matrix
# and the tail of every failed job's log.
#
#   ci/github-ci-status.sh [owner/repo] [run_id]
#
# Without a run id, the newest run on main is used. The token comes from
# GITHUB_TOKEN, or from ~/.config/ferrent/github.pat (override the path with
# GITHUB_PAT_FILE). That PAT only needs Actions: read for this script.
set -euo pipefail

repo=${1:-gabordeli/ferrent}
run=${2:-}
pat_file=${GITHUB_PAT_FILE:-$HOME/.config/ferrent/github.pat}
token=${GITHUB_TOKEN:-$(tr -d '\n\r ' <"$pat_file" 2>/dev/null || true)}
if [ -z "$token" ]; then
    echo "no GitHub token: set GITHUB_TOKEN or create $pat_file" >&2
    exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

api() {
    curl -fsSL -H "Authorization: Bearer $token" \
        -H 'Accept: application/vnd.github+json' "$@"
}

if [ -z "$run" ]; then
    run=$(api "https://api.github.com/repos/$repo/actions/runs?branch=main&per_page=1" |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["workflow_runs"][0]["id"])')
fi

api "https://api.github.com/repos/$repo/actions/runs/$run" -o "$work/run.json"
api "https://api.github.com/repos/$repo/actions/runs/$run/jobs?per_page=100" -o "$work/jobs.json"

python3 - "$work" <<'PY'
import json
import sys

run = json.load(open(sys.argv[1] + "/run.json"))
print(f"{run['name']} #{run['run_number']}  {run['head_branch']}  "
      f"{run['head_sha'][:8]}  {run['status']}/{run['conclusion']}")
print(run["html_url"])

jobs = json.load(open(sys.argv[1] + "/jobs.json"))["jobs"]
failed = []
for job in jobs:
    print(f"  {job['conclusion'] or job['status']:<10} {job['name']}")
    if job["conclusion"] == "failure":
        failed.append((job["id"], job["name"]))

with open(sys.argv[1] + "/failed", "w") as out:
    for job_id, name in failed:
        out.write(f"{job_id}\t{name}\n")
PY

while IFS=$'\t' read -r job_id name; do
    [ -n "$job_id" ] || continue
    echo
    echo "===== $name ($job_id)"
    api "https://api.github.com/repos/$repo/actions/jobs/$job_id/logs" -o "$work/$job_id.log"
    # The log ends with the runner cleanup; the failure is before that.
    awk '/Post job cleanup/{exit} {print}' "$work/$job_id.log" >"$work/$job_id.trim"
    grep -E 'error\[E[0-9]+\]|error:|panicked at|FAILED|not found|RPC |##\[error\]' "$work/$job_id.trim" |
        sed -E 's/^[^ ]*Z //' | head -25 || true
    echo "--- tail"
    tail -15 "$work/$job_id.trim"
done <"$work/failed"
