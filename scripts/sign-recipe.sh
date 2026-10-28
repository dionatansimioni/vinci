#!/bin/sh
# Signs data/recipe.toml after every change (bump `revision` first). The secret key never
# enters the repository. Usage:
#   scripts/sign-recipe.sh [path/to/vinci-recipes.key]
# Default key: ~/.minisign/vinci-recipes.key. Then run `cargo test` and commit both files.
set -eu
cd "$(dirname "$0")/.."
KEY="${1:-$HOME/.minisign/vinci-recipes.key}"
REV=$(sed -n 's/^revision = \([0-9]*\)$/\1/p' data/recipe.toml)
minisign -S -s "$KEY" -m data/recipe.toml -x data/recipe.toml.minisig -t "vinci recipe revision $REV"
minisign -V -p data/vinci-recipes.pub -m data/recipe.toml -x data/recipe.toml.minisig
