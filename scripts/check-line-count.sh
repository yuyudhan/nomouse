#!/usr/bin/env bash
# Fails when any tracked Rust source file exceeds the line limit (default 3000).
set -euo pipefail
limit="${1:-3000}"
cd "$(git rev-parse --show-toplevel)"
status=0
while IFS= read -r -d '' file; do
    lines=$(wc -l <"$file")
    if ((lines > limit)); then
        echo "line-count: $file has $lines lines (limit $limit)" >&2
        status=1
    fi
done < <(git ls-files -z --cached --others --exclude-standard -- '*.rs')
if ((status == 0)); then
    echo "line-count: ok (limit $limit)"
fi
exit "$status"
