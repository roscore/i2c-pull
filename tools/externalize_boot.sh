#!/usr/bin/env bash
# Trunk post_build hook: move Trunk's inline <script type="module"> bootstrap into boot.js
# so the strict CSP (script-src 'self', no 'unsafe-inline') allows it.
set -euo pipefail
dir="${TRUNK_STAGING_DIR:?run from trunk}"
html="$dir/index.html"
awk -v js="$dir/boot.js" -v src="${TRUNK_PUBLIC_URL:-./}boot.js" '
  !done && /<script type="module">/ { inside = 1; sub(/<script type="module">.*/, ""); printf "%s<script type=\"module\" src=\"%s\"></script>\n", $0, src; next }
  inside && /<\/script>/ { inside = 0; done = 1; sub(/.*<\/script>/, ""); if (length($0)) print; next }
  inside { print > js; next }
  { print }
' "$html" > "$html.tmp"
mv "$html.tmp" "$html"
if grep -q '<script type="module">' "$html"; then echo "inline script left in index.html" >&2; exit 1; fi
test -s "$dir/boot.js"
