#!/usr/bin/env bash
# Files every issue of this directory on GitHub with the gh CLI.
# Usage: docs/issues/file-issues.sh [owner/repo]
set -euo pipefail
cd "$(dirname "$0")"
repo=${1:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}
for f in [0-9][0-9]-*.md; do
  title=$(sed -n 's/^title: //p' "$f" | head -1)
  labels=$(sed -n 's/^labels: //p' "$f" | head -1 | tr -d ' ')
  body=$(awk 'BEGIN{n=0} /^---$/{n++; next} n>=2' "$f")
  echo "filing: $title"
  gh issue create --repo "$repo" --title "$title" --label "$labels" --body "$body"
done
