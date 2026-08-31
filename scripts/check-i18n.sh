#!/usr/bin/env bash
# Verify the message catalogues, and that no user-visible string escaped them.
#
# English is the source: every key exists in src/locales/en.json first, and the
# other catalogues are translations of it. A key missing from a translation is
# fine: vue-i18n falls back to English, so the key simply renders untranslated. A key that exists *only* in a translation is not fine: it
# is either a typo or a leftover, and nothing will ever render it.
#
# The second half is the regression barrier: a literal in a component is a
# string no translator can reach. Cyrillic is the cheap detector for it, since
# the interface used to be Russian and that is the language a contributor is
# most likely to hard-code again.
set -euo pipefail
# `comm` compares byte-wise; the key lists must be sorted the same way, so the
# whole script runs in the C locale.
export LC_ALL=C

locales_dir="src/locales"
source_catalogue="$locales_dir/en.json"
status=0

keys() { node -e '
  const flat = (o, p = "") =>
    Object.entries(o).flatMap(([k, v]) =>
      v && typeof v === "object" ? flat(v, p + k + ".") : [p + k + "\t" + v]);
  process.stdout.write(flat(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))).sort().join("\n") + "\n");
' "$1"; }

for file in "$locales_dir"/*.json; do
  node -e 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))' "$file" || {
    echo "!! $file is not valid JSON" >&2
    status=1
    continue
  }

  # A value that is present but empty renders as nothing at all, which is worse
  # than the English fallback an absent key would have given.
  if empty="$(keys "$file" | awk -F'\t' '$2 == ""  { print $1 }')" && [ -n "$empty" ]; then
    echo "!! $file has empty values:" >&2
    echo "$empty" | sed 's/^/     /' >&2
    status=1
  fi

  [ "$file" = "$source_catalogue" ] && continue

  if extra="$(comm -13 <(keys "$source_catalogue" | cut -f1) <(keys "$file" | cut -f1))" && [ -n "$extra" ]; then
    echo "!! $file has keys that are not in en.json:" >&2
    echo "$extra" | sed 's/^/     /' >&2
    status=1
  fi
  if missing="$(comm -23 <(keys "$source_catalogue" | cut -f1) <(keys "$file" | cut -f1))" && [ -n "$missing" ]; then
    printf '   %s is missing %d key(s); English will be shown for them\n' \
      "$(basename "$file")" "$(echo "$missing" | wc -l)"
  fi
done

# `src/i18n/index.ts` is exempt: it holds the languages' own names for
# themselves, which are deliberately never translated.
# The Cyrillic scan needs a UTF-8 locale of its own: under LC_ALL=C the range
# would be read byte-wise and would match half the em dashes in the comments.
if literals="$(LC_ALL=C.UTF-8 grep -rnP '[\x{0400}-\x{04FF}]' src --include='*.vue' --include='*.ts' \
    | grep -v '^src/locales/' | grep -v '^src/i18n/index.ts:' \
    | grep -v '^\s*//' | grep -vE '^[^:]+:[0-9]+:\s*(//|\*|/\*)')" && [ -n "$literals" ]; then
  echo "!! Cyrillic outside the catalogues; put these strings in src/locales/en.json:" >&2
  echo "$literals" | sed 's/^/     /' >&2
  status=1
fi

[ "$status" -eq 0 ] && echo "catalogues OK"
exit "$status"
