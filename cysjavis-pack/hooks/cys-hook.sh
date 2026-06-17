#!/bin/sh
# Claude Code/Codex 툴 이벤트 hook (cys-terminal T7 E1-④):
#   claude가 PreToolUse/PostToolUse/Stop/SubagentStop마다 실행하는 hook. stdin으로 받은 hook
#   JSON을 cysd에 push(usage.event)해 events 테이블에 툴·스킬·에이전트 호출과 exit_code를
#   적재한다(E3 스킬 TOP·반복실패 분석 토대). surface는 CYS_SURFACE_ID(에이전트 PTY 상속).
# ★불변: hook 경로는 **절대 에이전트를 막지 않는다** — PreToolUse에서 exit≠0/JSON 출력은 툴을
#   차단할 수 있으므로 금지. stdout 무출력·모든 실패 무해히 흘림·항상 exit 0.
IN=$(cat)
if [ -n "$IN" ] && command -v cys >/dev/null 2>&1; then
  printf '%s' "$IN" | cys usage-event-stdin >/dev/null 2>&1
fi
exit 0
