// wsbar.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 사이드바 폭·글자 배율 클램프가 경계·비정상 입력에서 결정론인지 검증한다.
import { describe, it, expect } from "bun:test";
import {
  clampWsbarWidth,
  clampWsbarFont,
  WSBAR_W_MIN,
  WSBAR_W_MAX,
  WSBAR_W_DEFAULT,
  WSBAR_FONT_MIN,
  WSBAR_FONT_MAX,
} from "./wsbar";

describe("clampWsbarWidth — 사이드바 폭", () => {
  it("범위 내 값은 정수 반올림 통과", () => {
    expect(clampWsbarWidth(300.4)).toBe(300);
    expect(clampWsbarWidth(216)).toBe(WSBAR_W_DEFAULT);
  });
  it("하한 클램프", () => {
    expect(clampWsbarWidth(80)).toBe(WSBAR_W_MIN);
  });
  it("상한 클램프", () => {
    expect(clampWsbarWidth(9999)).toBe(WSBAR_W_MAX);
  });
  it("비정상(NaN·Infinity)은 기본폭", () => {
    expect(clampWsbarWidth(NaN)).toBe(WSBAR_W_DEFAULT);
    expect(clampWsbarWidth(Infinity)).toBe(WSBAR_W_DEFAULT);
  });
});

describe("clampWsbarFont — 글자 배율", () => {
  it("범위 내 값은 소수 2자리 통과", () => {
    expect(clampWsbarFont(1.25)).toBe(1.25);
  });
  it("하한·상한 클램프", () => {
    expect(clampWsbarFont(0.3)).toBe(WSBAR_FONT_MIN);
    expect(clampWsbarFont(5)).toBe(WSBAR_FONT_MAX);
  });
  it("비정상(NaN·0·음수)은 1.0", () => {
    expect(clampWsbarFont(NaN)).toBe(1);
    expect(clampWsbarFont(0)).toBe(1);
    expect(clampWsbarFont(-2)).toBe(1);
  });
  it("부동소수 잔여 자릿수 절사(0.1 step 누적 안전)", () => {
    expect(clampWsbarFont(1.1 + 0.1 + 0.1)).toBe(1.3);
  });
});
