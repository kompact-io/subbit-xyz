#!/usr/bin/env bash
#
# simulate-aiken-tx.sh
#
# Finds matching tx-dump-*-tx.txt / -inputs.txt / -outputs.txt triples
# in a directory and runs `aiken tx simulate` on each, e.g.:
#
#   aiken tx simulate tx-dump-<ID>-tx.txt tx-dump-<ID>-inputs.txt \
#       tx-dump-<ID>-outputs.txt --script-override HASH1:HASH2 \
#       --blueprint aiken/traces.json
#
# Usage:
#   ./simulate-aiken-tx.sh [-d DIR] [-b BLUEPRINT] [-s HASH1:HASH2]
#
#   -d DIR        Directory to search for tx-dump files (default: .)
#   -b BLUEPRINT  Path to the blueprint JSON (default: aiken/traces.json)
#   -s OVERRIDE   --script-override value. If omitted, it's built as
#                 62ce4309e37e09e5c633c96c6ae68061c434f122d32626d6912d7c2a:<hash>
#                 where <hash> = jq -r '.validators[0].hash' BLUEPRINT

set -euo pipefail

DUMP_DIR="."
BLUEPRINT="aiken/traces.json"
OVERRIDE=""

while getopts ":d:b:s:h" opt; do
    case "$opt" in
        d) DUMP_DIR="$OPTARG" ;;
        b) BLUEPRINT="$OPTARG" ;;
        s) OVERRIDE="$OPTARG" ;;
        h) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Invalid option" >&2; exit 1 ;;
    esac
done

command -v aiken >/dev/null || { echo "Error: 'aiken' not found on PATH." >&2; exit 1; }

if [[ -z "$OVERRIDE" ]]; then
    command -v jq >/dev/null || { echo "Error: 'jq' is required to derive --script-override (or pass -s)." >&2; exit 1; }
    [[ -f "$BLUEPRINT" ]] || { echo "Error: blueprint '$BLUEPRINT' not found." >&2; exit 1; }
    hash="$(jq -r '.validators[0].hash' "$BLUEPRINT")"
    [[ -n "$hash" && "$hash" != "null" ]] || { echo "Error: could not read .validators[0].hash from '$BLUEPRINT'." >&2; exit 1; }
    OVERRIDE="62ce4309e37e09e5c633c96c6ae68061c434f122d32626d6912d7c2a:${hash}"
fi

shopt -s nullglob
tx_files=("$DUMP_DIR"/tx-dump-*-tx.txt)
shopt -u nullglob
[[ ${#tx_files[@]} -gt 0 ]] || { echo "Error: no tx-dump-*-tx.txt files found in '$DUMP_DIR'." >&2; exit 1; }

status=0
for tx_file in "${tx_files[@]}"; do
    id="$(basename "$tx_file" | sed -E 's/^tx-dump-(.*)-tx\.txt$/\1/')"
    inputs_file="$DUMP_DIR/tx-dump-${id}-inputs.txt"
    outputs_file="$DUMP_DIR/tx-dump-${id}-outputs.txt"

    if [[ ! -f "$inputs_file" || ! -f "$outputs_file" ]]; then
        echo "Skipping ID '$id': missing inputs/outputs file." >&2
        continue
    fi

    echo "=== Simulating tx ID: $id ==="
    aiken tx simulate "$tx_file" "$inputs_file" "$outputs_file" \
        --script-override "$OVERRIDE" --blueprint "$BLUEPRINT" || status=1
done

exit "$status"
