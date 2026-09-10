#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $(basename "$0") [-n ROWS] OUTPUT_FILE" >&2
  echo "  -n ROWS   number of rows to generate (default: 50)" >&2
  exit 1
}

n=50
while getopts "n:h" opt; do
  case "$opt" in
    n) n="$OPTARG" ;;
    h) usage ;;
    *) usage ;;
  esac
done
shift $((OPTIND - 1))

out="${1:-}"
[[ -z "$out" ]] && usage

awk -v n="$n" 'BEGIN {
  print "key,tag,amount"
  for (i = 1; i <= n; i++) {
    printf "%064x,%06x,%d\n", i, i, i
  }
}' > "$out"

echo "wrote $n rows to $out"
