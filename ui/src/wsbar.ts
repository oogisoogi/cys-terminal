// 워크스페이스 사이드바(#wsbar) 폭·글자 배율 계산 (순수 로직 — main.ts에서 분리해 단위테스트 대상).
// 오너 요청 2026-07-12: ①사이드바 폭을 마우스 드래그로 조절 ②글자 크기 조절 — 둘 다 localStorage 영속.
// 폭 하한은 헤더 버튼(＋부서/＋)이 잘리지 않는 선, 상한은 터미널 작업공간 잠식 방지.

export const WSBAR_W_MIN = 176; // 140이면 헤더(제목+버튼4)가 2단 랩(높이 33→70px) — 랩 없는 실측 하한(성찰 후속 2026-07-12)
export const WSBAR_W_MAX = 520;
export const WSBAR_W_DEFAULT = 216;
export const WSBAR_FONT_MIN = 0.8;
export const WSBAR_FONT_MAX = 2.2;
export const WSBAR_FONT_STEP = 0.1;

export function clampWsbarWidth(w: number): number {
  if (!Number.isFinite(w)) return WSBAR_W_DEFAULT;
  return Math.min(WSBAR_W_MAX, Math.max(WSBAR_W_MIN, Math.round(w)));
}

export function clampWsbarFont(f: number): number {
  if (!Number.isFinite(f) || f <= 0) return 1;
  return +Math.min(WSBAR_FONT_MAX, Math.max(WSBAR_FONT_MIN, f)).toFixed(2);
}

// 페인 CTX 행에 관측 나이를 병기할 자리가 있는가 (오너 발의 2026-08-08).
// ★왜 조건부인가 — 나이 칸은 고정폭이고 트랙바만 flex:1 이라, 좁은 폭에서는 **트랙바가 0이 되고도
//   행이 넘친다**(헤드리스 실측: 폭 176px = WSBAR_W_MIN 에서 4.1px 넘침·트랙 0px). 넘친 칸은
//   #wsbar 의 overflow:hidden 에 잘려 보이지도 않으므로, 그때는 새 칸을 **내주는 쪽**이 맞다.
//   브리프 제약("기존 정보 = 트랙바·%·출처 마크를 밀어내지 마라")을 지키는 유일한 방향이다.
// 산식은 실측 상수로 세운다 — 행의 고정 칸은 전부 em(패널 글자 크기 = 11px × chromeScale)이고
// 칸 사이 gap(6px×4)·패널 좌우 padding(16px)·오른쪽 경계선(1px)만 px 이다.
export const CTX_ROW_FIXED_EM = 4.2 /* 번호·이름 */ + 2.6 /* % */ + 1 /* 출처 마크 */ + 2.9 * 0.85 /* 나이(0.85em 글자의 2.9em) */;
export const CTX_TRACK_MIN_EM = 2.4; // 이보다 좁아지면 막대가 값을 못 보여준다(% 칸과 비슷한 폭이 하한)
export function showsRowAge(wsbarW: number, chromeScale: number): boolean {
  if (!Number.isFinite(wsbarW) || !Number.isFinite(chromeScale) || chromeScale <= 0) return false;
  const emPx = 11 * chromeScale; // #ws-usage 의 font-size: calc(11px * var(--ui-chrome-scale))
  const availPx = wsbarW - 1 /* border-right */ - 16 /* padding 8+8 */ - 24 /* gap 6px × 4 */;
  return availPx >= (CTX_ROW_FIXED_EM + CTX_TRACK_MIN_EM) * emPx;
}
