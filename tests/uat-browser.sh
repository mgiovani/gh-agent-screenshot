#!/usr/bin/env bash
# Browser-based UAT — runs via agent-browser CLI.
# Usage: bash tests/uat-browser.sh
# Requirements: agent-browser installed (npx agent-browser or local bin)
set -euo pipefail

PASS=0; FAIL=0
SESSION="uat-browser-$$"

pass() { echo "PASS: $1"; PASS=$((PASS+1)); }
fail() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }
cleanup() { agent-browser --session "$SESSION" close 2>/dev/null || true; }
trap cleanup EXIT

# Release page is publicly visible
agent-browser --session "$SESSION" open \
  https://github.com/mgiovani/gh-agent-screenshot/releases/tag/v0.1.0
agent-browser --session "$SESSION" wait --load networkidle
BODY=$(agent-browser --session "$SESSION" get text body 2>/dev/null || echo "")
echo "$BODY" | grep -q "v0.1.0" \
  && echo "$BODY" | grep -qi "skill" \
  && pass "release page loads and contains v0.1.0 + skill reference" \
  || fail "release page missing expected content"
agent-browser --session "$SESSION" close 2>/dev/null || true

# Repo topics include agent-skills
agent-browser --session "$SESSION" open https://github.com/mgiovani/gh-agent-screenshot
agent-browser --session "$SESSION" wait --load networkidle
TOPICS=$(agent-browser --session "$SESSION" eval \
  'Array.from(document.querySelectorAll("[data-testid=\"topic-tag\"], a.topic-tag, a[href*=\"topic%3Aagent-skills\"]")).map(el => el.textContent.trim()).join(",")' \
  2>/dev/null || echo "")
echo "$TOPICS" | grep -q "agent-skills" \
  && pass "agent-skills topic chip visible in repo sidebar" \
  || fail "agent-skills topic not found in DOM (topics: $TOPICS)"
agent-browser --session "$SESSION" close 2>/dev/null || true

# SKILL.md renders at the tagged ref
agent-browser --session "$SESSION" open \
  https://github.com/mgiovani/gh-agent-screenshot/blob/v0.1.0/skills/gh-agent-screenshot/SKILL.md
agent-browser --session "$SESSION" wait --load networkidle
SKILL=$(agent-browser --session "$SESSION" get text body 2>/dev/null || echo "")
echo "$SKILL" | grep -q "print-only" \
  && echo "$SKILL" | grep -qi "gh.agent.screenshot\|gh-agent-screenshot" \
  && pass "SKILL.md renders with --print-only and tool name" \
  || fail "SKILL.md page missing expected content"
agent-browser --session "$SESSION" close 2>/dev/null || true

echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
