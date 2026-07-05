# 멀티마스터 충돌 진단·설계 — 적대검증 패킷 (R2)

> 작성: master (R1 심판 종합 — gemini·codex 양 REVISE 수용 반영). 검증: reviewer-gemini·reviewer-codex.
> R1 대비 변경: 진단 6건 정정·설계 3건 재설계. 각 항목의 R1 출처(수용/반박)를 명기한다.
> 규칙: 지정 파일/범위만. 추측 금지 — 판정마다 증거 file:line. 점수 금지. verdict=ACCEPT|REVISE|BLOCK|ESCALATE.
> 회신: verdict JSON을 `_round/VERDICT-multimaster-R2-<이름>.json`에 저장(javis_verdict.py validate 통과) 후
> `cys send --queued --to master "[리뷰어명 R2] ..."`.

---

## 0. R1 판정 요약 (장부 ORCHESTRATION-multimaster.md)

- R1: gemini REVISE + codex REVISE — 양측 verdict JSON 검증(exit 0)·기록 완료.
- master 실측 확정 사실: ①/opt/homebrew/bin/cys-dept 심링크 실재(7/3 20:58) ②depts.json 완전 공백
  ③고아 pack-dept-dept-1~5 5건 ④allocate가 role=master 빈 셸 생성(cys-dept:438-450)
  ⑤빈 셸의 claim gate 선점 = 원 증상 재발 루트(gemini R2 코드검증·handlers.rs:1257-1281 holder_live)
  ⑥ready() 최악 12초(120×0.1s, cys-dept:222-223) ⑦btn-ws-new 배선 현행 위치 = main.ts:3883.

## 1. 진단 v2 (정정 반영)

### C1' — claim-role 경로엔 kill 없음 (좁힘 · codex 논쟁점 수용)
`claim-role` 거부 경로(handlers.rs:1257-1281)에는 kill이 없다 — **단 이 서술은 claim 경로 한정**이다.
cys-dept의 down/down-sock/reap(:595-683)에는 graceful_kill 경로가 실재한다. "시스템 전체 kill 없음"으로
일반화하지 않는다.

### C2' — 레지스트리 드리프트 상태 (재진단 · codex Issue 3 수용)
depts.json=`{"depts":{}}` 공백 + pack-dept-dept-1~5 잔존 = **registry drift**. R1의 "dept-1·dept-3 라이브
격리 증거"는 현 환경에서 성립하지 않는다(환경 드리프트). 격리·승격 메커니즘의 과거 동작 증거는 tombstone·
스냅샷으로만 남아 있다 — 라이브 재현은 R2 검증 항목이 아니라 D1' 구현 후 실측 게이트로 이월.

### C3' — 근본원인 유지 + 배선 증거 라인 갱신 (codex Issue 2 수용)
"새 ws가 기본 데몬에 얹힘" 진단은 유지하되 증거를 현행 라인으로 재인용:
`main.ts:3883` btn-ws-new→`addWorkspace()`(2591 — newSurface()에 socket 미전달=기본 데몬),
`main.ts:3885-3896` btn-ws-dept→`addDeptWorkspace()`(2614). R1의 3348-3350 인용은 stale(현행은 modal 코드) — 폐기.
★신규 확정: `main.ts:3879-3880` 주석 = **오너 2026-06-29 결정으로 이 이원화가 의도된 설계임이 명문**.

### C4' — allocate는 role=master 빈 셸을 생성한다 (R1 C4 폐기·전면 정정 · 양 리뷰어 일치)
`cys-dept:438-450`: CYS_DEPT_NO_MASTER=1이 아니면 `new-surface --role master` 빈 셸 생성(멱등 가드).
**함의(gemini R2 코드검증 확정)**: 이 빈 셸이 claim gate를 선점 → 사용자가 같은 데몬의 *다른* surface에서
claim-role master 시 claim_denied 재발(holder_live=true·holder≠sid). 의도된 UX는 "할당된 master pane에
직접 claude 연결"(self re-claim 허용) — 이 UX가 어디에도 안내되지 않는 것이 실질 결함.

### C5' — 곁가지 결함 (정정판)
- C5a 철회: 심링크 실재 — PATH 래퍼 설계 폐기(잔여 확인 항목: GUI/launchd 환경의 PATH만 별도 검증).
- C5b' 확대: 고아 = pack-dept-dept-**1~5 전부**(5건) + reap은 depts.json 스냅샷만 순회(:638-683)라
  **미등록 고아를 구조적으로 못 본다**(codex Issue 5 수용).
- C5c 세분: CEO 데몬 편성=오너 결정 C(현행 유지·불변). **부서 데몬**의 boot 풀편성은 별건 결함(D3'-c).

### C6' — 설계 변수 확장 (codex 논쟁점 수용)
"빠진 건 2가지"는 과소진단. 설계 변수: 진입 배선·부트 데몬 자각·lifecycle mutation gate(:323-333)·
CYS_DEPT_CAP hard cap(:384-387)·socket-scoped UI cleanup(main.ts:2748-2763)·registry drift·reaper semantics.

## 2. 설계 v2 (각각 반박 대상)

### D1' — 진입 이원화 유지 + 빈 셸 선점 해소 (R1 D1 철회·재설계)
- **(a) btn-ws-new 격리 전환 철회**: 오너 2026-06-29 결정(main.ts:3879 주석)과 정면 충돌 + CAP=8 차단 +
  데몬 증식 + 최악 12초 지연(양 리뷰어 Issue 1). 일반 ws=기본 데몬 공유 유지, 격리는 +부서 전담 유지.
- **(b) 원 증상의 실제 해소 지점** = 빈 셸 선점 UX. 택1 검증 요청:
  - **(b-1, master 제안)** claim gate 승계 완화: holder가 agent 미연결(빈 셸)이면 takeover 허용 —
    handlers.rs holder_live 판정에 agent-backed 여부 추가. (리스크: '빈 셸' 판정의 결정론 기준 필요)
  - **(b-2)** allocate 기본을 CYS_DEPT_NO_MASTER=1로 뒤집고 GUI 입양 경로 재설계. (리스크: main.ts:1268
    !s.role 가드 — role 없는 surface는 자동입양 불가 → GUI 배선 동반 수정 필요)
  - **(b-3)** 현행 유지 + UX 명문화(할당된 master pane에 claude 연결 안내·잘못된 surface에서 claim 시
    에러 메시지에 올바른 pane 안내 추가).
- **수용 기준(codex 채택)**: "새 격리 ws당 live master 정확히 1개" + 더블생성/오입양 0.

### D2' — misrouted-master 게이트: identity 대조로 재설계 (codex Issue 6 + gemini Issue 2 수용)
live-master 유무 대조가 아니라 **expected identity(socket·pack_dir·account) 대조**로 설계:
부트 시 "내가 붙은 데몬의 socket/pack_dir"이 "내 역할이 기대하는 것"과 일치하는지 검증.
caller=null(외부 터미널 수동 부트)은 게이트 skip이 아니라 **identity 대조만으로 판정**(오탐 방지 —
gemini Issue 2의 예외 처리를 identity 방식이 자연 흡수함을 검증하라).

### D3' — 위생 v2
- (a) PATH 래퍼 철회(C5a). (b) 고아 정리: **registry 독립 orphan inventory**(파일시스템 스캔 기반) 신설 →
  소유 PID·socket 증명 → tar 백업 → **no-demote guard**(정리 작업이 ceo_demote를 트리거하지 않게) →
  비가역 삭제=오너 승인 게이트(5건 전부). (c) 부서 데몬 boot 필터: 부서 데몬에서 `cys boot`는 worker만
  기동(CSO·리뷰어 제외) — CEO 데몬은 현행 유지(오너 결정 C 불변).

## 3. R2 검증 요청 (지정 범위)

범위: 이 파일 + handlers.rs:1257-1281 + main.ts:2591-2614·3879-3896·1264-1272 + cys-dept(allocate·down·
down-sock·reap·:323-333·:384-387) + depts.json. 각 C'/D'에 verdict + 증거. 특히:
① D1'(b) 3안 중 어느 것이 "격리 ws당 live master 정확히 1개"를 최소 변경으로 달성하는가(각 안의 실패모드 포함).
② D2' identity 대조가 caller=null 오탐을 실제로 흡수하는가.
③ D3'(b) no-demote guard 없이 5건 정리 시 ceo_demote 오발동 경로가 실재하는가(코드로).
