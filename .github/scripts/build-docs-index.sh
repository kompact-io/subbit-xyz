#!/usr/bin/env bash
# Writes a landing index.html into a `cargo doc` output directory,
# listing every crate/binary it actually documented. `cargo doc`
# itself never generates a root index for a workspace, only one
# `<name>/index.html` per documented target (and which names show up
# depends on which crates have a lib vs only bin targets), so this is
# generated from what's really there rather than a hardcoded list,
# and never drifts as crates are added or renamed.

set -euo pipefail

doc_dir="${1:-target/doc}"
index="$doc_dir/index.html"

items=""
for dir in "$doc_dir"/*/; do
  name="$(basename "$dir")"
  [ -f "$dir/index.html" ] || continue
  label="${name//_/-}"
  items+="<li><a href=\"./$name/index.html\"><code>$label</code></a></li>\n"
done
items="$(printf '%b' "$items" | sort)"

cat >"$index" <<HTML
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>subbit-xyz docs</title>
<style>
body { font-family: sans-serif; max-width: 40rem; margin: 3rem auto; line-height: 1.5; padding: 0 1rem; color: #222; }
h1 { font-size: 1.4rem; }
ul { padding-left: 1.2rem; }
a { color: #0550ae; text-decoration: none; }
a:hover { text-decoration: underline; }
code { background: #eee; padding: 0 0.3rem; border-radius: 3px; }
</style>
</head>
<body>
<h1>subbit-xyz docs</h1>
<p>Generated rustdoc for every workspace crate and binary with a doc target.</p>
<ul>
$items
</ul>
</body>
</html>
HTML
