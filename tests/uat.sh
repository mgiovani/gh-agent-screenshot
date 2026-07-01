#!/usr/bin/env bash
set -euo pipefail
PASS=0; FAIL=0
pass() { echo "PASS: $1"; PASS=$((PASS+1)); }
fail() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }
grep -q "^name:" skills/gh-agent-screenshot/SKILL.md && pass "name field" || fail "name field"
grep -q "^description:" skills/gh-agent-screenshot/SKILL.md && pass "description field" || fail "description field"
grep -iEq "screen.capture|take.a.screenshot|screencap" skills/gh-agent-screenshot/SKILL.md && fail "capture wording" || pass "no capture wording"
for flag in --print-only --new-comment --update-comment --edit-body; do
  grep -q -- "$flag" skills/gh-agent-screenshot/SKILL.md && pass "$flag" || fail "$flag missing"
done
for flag in --dry-run --confirm --older-than-days; do
  grep -q -- "$flag" skills/gh-agent-screenshot/SKILL.md && pass "$flag" || fail "$flag missing"
done
grep -iq "private" skills/gh-agent-screenshot/SKILL.md && pass "private" || fail "private missing"
grep -iq "token" skills/gh-agent-screenshot/SKILL.md && pass "token" || fail "token missing"
gh skill publish --dry-run . 2>&1 | grep -q "Dry run complete" && pass "dry-run" || fail "dry-run failed"
RESULT=$(gh release view v0.1.0 --repo mgiovani/gh-agent-screenshot --json tagName,isDraft 2>/dev/null || echo "{}")
echo "$RESULT" | grep -q '"isDraft":false' && pass "not draft" || fail "draft or missing"
echo "$RESULT" | grep -q '"tagName":"v0.1.0"' && pass "tag v0.1.0" || fail "tag missing"
gh api repos/mgiovani/gh-agent-screenshot/topics 2>/dev/null | grep -q "agent-skills" && pass "topic" || fail "topic missing"
gh skill install mgiovani/gh-agent-screenshot --all 2>/dev/null && pass "install" || fail "install failed"
test -f .agents/skills/gh-agent-screenshot/SKILL.md && pass "SKILL.md installed" || fail "SKILL.md missing"
echo ""
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
