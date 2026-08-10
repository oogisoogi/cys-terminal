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

// 사용량 패널(#ws-usage)의 글자 크기 — 오너 지시 2026-08-10(「토큰량/ctx 표시 폰트를 20 정도로 고정하자」).
// ★고정값인 이유: 초판은 calc(11px × --ui-chrome-scale)이라 **메뉴 배율에만** 연동됐고,
//   사이드바 자체 글자 배율(--wsbar-font)은 목록에만 걸려 있었다 — 목록을 키워도 이 패널만
//   작게 남는 구조였다. 어느 한쪽에 다시 매면 같은 어긋남이 방향만 바꿔 되돌아오므로,
//   두 배율 어디에도 매지 않고 px로 못박는다.
// ★CSS(style.css `#ws-usage`)와 이 상수는 **같은 값이어야** 아래 산식이 참이다.
//   둘을 손으로 맞추면 언젠가 어긋나므로 wsbar.test.ts가 style.css 선언을 읽어 대조한다(드리프트 가드).
export const WSU_FONT_PX = 20;

// 페인 CTX 행에 관측 나이를 병기할 자리가 있는가 (오너 발의 2026-08-08).
// ★왜 조건부인가 — 나이 칸은 고정폭이고 트랙바만 flex:1 이라, 좁은 폭에서는 **트랙바가 0이 되고도
//   행이 넘친다**(헤드리스 실측: 폭 176px = WSBAR_W_MIN 에서 4.1px 넘침·트랙 0px). 넘친 칸은
//   #wsbar 의 overflow:hidden 에 잘려 보이지도 않으므로, 그때는 새 칸을 **내주는 쪽**이 맞다.
//   브리프 제약("기존 정보 = 트랙바·%·출처 마크를 밀어내지 마라")을 지키는 유일한 방향이다.
// 산식은 실측 상수로 세운다 — 행의 고정 칸은 전부 em(패널 글자 크기 = WSU_FONT_PX)이고
// 칸 사이 gap(6px×4)·패널 좌우 padding(16px)·오른쪽 경계선(1px)만 px 이다.
// ★출처 마크·나이 칸은 폭이 em인데 **자기 글자가 .85em**이다 — em 폭은 그 칸의 제 글자 크기로
//   풀리므로 패널 기준으로는 둘 다 0.85를 곱해야 한다. 초판은 나이 칸에만 곱하고 출처 마크에는
//   빠뜨려 3px을 더 세고 있었다(20px 기준·헤드리스 역산으로 오차 0.00px까지 확인 2026-08-10).
//   글자가 13.75px이던 때는 오차가 2px이라 드러나지 않았다 — 글자를 키우자 보였다.
export const CTX_ROW_FIXED_EM = 4.2 /* 번호·이름 */ + 2.6 /* % */ + 1 * 0.85 /* 출처 마크(0.85em 글자의 1em) */ + 2.9 * 0.85 /* 나이(0.85em 글자의 2.9em) */;
export const CTX_TRACK_MIN_EM = 2.4; // 이보다 좁아지면 막대가 값을 못 보여준다(% 칸과 비슷한 폭이 하한)
// ★판정 인자가 폭 하나로 줄었다 — 패널 글자가 고정되면서 메뉴 배율이 이 행의 폭에 관여하지 않는다.
//   (배율을 인자로 남겨 두면 「받지만 쓰지 않는 값」이 되어 호출자가 여전히 연동돼 있다고 오해한다.)
export function showsRowAge(wsbarW: number): boolean {
  if (!Number.isFinite(wsbarW)) return false;
  const availPx = wsbarW - 1 /* border-right */ - 16 /* padding 8+8 */ - 24 /* gap 6px × 4 */;
  return availPx >= (CTX_ROW_FIXED_EM + CTX_TRACK_MIN_EM) * WSU_FONT_PX;
}
