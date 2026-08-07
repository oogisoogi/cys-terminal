// pane 외관의 순수 계산 — 터미널 폰트 조합·역할 신호 점 색.
//
// main.ts의 applyFontFace/setRoleDot는 이 함수들에 배선만 한다(DOM·localStorage는 호출측).
// 순수 함수라 폰트 폴백·역할 변형(worker-2 등)을 결정론으로 회귀 테스트할 수 있다(appearance.test.ts).

// 기본 터미널 폰트 스택 — ★Windows: Latin 등폭폰트(Cascadia Mono/Consolas)를 CJK 폰트보다
// 앞에 둔다. 아니면 Menlo/SF Mono 부재 시 xterm가 셀 폭을 CJK 전각폰트(Noto Sans KR)로 측정해
// Latin 글자가 넓게 벌어진다(자간 이상).
export const DEFAULT_FONT_STACK =
  "'JetBrains Mono', Menlo, 'SF Mono', 'Cascadia Mono', Consolas, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', monospace";

// 선택 폰트를 기본 스택 '앞'에 합성 — 미설치 폰트는 브라우저가 체인 아래로 폴백하고
// CJK 폴백(한글)은 항상 보존된다. null·공백 = 기본 스택 그대로.
export function composeFontFamily(face: string | null): string {
  const f = face?.trim().replace(/['"]/g, "");
  return f ? `'${f}', ${DEFAULT_FONT_STACK}` : DEFAULT_FONT_STACK;
}

// 폰트 선택지(테마 팝오버) — face null = 기본 스택. 미설치 폰트는 합성 폴백으로 무해.
export const FONT_CHOICES: { label: string; face: string | null }[] = [
  { label: "기본값", face: null },
  { label: "Menlo", face: "Menlo" },
  { label: "SF Mono", face: "SF Mono" },
  { label: "Monaco", face: "Monaco" },
  { label: "Cascadia Mono", face: "Cascadia Mono" },
  { label: "Consolas", face: "Consolas" },
  { label: "JetBrains Mono", face: "JetBrains Mono" },
  { label: "D2Coding", face: "D2Coding" },
  { label: "Nanum Gothic Coding", face: "Nanum Gothic Coding" },
  { label: "Courier New", face: "Courier New" },
];

// ── 메뉴(상단 툴바) 글자 크기 — 오너 요청 2026-08-07 「페인 상단 메뉴바 글자가 작다」.
//
// 상단 툴바(#topbar)·사이드바 헤더·크롬 버튼은 전부 `calc(<기본px> * var(--ui-chrome-scale))`로
// 그려진다(style.css). 즉 배율 변수는 이미 있었고 **그것을 조절할 UI만 없었다** — 이 함수가
// 테마 팝오버의 「메뉴 크기」 입력(사람이 읽는 %)을 CSS 배수로 옮긴다.
//
// %를 쓰는 이유: 대상 요소들의 기본 크기가 11·12·13px로 제각각이라 「몇 px」로 물으면 답이 하나가
// 아니다. 배율은 그 전부에 일관되게 걸리는 유일한 단위다.
export const MENU_SCALE_DEFAULT_PCT = 125; // style.css :root의 --ui-chrome-scale: 1.25와 같은 값
export const MENU_SCALE_MIN_PCT = 80;
export const MENU_SCALE_MAX_PCT = 250;

// %(문자열·숫자) → CSS 배수 문자열. null = 기본값으로 되돌림(변수 제거).
// ★비정상 입력(빈칸·문자·NaN·Infinity)도 null로 접는다 — CSS에 NaN이 들어가면 툴바 글자가
// 통째로 사라지고, 사용자는 그것을 「앱이 깨졌다」로 읽는다. 입력창은 지우는 중에 반드시 빈칸을
// 거치므로 이 경로는 예외가 아니라 정상 경로다.
export function menuScaleFromPct(pct: number | string | null | undefined): string | null {
  if (pct === null || pct === undefined) return null;
  const n = typeof pct === "number" ? pct : Number(String(pct).trim());
  if (!Number.isFinite(n) || n <= 0) return null;
  const clamped = Math.min(MENU_SCALE_MAX_PCT, Math.max(MENU_SCALE_MIN_PCT, n));
  // 부동소수 꼬리(1.2500000000000002) 차단 — CSS 변수는 문자열로 남으므로 자릿수를 고정한다.
  return String(Math.round((clamped / 100) * 1000) / 1000);
}

// 역할 → 신호 색 — Control Center(CC_ROLE_COLOR)와 pane 역할 점의 단일 출처.
export const ROLE_COLOR: Record<string, string> = {
  master: "#3b82f6", cso: "#8b5cf6", worker: "#00e676",
  "reviewer-gemini": "#ffa726",
};

// 노드 '작동중' 판정 — pane 역할 점 깜빡(main.ts surfaceWorking)·CC 작업중 카운트의 단일 출처.
// 자기보고(set-status)는 신선할 때만 신뢰한다: 워커가 working 보고 후 완료 시 idle을 안 보내면
// status가 working에 박제(stale)돼 점이 영구 깜빡인다. stale·부재 시엔 출력 활동(idle_secs)으로
// 파생 판정한다 — 신선 임계 120s는 CC의 stale 배지(.cc-task-row.stale)와 동일 기준.
export const STATUS_FRESH_SECS = 120;
export const OUTPUT_IDLE_SECS = 60;
export type AgentStatus = { state?: string; age_secs?: number } | null | undefined;
export function nodeWorking(
  status: AgentStatus,
  idleSecs: number | null | undefined,
  exited = false,
): boolean {
  if (exited) return false;
  if (status && (status.age_secs ?? 0) <= STATUS_FRESH_SECS) return status.state === "working";
  return (idleSecs ?? Infinity) <= OUTPUT_IDLE_SECS;
}

// pane 제목 앞 역할 점 색 — 정확 일치 우선, 변형은 접두 매칭(master-2·cso-1·worker-2·reviewer-* —
// overrides.rs·pack.rs의 역할 접두 매칭 관례와 동일), 미지 역할은 회색, 무역할(일반 셸)은 null(점 숨김).
export function roleDotColor(role: string | null | undefined): string | null {
  if (!role) return null;
  if (ROLE_COLOR[role]) return ROLE_COLOR[role];
  if (role.startsWith("master")) return ROLE_COLOR.master;
  if (role.startsWith("cso")) return ROLE_COLOR.cso;
  if (role.startsWith("worker")) return ROLE_COLOR.worker;
  if (role.startsWith("reviewer")) return ROLE_COLOR["reviewer-gemini"];
  return "#64748b";
}
