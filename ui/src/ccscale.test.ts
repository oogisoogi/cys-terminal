// ccscale.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// CC 본문 자동 배율의 기준창=1.0x, min(가로비,세로비) 축 선택, 상·하한 클램프,
// 수동 zoom 합성과 CC_ZOOM_CAP 상한이 경계에서 결정론인지 검증한다.
import { describe, it, expect } from "bun:test";
import {
  ccAutoScale,
  ccEffectiveZoom,
  CC_SCALE_BASE_W,
  CC_SCALE_BASE_H,
  CC_AUTO_MIN,
  CC_AUTO_MAX,
  CC_ZOOM_CAP,
} from "./ccscale";

describe("ccAutoScale — 창 비율 자동 배율", () => {
  it("기준 창(820×760)은 정확히 1.0", () => {
    expect(ccAutoScale(CC_SCALE_BASE_W, CC_SCALE_BASE_H)).toBe(1);
  });
  it("가로·세로 중 작은 비율을 채택 — 세로가 병목", () => {
    // 1400/820=1.707…, 1000/760=1.315…  → 세로 쪽
    expect(ccAutoScale(1400, 1000)).toBeCloseTo(1000 / 760, 6);
  });
  it("가로·세로 중 작은 비율을 채택 — 가로가 병목", () => {
    // 900/820=1.097…, 1500/760=1.973… → 가로 쪽
    expect(ccAutoScale(900, 1500)).toBeCloseTo(900 / 820, 6);
  });
  it("작은 창은 하한 클램프", () => {
    expect(ccAutoScale(400, 300)).toBe(CC_AUTO_MIN);
  });
  it("거대 창은 상한 클램프", () => {
    expect(ccAutoScale(3000, 3000)).toBe(CC_AUTO_MAX);
  });
  it("클램프 경계 정확값 — 상한 진입 직전", () => {
    // 세로 병목으로 정확히 2.2가 되는 창: h = 760×2.2 = 1672
    expect(ccAutoScale(10000, 1672)).toBeCloseTo(CC_AUTO_MAX, 6);
  });
  it("비정상 치수(0·NaN·Infinity)는 1.0 폴백", () => {
    expect(ccAutoScale(0, 760)).toBe(1);
    expect(ccAutoScale(NaN, 760)).toBe(1);
    expect(ccAutoScale(820, Infinity)).toBe(1); // 한 축만 무한이면 유한한 축이 min으로 지배
    expect(ccAutoScale(Infinity, Infinity)).toBe(1); // 둘 다 무한 → 폴백
  });
});

describe("ccEffectiveZoom — 수동×자동 합성", () => {
  it("수동 1.0 × 기준 창 = 1.0", () => {
    expect(ccEffectiveZoom(1, CC_SCALE_BASE_W, CC_SCALE_BASE_H)).toBe(1);
  });
  it("수동 zoom은 자동 배율에 곱으로 합성", () => {
    const auto = ccAutoScale(1400, 1000);
    expect(ccEffectiveZoom(1.5, 1400, 1000)).toBeCloseTo(1.5 * auto, 6);
  });
  it("극단 조합(수동 2.0 × 자동 2.2 = 4.4)은 CC_ZOOM_CAP으로 봉인", () => {
    expect(ccEffectiveZoom(2, 3000, 3000)).toBe(CC_ZOOM_CAP);
  });
  it("축소 방향 합성(수동 0.6 × 자동 0.7)은 상한 미달 — 곱 그대로", () => {
    expect(ccEffectiveZoom(0.6, 400, 300)).toBeCloseTo(0.6 * CC_AUTO_MIN, 6);
  });
  it("비정상 수동값(NaN·0·음수)은 1.0 취급", () => {
    const auto = ccAutoScale(1400, 1000);
    expect(ccEffectiveZoom(NaN, 1400, 1000)).toBeCloseTo(auto, 6);
    expect(ccEffectiveZoom(0, 1400, 1000)).toBeCloseTo(auto, 6);
    expect(ccEffectiveZoom(-1, 1400, 1000)).toBeCloseTo(auto, 6);
  });
});
