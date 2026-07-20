import { describe, expect, test } from "bun:test";
import {
  composeFontFamily,
  DEFAULT_FONT_STACK,
  FONT_CHOICES,
  nodeWorking,
  OUTPUT_IDLE_SECS,
  ROLE_COLOR,
  roleDotColor,
  STATUS_FRESH_SECS,
} from "./appearance";

describe("composeFontFamily", () => {
  test("null·공백 = 기본 스택 그대로", () => {
    expect(composeFontFamily(null)).toBe(DEFAULT_FONT_STACK);
    expect(composeFontFamily("")).toBe(DEFAULT_FONT_STACK);
    expect(composeFontFamily("   ")).toBe(DEFAULT_FONT_STACK);
  });

  test("선택 폰트를 기본 스택 앞에 합성 — CJK 폴백 보존", () => {
    const fam = composeFontFamily("JetBrains Mono");
    expect(fam.startsWith("'JetBrains Mono', ")).toBe(true);
    expect(fam.endsWith(DEFAULT_FONT_STACK)).toBe(true);
  });

  test("따옴표 섞인 입력은 소거 후 인용 — CSS 리스트 파손 방지", () => {
    expect(composeFontFamily("'SF Mono'")).toBe(`'SF Mono', ${DEFAULT_FONT_STACK}`);
    expect(composeFontFamily('D2"Coding')).toBe(`'D2Coding', ${DEFAULT_FONT_STACK}`);
  });

  test("FONT_CHOICES 전 선택지가 유효 합성값을 낸다(기본값 포함)", () => {
    for (const c of FONT_CHOICES) {
      const fam = composeFontFamily(c.face);
      expect(fam.endsWith(DEFAULT_FONT_STACK)).toBe(true);
    }
  });
});

describe("roleDotColor", () => {
  test("무역할(일반 셸) = null → 점 숨김", () => {
    expect(roleDotColor(null)).toBeNull();
    expect(roleDotColor(undefined)).toBeNull();
    expect(roleDotColor("")).toBeNull();
  });

  test("정식 역할 4종은 CC 색상표와 일치", () => {
    for (const role of ["master", "cso", "worker", "reviewer-gemini"]) {
      expect(roleDotColor(role)).toBe(ROLE_COLOR[role]);
    }
  });

  test("변형 역할은 접두 매칭 — 데몬 역할 변형 계약(overrides.rs·pack.rs)과 정합", () => {
    expect(roleDotColor("worker-2")).toBe(ROLE_COLOR.worker);
    expect(roleDotColor("cso-1")).toBe(ROLE_COLOR.cso);
    expect(roleDotColor("master-2")).toBe(ROLE_COLOR.master);
    expect(roleDotColor("reviewer")).toBe(ROLE_COLOR["reviewer-gemini"]);
  });

  test("리뷰어는 에이전트 불문 한 색(오너 확정) — codex 개별 항목 없이 접두 매칭으로 통일", () => {
    expect(ROLE_COLOR["reviewer-codex"]).toBeUndefined();
    expect(roleDotColor("reviewer-codex")).toBe(ROLE_COLOR["reviewer-gemini"]);
    expect(roleDotColor("reviewer-claude")).toBe(ROLE_COLOR["reviewer-gemini"]);
  });

  test("미지 역할은 회색 폴백", () => {
    expect(roleDotColor("librarian")).toBe("#64748b");
  });

  test("4대 역할군 색이 서로 구별된다(오너 요구)", () => {
    const set = new Set([roleDotColor("master"), roleDotColor("cso"), roleDotColor("worker"), roleDotColor("reviewer-gemini")]);
    expect(set.size).toBe(4);
  });
});

describe("nodeWorking", () => {
  test("신선한 자기보고는 state 그대로 — working만 작동중", () => {
    expect(nodeWorking({ state: "working", age_secs: 0 }, 999)).toBe(true);
    expect(nodeWorking({ state: "working", age_secs: STATUS_FRESH_SECS }, 999)).toBe(true);
    expect(nodeWorking({ state: "waiting", age_secs: 0 }, 0)).toBe(false);
    expect(nodeWorking({ state: "done", age_secs: 0 }, 0)).toBe(false);
  });

  test("★stale working은 불신 — 완료 후 idle 미보고 워커의 영구 깜빡 차단(버그 재현 케이스)", () => {
    // 워커가 working 보고 후 완료했지만 idle을 안 보냄: status는 working에 박제, 출력은 멈춤(idle 큼)
    expect(nodeWorking({ state: "working", age_secs: STATUS_FRESH_SECS + 1 }, 600)).toBe(false);
    // stale이어도 실제 출력이 흐르면 활동 파생으로 작동중 유지
    expect(nodeWorking({ state: "working", age_secs: 3600 }, 5)).toBe(true);
  });

  test("자기보고 부재 시 출력 활동(idle_secs)으로 파생 판정", () => {
    expect(nodeWorking(null, OUTPUT_IDLE_SECS)).toBe(true);
    expect(nodeWorking(null, OUTPUT_IDLE_SECS + 1)).toBe(false);
    expect(nodeWorking(undefined, null)).toBe(false); // idle 미상 = 비작동(안전 기본)
  });

  test("종료 surface는 무조건 비작동", () => {
    expect(nodeWorking({ state: "working", age_secs: 0 }, 0, true)).toBe(false);
  });
});
