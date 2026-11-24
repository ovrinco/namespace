#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$root/template/." "$work/"
checker=${OVRIN_NS_BIN:-ovrin-ns}
"$checker" --root "$work" check "$work/04_products/example" --class core --json > "$work/valid.json"
printf '# Atlass\n' > "$work/04_products/example/README.md"
if "$checker" --root "$work" check "$work/04_products/example" --class core --json > "$work/invalid.json"; then
 echo 'Expected invalid spelling to fail' >&2
 exit 1
fi
grep -q 'Atlas' "$work/invalid.json"
printf 'Valid example passed; invalid spelling was reported.\n'
