// 부서 런칭 중(pending) 탭 라벨의 순수 계산 — main.ts의 buildTab이 이 함수에 배선만 한다(스피너 글리프·DOM은 호출측).
//
// WP-10: '＋부서' 클릭 후 부서 데몬 준비(~12초) 동안 라벨이 "…"만 보이면 사용자가 '멈춘 줄'로 오해한다.
// pending 탭엔 "부서 제작 중…"을 표시해 진행 중임을 명시한다. 순수 함수라 pending/확정 라벨을
// 결정론으로 회귀 테스트할 수 있다(deptlabel.test.ts).

// pending 부서 탭에 표시할 진행 라벨(스피너 글리프는 CSS가 담당 — 여기선 텍스트만).
export const DEPT_PENDING_LABEL = "부서 제작 중…";

// pending이면 진행 라벨, 확정되면 실제 부서 표시명.
export function deptPlaceholderLabel(ws: { pending?: boolean; name: string }): string {
  return ws.pending ? DEPT_PENDING_LABEL : ws.name;
}
