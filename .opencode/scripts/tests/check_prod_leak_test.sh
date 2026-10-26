#!/bin/sh
# Tests of .opencode/scripts/check_prod_leak.sh (ADR-0058).
#
# Every case runs in a throw-away git repository under a temporary directory,
# with a fake .env.prod made of invented values. No real value of the project
# .env.prod is used or read here.
#
# Usage: ./.opencode/scripts/tests/check_prod_leak_test.sh

set -u

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
script=$here/../check_prod_leak.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/crible-leak-test.XXXXXX")
runtmp=$work/tmp
mkdir "$runtmp"

cleanup() {
    chmod -R u+rwX "$work"
    rm -rf "$work"
}
trap cleanup EXIT

pass=0
fail=0

ok() {
    pass=$((pass + 1))
    printf 'ok %d - %s\n' "$pass" "$1"
}

ko() {
    fail=$((fail + 1))
    printf 'not ok - %s\n' "$1"
    if [ "$#" -gt 1 ]; then
        printf '  %s\n' "$2"
    fi
}

expect_status() { # label got expected
    if [ "$2" -eq "$3" ]; then
        ok "$1"
    else
        ko "$1" "expected exit $3, got $2"
    fi
}

expect_output() { # label got expected
    if [ "$2" = "$3" ]; then
        ok "$1"
    else
        ko "$1" "expected [$3], got [$2]"
    fi
}

expect_contains() { # label haystack needle
    case $2 in
        *"$3"*) ok "$1" ;;
        *) ko "$1" "expected [$3] in [$2]" ;;
    esac
}

expect_absent() { # label haystack needle
    case $2 in
        *"$3"*) ko "$1" "output contains [$3]" ;;
        *) ok "$1" ;;
    esac
}

# Invented values, shaped like the ones of .env.prod.example.
fake_env='PROD_SSH=fakeuser@fakehost.example
PROD_HOSTNAME=fakehost-other.example
PROD_REPO=/fake/path/to/repo
PROD_PATH=/fake/bin:/usr/bin
PROD_DB_CONTAINER=fakedb
PROD_PGOPTIONS="-c fake_option_for_tests=on"
PROD_SERVICES=""
PROD_FLAG=q7x
PROD_DAILY_WINDOW="01:23-04:56"'

new_repo() {
    dir=$(mktemp -d "$work/repo.XXXXXX")
    git -C "$dir" init -q
    git -C "$dir" config user.email test@example.invalid
    git -C "$dir" config user.name "Crible test"
    printf '%s\n' "$fake_env" >"$dir/.env.prod"
    printf 'nothing secret here\n' >"$dir/clean.txt"
    git -C "$dir" add clean.txt
    printf '%s\n' "$dir"
}

stage() { # dir text
    printf '%s\n' "$2" >>"$1/staged.txt"
    git -C "$1" add staged.txt
}

run_check() { # dir message
    (cd "$1" && TMPDIR=$runtmp "$script" "$2") 2>&1
}

run_check_args() { # dir message parts...
    dir=$1
    shift
    (cd "$dir" && TMPDIR=$runtmp "$script" "$@") 2>&1
}

# 1. Value in the staged diff: exit 1, name only, no value in the output.
repo=$(new_repo)
stage "$repo" "connect to fakeuser@fakehost.example please"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "staged diff leaks a value: exit 1" "$?" 1
expect_output "staged diff leaks a value: only the name is printed" "$out" "PROD_SSH"
expect_absent "staged diff leaks a value: the value is absent" "$out" "fakeuser@fakehost.example"

# 2. Value in the commit message only: same behaviour, with a clean diff.
repo=$(new_repo)
out=$(run_check "$repo" "chore(test): reach /fake/path/to/repo")
expect_status "commit message leaks a value: exit 1" "$?" 1
expect_output "commit message leaks a value: only the name is printed" "$out" "PROD_REPO"
expect_absent "commit message leaks a value: the value is absent" "$out" "/fake/path/to/repo"

# 2b. Value in the second argument only, with a clean diff: every argument of
# the message is checked.
repo=$(new_repo)
out=$(run_check_args "$repo" "chore(test): clean summary" "Task: reach /fake/path/to/repo")
expect_status "value in second argument: exit 1" "$?" 1
expect_output "value in second argument: only the name is printed" "$out" "PROD_REPO"
expect_absent "value in second argument: the value is absent" "$out" "/fake/path/to/repo"

# 3. Clean diff and clean message: exit 0 and no output.
repo=$(new_repo)
out=$(run_check "$repo" "chore(test): nothing to report")
expect_status "clean diff and message: exit 0" "$?" 0
expect_output "clean diff and message: no output" "$out" ""

# 4. Missing .env.prod: exit 2 and an explicit message.
repo=$(new_repo)
rm -f "$repo/.env.prod"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "missing .env.prod: exit 2" "$?" 2
expect_contains "missing .env.prod: explicit message" "$out" ".env.prod"

# 5. Unreadable .env.prod: exit 2 and an explicit message.
if [ "$(id -u)" -eq 0 ]; then
    printf 'skip - unreadable .env.prod (running as root)\n'
else
    repo=$(new_repo)
    chmod 000 "$repo/.env.prod"
    out=$(run_check "$repo" "chore(test): clean message")
    expect_status "unreadable .env.prod: exit 2" "$?" 2
    expect_contains "unreadable .env.prod: explicit message" "$out" "not readable"
    chmod 600 "$repo/.env.prod"
fi

# 6. Values between double quotes are recognised without their quotes.
repo=$(new_repo)
stage "$repo" "options: -c fake_option_for_tests=on"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "quoted value: exit 1" "$?" 1
expect_output "quoted value: only the name is printed" "$out" "PROD_PGOPTIONS"
expect_absent "quoted value: the value is absent" "$out" "fake_option_for_tests=on"

# 6b. No length threshold: a three-character value is compared like any other.
repo=$(new_repo)
stage "$repo" "flag is q7x here"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "short value: exit 1" "$?" 1
expect_output "short value: only the name is printed" "$out" "PROD_FLAG"
expect_absent "short value: the value is absent" "$out" "q7x"

# 7. Fixed-string comparison: a dot must not match another character.
repo=$(new_repo)
stage "$repo" "host fakehost-otherXexample"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "fixed-string comparison: exit 0" "$?" 0
expect_output "fixed-string comparison: no output" "$out" ""

# 8. The script writes nothing: TMPDIR stays empty.
repo=$(new_repo)
stage "$repo" "connect to fakeuser@fakehost.example please"
out=$(run_check "$repo" "chore(test): clean message")
expect_status "TMPDIR case: exit 1" "$?" 1
if [ -z "$(ls -A "$runtmp")" ]; then
    ok "TMPDIR stays empty after a run"
else
    ko "TMPDIR stays empty after a run" "found: $(ls -A "$runtmp")"
fi

# 9. Missing message argument: exit 2.
repo=$(new_repo)
out=$( (cd "$repo" && TMPDIR=$runtmp "$script") 2>&1 )
expect_status "missing argument: exit 2" "$?" 2
expect_contains "missing argument: explicit message" "$out" "argument"

# 10. Outside a git repository: exit 2.
outside=$(mktemp -d "$work/outside.XXXXXX")
out=$( (cd "$outside" && TMPDIR=$runtmp "$script" "chore(test): clean message") 2>&1 )
expect_status "outside a repository: exit 2" "$?" 2
expect_contains "outside a repository: explicit message" "$out" "repository"

printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
