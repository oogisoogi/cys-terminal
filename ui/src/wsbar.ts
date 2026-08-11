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

// 사용량 패널(#ws-usage)의 **기준** 글자 크기 — 실제 크기 = 이 수 × 사이드바 글자 배율(--wsbar-font).
// ★의미가 한 번 바뀐 상수다(고정값 → 기준값). 오너 지시 2026-08-10은 「20 정도로 고정」이었고
//   c48dbdf는 두 배율 어디에도 매지 않았다. 오너 판정 2026-08-11이 그 위에 얹혔다 —
//   「글자 크기 바꿀 때마다 재설치는 아니다, 조절이 되게 하자」. 그래서 값은 그대로 두고
//   **배율의 기준점**으로 역할만 옮겼다(배율 1.0 = 20px = c48dbdf 라이브와 동일).
// ★어느 축에 매는가가 이 결정의 전부다: 원 사고는 이 패널이 **메뉴 배율**(--ui-chrome-scale)에
//   매여 사이드바 목록(.ws-tab)과 따로 논 것이었다. 지금 매는 축은 **목록이 매여 있는 그 축**
//   (--wsbar-font)이라 둘이 함께 움직인다 — 원 사고의 재발이 아니라 그 반대편이다.
//   메뉴 배율과의 비연동은 그대로 유지되며 wsbar.test.ts가 그 부재를 단언한다.
// ★CSS(style.css `#ws-usage`)의 수식 `calc(<이 수>px * var(--wsbar-font, 1))`과 이 상수는
//   **같은 수를 써야** 아래 산식이 참이다. 손으로 맞추면 언젠가 어긋나므로 wsbar.test.ts가
//   style.css 선언을 읽어 대조한다(드리프트 가드).
// ★값이 20 → 16으로 내려왔다(오너 캡처 판정 2026-08-11 후속 · 서열 교정). 바뀐 것은 「읽히는 크기」가
//   아니라 **서열**이다 — 상단 버튼(16) = 이 패널 < 목록 제목(20). 상세 사유는 style.css `#ws-usage` 주석.
//   ⚠이 수는 화면 크기만이 아니라 **아래 showsRowAge 산식의 배율기**이기도 하다. 내려가면 행의 고정 칸이
//     함께 좁아져 **나이 칸이 더 좁은 폭부터 나온다**(임계 292px → 242px · 배율 1.0). 크기 변경의
//     부수효과가 아니라 같은 결정의 다른 얼굴이므로 테스트의 임계 기대값을 함께 다시 겨눴다.
export const WSU_FONT_PX = 16;

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
// ★판정 인자는 폭과 **사이드바** 배율 둘이다(오너 판정 2026-08-11로 되돌아온 인자).
//   c48dbdf에서 인자가 폭 하나로 줄었던 것은 패널 글자가 어느 배율에도 매이지 않아
//   행의 고정 칸(전부 em)이 화면 설정과 무관해졌기 때문이다. 이제 패널 글자가 --wsbar-font를
//   타므로 **같은 폭이라도 배율을 키우면 자리가 없어진다** — 인자를 되살리지 않으면 큰 배율에서
//   트랙바가 0이 되고 행이 넘쳐 잘린다(c48dbdf 이전과 같은 실패 모드).
//   ⚠되살린 인자는 메뉴 배율이 아니라 **사이드바 배율**이다 — 축이 다르다.
// ★폭은 fail-safe(모르면 거짓), 배율은 **1.0 폴백**이다 — 비대칭이 의도다: CSS가
//   `var(--wsbar-font, 1)`로 같은 폴백을 쓰므로, 변수가 없을 때 화면은 실제로 1.0로 그려진다.
//   여기서 거짓을 내면 「화면은 20px인데 산식만 자리 없다고 판정」하는 어긋남이 된다.
export function showsRowAge(wsbarW: number, wsbarFont: number): boolean {
  if (!Number.isFinite(wsbarW)) return false;
  const scale = Number.isFinite(wsbarFont) && wsbarFont > 0 ? wsbarFont : 1;
  const availPx = wsbarW - 1 /* border-right */ - 16 /* padding 8+8 */ - 24 /* gap 6px × 4 */;
  return availPx >= (CTX_ROW_FIXED_EM + CTX_TRACK_MIN_EM) * WSU_FONT_PX * scale;
}
