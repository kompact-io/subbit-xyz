#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: ious_to_sub_steps.sh IOUS_CSV

Converts iou batch output (key,tag,amount,signature) to step.sh input
(iou_key,tag,kind,amount,signature,duration), kind fixed to "sub".
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" || -z "${1:-}" ]]; then
  usage
  exit "${1:+0}${1:-1}"
fi
ious="$1"

echo "iou_key,tag,kind,amount,signature,duration"
tail -n +2 "$ious" | awk -F, '$3 == "" { next } { print $1","$2",sub,"$3","$4","}'
