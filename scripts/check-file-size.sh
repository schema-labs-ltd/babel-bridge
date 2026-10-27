#!/usr/bin/env bash
# Blocks commits that stage a hand-written text file over the token budget.
#
# Run by Lefthook as a pre-commit hook (see lefthook.yml). It checks only the
# staged version of each file, so unstaged edits never block or unblock a
# commit. Pass paths to check those staged files; with no arguments it checks
# every staged file.
#
# Skipped automatically:
#   - binary files (images, audio, video), as git detects them;
#   - files marked `linguist-generated` in .gitattributes (lockfiles, build
#     output, Xcode project files). Mark new generated files there, not here.
#
# Tokens are estimated as bytes / 4.

set -euo pipefail

MAX_TOKENS="${MAX_FILE_TOKENS:-5000}"
MAX_BYTES=$((MAX_TOKENS * 4))

cd "$(git rev-parse --show-toplevel)"

if [ "$#" -gt 0 ]; then
  files=("$@")
else
  files=()
  while IFS= read -r -d '' path; do
    files+=("$path")
  done < <(git diff --cached --name-only --diff-filter=ACMR -z)
fi

[ "${#files[@]}" -eq 0 ] && exit 0

# Staged files git sees as text; binary files (images, audio) are left out.
declare -A text=()
while IFS= read -r -d '' path; do
  text["$path"]=1
done < <(git grep --cached -I -l -z -e '' -- "${files[@]}" || true)

declare -A generated=()
while IFS= read -r -d '' path && IFS= read -r -d '' _attr && IFS= read -r -d '' value; do
  case "$value" in
    unspecified | unset | false) ;;
    *) generated["$path"]=1 ;;
  esac
done < <(git check-attr -z linguist-generated -- "${files[@]}")

oversized=()
for path in "${files[@]}"; do
  [ -n "${text[$path]:-}" ] || continue
  [ -n "${generated[$path]:-}" ] && continue
  # Skip paths with no staged blob (deleted or never added).
  bytes=$(git cat-file -s ":$path" 2>/dev/null) || continue
  if [ "$bytes" -gt "$MAX_BYTES" ]; then
    oversized+=("$(printf '%8d tokens  %s' $((bytes / 4)) "$path")")
  fi
done

[ "${#oversized[@]}" -eq 0 ] && exit 0

cat >&2 <<EOF
Commit blocked: these staged files are over the ${MAX_TOKENS}-token limit
(tokens = bytes / 4, so ${MAX_BYTES} bytes):

$(printf '  %s\n' "${oversized[@]}")

Large files are hard for agents and reviewers to read in one pass. Split each
file above into smaller files, each under ${MAX_BYTES} bytes, then stage them
and commit again. Do not just delete content or comments to squeeze under it.

How to split:

  1. Find the seams. Group the file's items (types, functions, handlers,
     tests) by the feature or responsibility they serve. Each group becomes a
     new file named after that responsibility, not part1/part2 or utils.
  2. Move whole items, unchanged. Make the move its own change: no renames or
     behaviour changes mixed in, so the diff is easy to review.
  3. Keep the public surface. Callers should not need to change their imports
     where the language allows re-exporting.
  4. Check each new file is under the limit, then run the build and tests.

By language in this repo:

  Rust (apps/server/src/)
                   Turn foo.rs into a module directory: keep foo.rs (or
                   foo/mod.rs) for the shared types and \`mod\` declarations,
                   and move each group into foo/<group>.rs. Re-export with
                   \`pub use\` so callers keep using crate::foo::Thing. Move
                   #[cfg(test)] mod tests into foo/tests.rs (or a
                   foo/tests/ directory). apps/server/src/storage/ and
                   apps/server/src/web/ follow this pattern.
  Swift (apps/ios/)
                   Move each group into an extension in its own file named
                   Type+Feature.swift next to the original. The Xcode project
                   does not pick up new files by itself: add each new file to
                   the same targets as the original in
                   apps/ios/WhatsAppTranslator.xcodeproj.
  JavaScript/TS    Move each group into its own ES module beside the file
  (apps/web/,      and import it from the original with a relative import,
  libs/)
                   the way apps/web/public/app.js imports ./voice-notes.js. Move
                   the matching tests in apps/web/tests alongside.
  Go (apps/wa-bridge/)
                   Move each group into a new .go file in the same directory
                   and package. No import changes are needed.
  CSS              Split by component or screen into separate files and link
                   or @import each one.
  Markdown/docs    Keep a short overview and move each section into its own
                   page under docs/, linking to it.

If the file is generated (a lockfile, build output, codegen) rather than
written by hand, mark it \`linguist-generated\` in .gitattributes instead of
splitting it.
EOF
exit 1
