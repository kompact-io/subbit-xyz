#!/usr/bin/env bash
# Turn a base64url-encoded CBOR blob into human-readable diagnostic notation.
#
# Usage: b64cbor.sh [TOKEN]
#   TOKEN  base64url string to decode (read from stdin if omitted)
#
# Requires cbor-diag-cli (the `cbor-diag` binary) and base64 on PATH.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: b64cbor.sh [TOKEN]
       echo "$TOKEN" | b64cbor.sh

Decodes a base64url-encoded CBOR blob (no padding, '-'/'_' alphabet -
the style used by the subbit tokens) and prints it as CBOR diagnostic
notation via `cbor-diag --to diag`.

  TOKEN  the base64url string to decode; if omitted, read from stdin

Requires cbor-diag-cli (the `cbor-diag` binary) on PATH.
EOF
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
esac

if ! command -v cbor-diag >/dev/null 2>&1; then
    echo "b64cbor.sh: 'cbor-diag' not found on PATH (cargo install cbor-diag-cli)" >&2
    exit 1
fi

token="${1:-$(cat)}"

# base64url -> standard base64, then pad to a multiple of 4
token="${token//-/+}"
token="${token//_//}"
case $(( ${#token} % 4 )) in
    2) token="${token}==" ;;
    3) token="${token}=" ;;
esac

printf '%s' "$token" | base64 -d | cbor-diag --to diag
