#!/bin/sh
# Check that no value of .env.prod appears in the staged diff or in the commit
# message. See ADR-0058 (mechanism) and ADR-0032 (production access).
#
# Usage: ./.opencode/scripts/check_prod_leak.sh "<commit message>"
# Every argument is checked, so a message passed in several parts is covered.
#
# The script works on the git repository that contains the current directory.
# It reads .env.prod in memory only: it creates no file, uses no mktemp, tee,
# process substitution or here-document, and never enables set -x. It never
# prints a value nor the matching diff line, only the names of the variables.
#
# Exit codes:
#   0  no value found, the commit may proceed
#   1  at least one value found, the names are listed on stdout
#   2  check impossible (missing argument, no repository, .env.prod missing or
#      unreadable); this is not a success.

set -u

prog=check_prod_leak

# Fixed-string membership test: the pattern is quoted, so no character of the
# value is interpreted as a regular expression or as a glob.
contains_value() {
    case $1 in
        *"$2"*) return 0 ;;
    esac
    return 1
}

if [ "$#" -lt 1 ]; then
    printf '%s: missing commit message argument\n' "$prog" >&2
    exit 2
fi
# Every argument is checked, so a message split over several arguments (for
# example several -m parts) is fully covered.
message=$*

if ! root=$(git rev-parse --show-toplevel); then
    printf '%s: not inside a git repository\n' "$prog" >&2
    exit 2
fi

env_file=$root/.env.prod
if [ ! -e "$env_file" ]; then
    printf '%s: %s is missing, the check is impossible\n' "$prog" "$env_file" >&2
    exit 2
fi
if [ ! -f "$env_file" ] || [ ! -r "$env_file" ]; then
    printf '%s: %s is not readable, the check is impossible\n' "$prog" "$env_file" >&2
    exit 2
fi

if ! staged=$(git -C "$root" diff --cached); then
    printf '%s: git diff --cached failed, the check is impossible\n' "$prog" >&2
    exit 2
fi

found=
cr=$(printf '\r')

while IFS= read -r line || [ -n "$line" ]; do
    case $line in
        PROD_*=*) ;;
        *) continue ;;
    esac
    name=${line%%=*}
    case $name in
        *[!A-Z_]*) continue ;;
    esac
    value=${line#*=}
    value=${value%"$cr"}
    case $value in
        \"*\") value=${value#\"}; value=${value%\"} ;;
    esac
    [ -n "$value" ] || continue
    if contains_value "$staged" "$value" || contains_value "$message" "$value"; then
        found="$found$name
"
    fi
done < "$env_file"

if [ -n "$found" ]; then
    printf '%s' "$found"
    exit 1
fi
exit 0
