#!/usr/bin/env bash
# publish_pages.sh <subpath|-> <src-dir> <git-url> <branch> <version> [--update CMD]
#
# Publishes <src-dir> to the Pages branch of one repository as a single orphan commit (force push guarded by a lease).
#   -        the whole repository is replaced by the contents of <src-dir> (OpenWrt and Keenetic feeds); the paths listed
#            in PAGES_KEEP (space separated, e.g. "linux/debian") are taken over from the old tree instead.
#   subpath  only this nested directory is replaced (e.g. linux/debian, the apt repository inside the dist repository);
#            everything else is carried over byte for byte.
# With --update CMD (subpath mode) the current <subpath> is exported to a temporary directory and "CMD <that-dir>
# <src-dir>" must leave the new contents there (apt keeps its pool and by-hash files this way); it runs on every attempt.
# An empty remote repository (no branch yet) is fine. A lease miss means somebody pushed meanwhile: the attempt is
# repeated on a fresh clone of the new head.
# DEPLOY_KEY_FILE: SSH private key used for the push (GIT_SSH_COMMAND is set from it).
# Test hook: TOWNJUMP_PAGES_BEFORE_PUSH is a shell command run between reading the branch and the push.
# Exit codes: 0 published, 1 failure (branch untouched), 2 bad arguments.
set -eu

usage() {
  echo "usage: publish_pages.sh <subpath|-> <src-dir> <git-url> <branch> <version> [--update CMD]" >&2
  exit 2
}

[ "$#" -eq 5 ] || [ "$#" -eq 7 ] || usage
subdir="$1" src="$2" url="$3" branch="$4" version="$5"
update=""
if [ "$#" -eq 7 ]; then
  [ "$6" = "--update" ] && [ -n "$7" ] || usage
  update="$7"
fi
safe_path() {
  case "$1" in "" | /* | . | .. | ../* | */.. | */../* | ./* | */./* | */. | */ | *//* | .git | .git/* | */.git | */.git/*) return 1 ;; esac
}
keep="${PAGES_KEEP:-}"
set -f # the keep list is split on spaces only, never expanded as a glob
if [ "$subdir" = "-" ]; then
  [ -z "$update" ] || usage
  for k in $keep; do safe_path "$k" || usage; done
else
  safe_path "$subdir" || usage
  [ -z "$keep" ] || usage
fi
if [ ! -d "$src" ] || [ -z "$(ls -A "$src")" ]; then
  echo "publish_pages.sh: source directory '$src' is missing or empty" >&2
  exit 2
fi
case "$src" in /*) ;; *) src="$PWD/$src" ;; esac
if [ -n "${DEPLOY_KEY_FILE:-}" ]; then
  export GIT_SSH_COMMAND="ssh -i '$DEPLOY_KEY_FILE' -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export GIT_DIR="$work/repo" GIT_INDEX_FILE="$work/index" TZ=UTC
export GIT_AUTHOR_NAME="Mesh Town" GIT_AUTHOR_EMAIL="340663027+mesh-town@users.noreply.github.com"
export GIT_COMMITTER_NAME="Mesh Town" GIT_COMMITTER_EMAIL="340663027+mesh-town@users.noreply.github.com"
when="$(date -u +%s) +0000"
export GIT_AUTHOR_DATE="$when" GIT_COMMITTER_DATE="$when"
git init -q --bare "$GIT_DIR"

remote_head() {
  git ls-remote --heads "$url" "refs/heads/$branch" | cut -f1
}

# Build the commit for one attempt; prints its id. $old is the branch head it is based on ("" if none).
build_commit() {
  local old="$1" tree="$work/tree" cur="$work/cur" head
  rm -rf "$tree" "$cur" "$GIT_INDEX_FILE"
  mkdir -p "$tree" "$cur"
  if [ -n "$old" ]; then
    git read-tree "$old" || return 1
    git --work-tree="$tree" checkout-index -a -f || return 1
  fi
  if [ "$subdir" = "-" ]; then
    rm -rf "${tree:?}" "$cur"
    mkdir -p "$tree" "$cur"
    for k in $keep; do
      if [ -n "$old" ] && git cat-file -e "$old:$k" 2>/dev/null; then
        mkdir -p "$tree/$(dirname "$k")"
        git archive "$old" "$k" | tar -x -C "$tree" || return 1
      fi
    done
    cp -R "$src/." "$tree/" || return 1
  else
    rm -rf "${tree:?}/$subdir"
    mkdir -p "$(dirname "$tree/$subdir")"
    if [ -n "$update" ]; then
      mkdir -p "$cur/$subdir"
      if [ -n "$old" ] && git cat-file -e "$old:$subdir" 2>/dev/null; then
        git archive "$old" "$subdir" | tar -x -C "$cur" || return 1
      fi
      env -u GIT_DIR -u GIT_INDEX_FILE sh -c "$update \"\$1\" \"\$2\"" sh "$cur/$subdir" "$src" >&2 || return 1
      cp -R "$cur/$subdir" "$tree/$subdir" || return 1
    else
      mkdir -p "$tree/$subdir"
      cp -R "$src/." "$tree/$subdir/" || return 1
    fi
  fi
  : > "$tree/.nojekyll"
  git --work-tree="$tree" add -A -f . || return 1
  head="$(git write-tree)" || return 1
  git -c commit.gpgsign=false commit-tree "$head" -m "townjump $version"
}

attempt=1
while [ "$attempt" -le 5 ]; do
  old=""
  if [ -n "$(remote_head)" ]; then
    git fetch -q "$url" "refs/heads/$branch"
    old="$(git rev-parse FETCH_HEAD)"
  fi
  if ! commit="$(build_commit "$old")"; then
    echo "publish_pages.sh: update command failed, branch not touched" >&2
    exit 1
  fi
  if [ -n "${TOWNJUMP_PAGES_BEFORE_PUSH:-}" ]; then
    env -u GIT_DIR -u GIT_INDEX_FILE sh -c "$TOWNJUMP_PAGES_BEFORE_PUSH" >&2 || { echo "publish_pages.sh: before-push hook failed" >&2; exit 1; }
  fi
  if git push -q "--force-with-lease=refs/heads/$branch:$old" "$url" "$commit:refs/heads/$branch" 2>"$work/push.err"; then
    echo "publish_pages.sh: published townjump $version to ${subdir}/ ($commit)"
    exit 0
  fi
  if [ "$(remote_head)" = "$old" ]; then
    cat "$work/push.err" >&2
    echo "publish_pages.sh: push failed and the branch did not change" >&2
    exit 1
  fi
  echo "publish_pages.sh: branch moved during attempt $attempt, retrying" >&2
  attempt=$((attempt + 1))
done
echo "publish_pages.sh: gave up after 5 lease failures, branch not touched by this run" >&2
exit 1
