// theme.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 배경 휘도에 따른 가독 전경색 자동 전환이 어두운 색·밝은 색·0.5 경계에서 결정론인지 검증한다.
import { describe, it, expect } from "bun:test";
import {
  hexToRgb,
  relativeLuminance,
  readableForeground,
  DARK_FG,
  LIGHT_FG,
  DEFAULT_BG,
} from "./theme";

describe("hexToRgb — 파싱", () => {
  it("# 있는 6자리", () => {
    expect(hexToRgb("#0d1117")).toEqual([13, 17, 23]);
  });
  it("# 없는 6자리도 허용", () => {
    expect(hexToRgb("ffffff")).toEqual([255, 255, 255]);
  });
  it("검정·빨강·초록·파랑 채널 분해", () => {
    expect(hexToRgb("#000000")).toEqual([0, 0, 0]);
    expect(hexToRgb("#ff0000")).toEqual([255, 0, 0]);
    expect(hexToRgb("#00ff00")).toEqual([0, 255, 0]);
    expect(hexToRgb("#0000ff")).toEqual([0, 0, 255]);
  });
  it("잘못된 형식은 null", () => {
    expect(hexToRgb("#fff")).toBeNull();
    expect(hexToRgb("notacolor")).toBeNull();
    expect(hexToRgb("")).toBeNull();
  });
});

describe("relativeLuminance — 0~1 정규화", () => {
  it("검정=0, 흰색=1", () => {
    expect(relativeLuminance("#000000")).toBe(0);
    expect(relativeLuminance("#ffffff")).toBeCloseTo(1, 10);
  });
  it("기본 다크 배경은 매우 어둡다", () => {
    expect(relativeLuminance(DEFAULT_BG)).toBeLessThan(0.1);
  });
  it("파싱 실패는 0(어둡다고 간주)", () => {
    expect(relativeLuminance("bad")).toBe(0);
  });
});

describe("readableForeground — 휘도>0.5 경계 전환", () => {
  it("어두운 배경 → 밝은 글자(DARK_FG)", () => {
    expect(readableForeground("#000000")).toBe(DARK_FG);
    expect(readableForeground(DEFAULT_BG)).toBe(DARK_FG);
    expect(readableForeground("#0000ff")).toBe(DARK_FG); // 순수 파랑=휘도 0.07 → 흰 글자 유지
  });
  it("밝은 배경 → 어두운 글자(LIGHT_FG)", () => {
    expect(readableForeground("#ffffff")).toBe(LIGHT_FG);
    expect(readableForeground("#ffff00")).toBe(LIGHT_FG); // 노랑=휘도 0.93
  });
  it("경계: #808080(L≈0.502)>0.5 → 어두운 글자, #7f7f7f(L≈0.498)≤0.5 → 밝은 글자", () => {
    expect(readableForeground("#808080")).toBe(LIGHT_FG);
    expect(readableForeground("#7f7f7f")).toBe(DARK_FG);
  });
  it("파싱 실패는 밝은 글자(기본 다크 전제)", () => {
    expect(readableForeground("bad")).toBe(DARK_FG);
  });
});
