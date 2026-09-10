#!/usr/bin/env bash
set -euo pipefail

csv_path="${1:?usage: $(basename "$0") OPENS_CSV}"

upper() { echo "$1" | tr '[:lower:]' '[:upper:]'; }

render_open() {
  local tag="$1" iou_key="$2" consumer="$3" provider="$4" close_period="$5" amount="$6"
  cat <<EOF
{
  "channel": {
    "constants": {
      "tag": "$tag",
      "currency": "Ada",
      "iou_key": "$iou_key",
      "consumer": "$consumer",
      "provider": "$provider",
      "close_period": $close_period
    },
    "variables": {
      "amount": $amount,
      "stage": {"Opened": {"subbed": 0}}
    }
  },
  "delegation": null
}
EOF
}

source <(cargo run --release --bin subbit-cli -- keyring env)

while IFS=, read -r iou_key tag consumer provider close_period amount; do
  [[ "$iou_key" == "iou_key" ]] && continue  # skip header row

  iou_key_var="$(upper "$iou_key")_VK"
  consumer_var="$(upper "$consumer")_VKH"
  provider_var="$(upper "$provider")_VKH"

  render_open "$tag" "${!iou_key_var}" "${!consumer_var}" "${!provider_var}" "$close_period" "$amount" \
    > /tmp/open.json

  cargo run --quiet --release --bin subbit-cli -- tx open --open @/tmp/open.json
done < "$csv_path"
