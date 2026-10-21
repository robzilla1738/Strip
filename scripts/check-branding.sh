#!/usr/bin/env bash
# Fail when product sources name the commercial app this project must not copy.
# Roadmap.md is the spec and names that rule. docs/DECISIONS.md records the exclusion.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
cd "$root"
# The forbidden name is assembled so this file does not contain it.
left=chain
mid=your
right=mac
compact="${left}${mid}${right}"
spaced="$left $mid $right"
if rg -i -n --hidden -g '!.git/**' -g '!Roadmap.md' -g '!docs/DECISIONS.md' -g '!build/**' -g '!target/**' -g '!**/target/**' -e "$compact" -e "$spaced" .; then
  echo "branding check failed" >&2
  exit 1
fi
echo "branding ok"
