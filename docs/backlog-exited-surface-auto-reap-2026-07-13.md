# [백로그·개발] cysd exited surface 즉시/주기 자동 reap (GC)

> 상태: **미착수(스펙만)** · 우선도: 낮음(잔재는 복원 사이클·앱 메뉴로 청소됨) ·
> 시의성: 중(임시페인 설계 도입으로 잔재 빈도 증가) · 제기일 2026-07-13(master 실측)

## 문제 (증상)
워커/노드가 죽으면(자기닫힘·exit) 프로세스는 죽지만 **surface 레코드가 'exited' 잔재로 남아 자리를 차지**한다. 이를 즉시/주기적으로 청소(reap)하는 데몬 레벨 GC가 없다 — 현재 자동 회수는 **phoenix 복원(restore) 흐름 안에서만**(`javis_phoenix.py::c6_reap_stale_surfaces`) 실행돼 재부팅·복원 사이클 때만 청소된다. 평시엔 잔재가 계속 남고, 사용자가 앱 Close 메뉴로 수동 정리해야 한다.

## 왜 지금 (시의성)
2026-07-13 정기스캔을 **임시페인(ephemeral) 설계**로 전환(매일 fresh 발화→스캔→워커 self-close). self-close가 매일 exited 잔재를 만들므로, 데몬 auto-reap이 없으면 잔재가 누적된다(복원 전까지). 이 개선이 임시페인 설계와 짝을 이룬다.

## 실측 근거 (2026-07-13 master 디버깅)
- 마스터(surface-bound)가 `cys close-surface <타 surface> --reap` → `close_denied: caller (surface 86) may only close its own surface`. **소유권은 소켓 연결 주체 기반** — 어떤 surface(마스터·CSO 포함)도 남의 잔재를 못 닫음(안전 설계). env(`CYS_SURFACE_ID`) 벗겨도 동일(연결 기반 귀속).
- 유일한 자동 reap = `c6_reap_stale_surfaces`(phoenix), **restore 흐름 내부(line ~1655)**에서만 호출. `cmd_reconcile`엔 미포함.
- pack 레벨 임시 sweep(`javis_phoenix` import 후 함수 직접 호출) 시도 → 내부 `cys()` 래퍼 바이너리 해석 실패(`executable None`)로 폐기. → 구현은 **cys CLI 직접 호출** 또는 **cysd 네이티브**로.

## 제안 구현 (택1, cysd 네이티브 선호)
1. ★**cysd 네이티브(선호)**: surface **exit 이벤트 순간 데몬이 즉시 Reap**(이벤트 기반). 또는 짧은 주기(예: 60s) GC 틱.
2. **pack 차선**: 데몬 스케줄 `--command` 잡이 `cys status/roster`로 exited 잔재를 열거 → 각 `cys close-surface <ref> --reap`(데몬 컨텍스트=소유권 우회). javis_phoenix 경유 금지(binary snag).

## 불변 제약 (깨면 안 됨 — 회귀 위험)
- **Reap 사유로만**(`CloseCause::Reap`·묘비 미생성·부활 대상 유지). OwnerClose(묘비 생성) 절대 금지 — 폐역되면 스케줄 fresh 재발화가 막힘(P0-6 오묘비 함정).
- **라이브(exited=false)는 절대 비대상** — 살아있는 surface 접촉 시 작업 파괴.
- **데몬 특권 경로만** — surface-bound 호출은 소유권으로 계속 거부(안전장치 유지).
- **phoenix 복원 흐름의 기존 reap과 충돌·이중 처리 없게**(락·멱등).

## 수용 기준 (검증)
- [ ] exited surface가 N초(예: ≤60s) 내 회수됨
- [ ] 회수 후 **묘비 0**(부활 대상 유지 — 스케줄 fresh 재발화 정상)
- [ ] 라이브 surface **무접촉**(exited=false 회귀 0)
- [ ] phoenix 복원 사이클 reap **회귀 0**(중복·경합 없음)
- [ ] 임시페인 일일 사이클(self-close→exit→reap) 무인 실증 1회

## 흐름 (정본 §1-A)
master 스펙 박제(본 문서) → cys-terminal-src 개발 백로그 → **워커에 구현·테스트·검증 위임** → master 리뷰. 마스터 직접 코딩 금지(토큰 절약).
