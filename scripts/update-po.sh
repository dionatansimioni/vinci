#!/bin/sh
# Regenerates po/vinci.pot from the app sources and merges it into every language.
# xgettext < 0.24 has no Rust parser; C mode works for our plain tr("...") calls.
# crates/app/tests/translations.rs is the real guard: it fails on any missing string.
set -e
cd "$(dirname "$0")/.."
ls crates/app/src/*.rs > po/POTFILES
xgettext --language=C --keyword= --keyword=tr --from-code=UTF-8 \
    --package-name=vinci --msgid-bugs-address=https://github.com/dionatansimioni/vinci/issues \
    -f po/POTFILES -o po/vinci.pot 2>/dev/null
for lang in $(grep -v '^#' po/LINGUAS); do
    msgmerge --quiet --update --backup=none "po/$lang.po" po/vinci.pot
done
echo "Updated po/vinci.pot and $(grep -vc '^#' po/LINGUAS) translation(s)."
