#!/usr/bin/env bash
# stdin: binary CBOR  ->  stdout: key,tag,amount,sig
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: cbor2csv.sh < input.cbor > output.csv
       curl -s <url> | cbor2csv.sh > output.csv

Reads binary CBOR (a map of byte-string keys to either null or
[amount, signature]) on stdin, writes CSV rows to stdout:

    key,tag,amount,signature

  key       first 32 bytes of the map-key, as 64 hex chars
  tag       any remaining bytes of the map-key, as hex (empty if none)
  amount    value[0] if the value is a 2-item array, else empty
  signature value[1] as hex if the value is a 2-item array, else empty

Requires cbor-diag-cli (the `cbor-diag` binary) on PATH.
No arguments are read; the script only consumes stdin.
EOF
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
    "") ;;
    *)
        usage
        exit 1
        ;;
esac

if ! command -v cbor-diag >/dev/null 2>&1; then
    echo "cbor2csv.sh: 'cbor-diag' not found on PATH (cargo install cbor-diag-cli)" >&2
    exit 1
fi

cbor-diag --to diag | awk '
    function extract_hex(line,    p, s) {
        p = index(line, "h'"'"'")
        if (!p) return ""
        s = substr(line, p + 2)
        return substr(s, 1, index(s, "'"'"'") - 1)
    }

    # h'"'"'....'"'"': null,
    /h'"'"'.*: null/ {
        key = extract_hex($0)
        print substr(key, 1, 64) "," substr(key, 65) ",,"
        next
    }

    # h'"'"'....'"'"': [
    /h'"'"'.*: \[/ {
        key = extract_hex($0)
        in_array = 1
        amount = ""
        sig = ""
        next
    }

    in_array && /^[ \t]*[0-9]+,?[ \t]*$/ {
        amount = $0
        gsub(/[^0-9]/, "", amount)
        next
    }

    in_array && /h'"'"'/ {
        sig = extract_hex($0)
        next
    }

    in_array && /\]/ {
        print substr(key, 1, 64) "," substr(key, 65) "," amount "," sig
        in_array = 0
    }
'
