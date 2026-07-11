// Control Center 본문 배율 계산 (순수 로직 — main.ts에서 분리해 단위테스트 대상으로).
// 자동 배율: 창 크기에 CC 본문을 비례 연동(오너 승인 2026-07-12 '비례 스케일').
// 기준 창(820×760)=1.0x — 820 = cc-body 폭 상한 780px + 좌우 패딩 40px.
// 가로·세로 비율 중 작은 쪽 채택: 어느 축을 줄여도 본문이 함께 작아진다.
// 합성 배율: 수동 zoom(Cmd +/-, 0.6~2.0)과 곱으로 합성하되 CC_ZOOM_CAP으로 상한 —
// 극단 조합(2.0×2.2=4.4x)의 암묵 폭주를 계약으로 봉인.

export const CC_SCALE_BASE_W = 820;
export const CC_SCALE_BASE_H = 760;
export const CC_AUTO_MIN = 0.7;
export const CC_AUTO_MAX = 2.2;
export const CC_ZOOM_CAP = 2.5;

export function ccAutoScale(winW: number, winH: number): number {
  const s = Math.min(winW / CC_SCALE_BASE_W, winH / CC_SCALE_BASE_H);
  if (!Number.isFinite(s) || s <= 0) return 1; // 비정상 창 치수 방어(마운트 직후 0 등)
  return Math.min(CC_AUTO_MAX, Math.max(CC_AUTO_MIN, s));
}

export function ccEffectiveZoom(manualZoom: number, winW: number, winH: number): number {
  const manual = Number.isFinite(manualZoom) && manualZoom > 0 ? manualZoom : 1;
  return Math.min(CC_ZOOM_CAP, manual * ccAutoScale(winW, winH));
}
