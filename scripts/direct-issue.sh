#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: subbit-roundtrip.sh [PATH] [URL]

Stitches together: request a subbit -> spend it via curl -> decode the
response subbit for its balance.

  PATH  path to request the subbit for       (default: /)
  URL   endpoint to spend the subbit against (default: http://127.0.0.1:7822/v1/x/spend/echo)

Prints only the final "balance: N" line to stdout; progress goes to stderr.
Requires cargo (with the subbit-issuer binary) and curl on PATH.
EOF
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
esac

path="${1:-/}"
url="${2:-http://127.0.0.1:7822/v1/x/spend/echo}"

# 1. request a fresh subbit
token=$(cargo run --quiet --release --bin subbit-issuer -- request "$path")
if [[ -z "$token" ]]; then
    echo "subbit-roundtrip: 'request' produced no token" >&2
    exit 1
fi
echo "requested: $token" >&2

# 2. spend it: send as the subbit header, pull the new subbit back out
#    of the response headers (body is discarded, we only need the header)
headers=$(curl -sS -D - -o /dev/null "$url" -H "subbit: $token")
resp_token=$(printf '%s' "$headers" | tr -d '\r' | awk -F': ' 'tolower($1) == "subbit" { print $2 }')

if [[ -z "$resp_token" ]]; then
    status=$(printf '%s' "$headers" | head -n1 | tr -d '\r')
    echo "subbit-roundtrip: no subbit header in response ($status)" >&2
    exit 1
fi
echo "response: $resp_token" >&2

# 3. decode the response subbit to see the remaining balance
cargo run --quiet --release --bin subbit-issuer -- response "$resp_token"
