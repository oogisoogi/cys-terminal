#!/bin/bash
# min_binary 정책 공식 (해법③⑤ · 오너 승인 2026-07-14)
#   min_binary = max( security_floor , 지원창 하한(최신 본체에서 SUPPORT_WINDOW 세대 전) )
# - security_floor: security-floor.txt 첫 비주석 줄(래칫 — 내리기 금지)
# - SUPPORT_WINDOW: 본체 태그 세대 수(기본 99 = 사실상 floor 단독 = 현행 유지).
#   ★N-2로 조이는 것은 오너 결정: 2026-07-14 현재 오너 기계 본체가 0.12.50이라 즉시 N-2 적용 시
#   자체 함대가 팩 레인에서 차단된다 — 함대가 0.12.57+로 정리된 후 SUPPORT_WINDOW=2 활성 권장.
# 출력: 마지막 줄 = 확정 min_binary (CI가 캡처). 그 위 줄들 = 판정 근거(가시화).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

FLOOR=$(grep -v '^#' security-floor.txt | grep -m1 . | tr -d '[:space:]')
test -n "$FLOOR" || { echo "::error::security-floor.txt에 유효 버전 없음" >&2; exit 1; }

WINDOW="${SUPPORT_WINDOW:-99}"
# 본체 태그만(v* 중 pack-v 제외) 내림차순. mapfile 금지 — macOS bash 3.2 이식성(로컬 lane-check 공용).
BINS=$(git tag --list 'v*' --sort=-v:refname | grep -Ev '^pack-' || true)
COUNT=$(printf '%s\n' "$BINS" | grep -c . || true)
nth() { printf '%s\n' "$BINS" | sed -n "$1p"; }  # 1-기반
if [ "$COUNT" -eq 0 ]; then
  echo "본체 태그 없음 — floor 단독 적용" >&2
  WINDOW_FLOOR="$FLOOR"
else
  IDX=$(( WINDOW + 1 )); [ "$IDX" -gt "$COUNT" ] && IDX="$COUNT"
  N2=$(( 3 > COUNT ? COUNT : 3 ))
  WINDOW_FLOOR="$(nth "$IDX" | sed 's/^v//')"
  STRICT_N2="$(nth "$N2" | sed 's/^v//')"
  LATEST="$(nth 1 | sed 's/^v//')"
  echo "정책 근거: security_floor=$FLOOR · 최신 본체=$LATEST · 지원창(${WINDOW}세대)=$WINDOW_FLOOR · (참고: N-2 적용 시=$STRICT_N2)" >&2
fi

# max(FLOOR, WINDOW_FLOOR) — semver 비교는 sort -V
MIN_BINARY=$(printf '%s\n%s\n' "$FLOOR" "$WINDOW_FLOOR" | sort -V | tail -1)
echo "$MIN_BINARY"
