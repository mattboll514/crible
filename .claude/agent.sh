#!/bin/sh
# Runs one Crible agent headlessly in opencode (DeepSeek V4.1 Flash via Ollama) and prints
# only its final report, so the orchestrator's context stays small (ADR-0033).
#
# Usage: .claude/agent.sh <agent> [--session <id>] [message...]
#   Without a message argument, the message is read from stdin (use a heredoc for long ones).
#   --session continues an earlier session, e.g. @dev after a review round.
# The full event log goes to .claude/runs/ (gitignored); the session is also listed in opencode.
set -eu

usage() {
	echo "usage: .claude/agent.sh <agent> [--session <id>] [message...]" >&2
	exit 2
}

[ $# -ge 1 ] || usage
agent=$1
shift
session=
if [ "${1:-}" = "--session" ]; then
	[ $# -ge 2 ] || usage
	session=$2
	shift 2
fi
if [ $# -gt 0 ]; then message="$*"; else message=$(cat); fi
[ -n "$message" ] || usage

cd "$(dirname "$0")/.."
mkdir -p .claude/runs
log=".claude/runs/$(date +%Y%m%d-%H%M%S)-$agent.jsonl"
title="$agent: $(printf '%s' "$message" | tr '\n' ' ' | cut -c1-60)"
start=$(date +%s)

# --auto approves every permission that is not explicitly denied: nobody can answer a prompt
# in a headless run, so the deny rules of ADR-0026 are the guard.
set -- run --auto --agent "$agent" --format json --title "$title"
if [ -n "$session" ]; then set -- "$@" --session "$session"; fi
status=0
opencode "$@" "$message" >"$log" 2>"$log.err" || status=$?
[ -s "$log.err" ] || rm -f "$log.err"

elapsed=$(($(date +%s) - start))
sid=$(jq -r '.sessionID // empty' "$log" 2>/dev/null | head -n 1)
# The report is the text of the last step (the last step_start onwards).
report=$(jq -rs '([.[].type] | indices("step_start") | last // 0) as $i
	| .[$i:] | map(select(.type == "text") | .part.text) | join("\n")' "$log" 2>/dev/null || true)
denied=$(jq -r 'select(.type == "tool_use" and ((.part.state.error // "") | startswith("Permission denied")))
	| "refused: \(.part.tool) \(.part.state.input.command // .part.state.input.path // "")"' "$log" 2>/dev/null || true)
errors=$(jq -c 'select(.type == "error")' "$log" 2>/dev/null | cut -c1-300 || true)

printf '%s\n' "$report"
echo
[ -z "$denied" ] || printf '%s\n' "$denied"
[ -z "$errors" ] || printf 'error: %s\n' "$errors"
echo "--- agent=$agent session=${sid:-?} duration=$((elapsed / 60))m$((elapsed % 60))s exit=$status log=$log"

if [ "$status" -ne 0 ] || [ -z "$report" ]; then
	echo "FAILED: no final report. Last lines of $log.err:" >&2
	tail -n 20 "$log.err" >&2 2>/dev/null || true
	exit 1
fi
