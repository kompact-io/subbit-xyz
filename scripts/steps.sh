#!/usr/bin/env bash
set -eo pipefail

usage() {
  cat <<'EOF'
usage: step.sh STEPS_CSV [BATCH_SIZE]

Drives `tx step` (and `tx submit` every BATCH_SIZE rows, default 50) from
a CSV of wants against staged channels, identified by (iou_key, tag).
iou_key is a keyring label, resolved to its VK via `keyring env`.

CSV columns (header row optional):
  iou_key,tag,kind,amount,signature,duration

  iou_key    keyring label (resolved to VK)
  tag        hex tag
  kind       add | sub | close | settle | end | elapse
  amount     used by add/sub/settle, blank otherwise
  signature  used by sub/settle (hex), blank otherwise
  duration   used by close/elapse, relative e.g. "210s", blank otherwise

Example rows:
  ALICE_IOU_1,000001,add,500,,
  ALICE_IOU_1,000001,sub,200,9f8e7d...,
  ALICE_IOU_1,000002,close,,,210s
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" || -z "${1:-}" ]]; then
  usage
  exit "${1:+0}${1:-1}"
fi
csv_path="$1"
batch_size="${2:-50}"

bin="cargo run --release --bin subbit-cli --"

upper() { echo "$1" | tr '[:lower:]' '[:upper:]'; }

render_want() {
  local kind="$1" amount="$2" signature="$3" duration="$4"
  case "$kind" in
    add)    printf '{"Add":{"amount":%s}}' "$amount" ;;
    sub)    printf '{"Sub":{"iou":{"amount":%s,"signature":"%s"}}}' "$amount" "$signature" ;;
    close)  printf '{"Close":{"upper":"%s"}}' "$duration" ;;
    settle) printf '{"Settle":{"iou":{"amount":%s,"signature":"%s"}}}' "$amount" "$signature" ;;
    end)    printf '"End"' ;;
    elapse) printf '{"Elapse":{"lower":"%s"}}' "$duration" ;;
    *) echo "unknown kind: $kind" >&2; exit 1 ;;
  esac
}

is_hex_key() {
  [[ "$1" =~ ^[0-9a-fA-F]{64}$ ]]
}

if is_hex_key "$iou_key"; then
  iou_key_hex="$iou_key"
else
  iou_key_var="$(upper "$iou_key")_VK"
  iou_key_hex="${!iou_key_var:-$iou_key}"
fi

source <($bin keyring env)

count=0
while IFS=, read -r iou_key tag kind amount signature duration; do
  [[ "$iou_key" == "iou_key" ]] && continue

  if is_hex_key "$iou_key"; then
    iou_key_hex="$iou_key"
  else
    iou_key_var="$(upper "$iou_key")_VK"
    iou_key_hex="${!iou_key_var:-$iou_key}"
  fi

  want=$(render_want "$kind" "$amount" "$signature" "$duration")
  $bin tx step --iou-key "$iou_key_hex" --tag "$tag" --want "$want"

  count=$((count + 1))
  if (( count % batch_size == 0 )); then
    $bin tx submit
  fi
done < "$csv_path"
