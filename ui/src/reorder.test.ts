// reorder.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 배열 splice 기반 순서 변경이 같은 그룹 재정렬·그룹 간 이동·tier 클램프·경계(자기 위 no-op,
// 마지막 멤버 탈출, ungrouped↔group 왕복)에서 결정론이고 불변식(id 보존·중복 없음·pending 불변)을
// 지키는지 검증한다.
import { describe, it, expect } from "bun:test";
import { reorderWorkspace, reorderGroup, type ReorderWs, type ReorderGroup } from "./reorder";

// 테스트 편의: id·groupId만 담은 최소 ws (pending 옵션).
const ws = (id: number, groupId?: number, pending?: boolean): ReorderWs => ({ id, groupId, pending });
const ids = (list: ReorderWs[]) => list.map((w) => w.id);
const grp = (id: number, pinned: boolean): ReorderGroup => ({ id, pinned });

// 불변식: 반환 배열의 id 집합이 원본과 동일(보존 + 중복 없음).
function expectSameIdSet(out: { id: number }[], src: { id: number }[]) {
  expect([...out].map((x) => x.id).sort()).toEqual([...src].map((x) => x.id).sort());
  expect(new Set(out.map((x) => x.id)).size).toBe(out.length); // 중복 없음
}

describe("reorderWorkspace — 같은 컨테이너 내 순서 변경", () => {
  it("ungrouped 리스트에서 뒤 항목을 앞 항목 앞으로", () => {
    const list = [ws(1), ws(2), ws(3), ws(4)];
    const out = reorderWorkspace(list, 4, undefined, 2, true); // 4를 2 앞으로
    expect(ids(out)).toEqual([1, 4, 2, 3]);
    expectSameIdSet(out, list);
  });
  it("앞 항목을 뒤 항목 뒤로(before=false)", () => {
    const out = reorderWorkspace([ws(1), ws(2), ws(3), ws(4)], 1, undefined, 3, false); // 1을 3 뒤로
    expect(ids(out)).toEqual([2, 3, 1, 4]);
  });
  it("그룹 body 안 재정렬은 groupId를 유지한다", () => {
    const list = [ws(1, 10), ws(2, 10), ws(3, 10)];
    const out = reorderWorkspace(list, 3, 10, 1, true); // 3을 1 앞으로(같은 그룹)
    expect(ids(out)).toEqual([3, 1, 2]);
    for (const w of out) expect(w.groupId).toBe(10);
  });
  it("원본 배열을 변형하지 않는다(순수)", () => {
    const list = [ws(1), ws(2), ws(3)];
    const snapshot = ids(list);
    reorderWorkspace(list, 3, undefined, 1, true);
    expect(ids(list)).toEqual(snapshot);
  });
});

describe("reorderWorkspace — 자기 자신 위 드롭 no-op", () => {
  it("srcId === anchorId면 순서 불변", () => {
    const list = [ws(1), ws(2), ws(3)];
    expect(ids(reorderWorkspace(list, 2, undefined, 2, true))).toEqual([1, 2, 3]);
    expect(ids(reorderWorkspace(list, 2, undefined, 2, false))).toEqual([1, 2, 3]);
  });
  it("없는 소스는 무변경", () => {
    const list = [ws(1), ws(2)];
    expect(ids(reorderWorkspace(list, 99, undefined, 1, true))).toEqual([1, 2]);
  });
});

describe("reorderWorkspace — 그룹 간 이동", () => {
  it("ungrouped ws를 그룹 멤버 뒤로 넣으면 groupId가 바뀐다", () => {
    const list = [ws(1, 10), ws(2, 10), ws(3), ws(4)];
    const out = reorderWorkspace(list, 3, 10, 2, false); // 3을 그룹10의 2 뒤로
    expect(ids(out)).toEqual([1, 2, 3, 4]);
    expect(out.find((w) => w.id === 3)!.groupId).toBe(10); // 그룹 합류
    expect(out.find((w) => w.id === 4)!.groupId).toBeUndefined(); // 나머지 ungrouped 불변
  });
  it("그룹 멤버를 ungrouped 끝으로 빼면 groupId=undefined", () => {
    const list = [ws(1, 10), ws(2, 10), ws(3), ws(4)];
    const out = reorderWorkspace(list, 1, undefined, null, false); // 1을 ungrouped 끝에
    expect(out.find((w) => w.id === 1)!.groupId).toBeUndefined();
    expect(ids(out)).toEqual([2, 3, 4, 1]); // ungrouped 마지막 멤버(4) 뒤
  });
  it("마지막 멤버가 그룹을 탈출하면 그 멤버 groupId만 풀린다(그룹 해체는 상위 normalizeGroups 담당)", () => {
    const list = [ws(1, 10), ws(2), ws(3)];
    const out = reorderWorkspace(list, 1, undefined, 2, true); // 그룹10 유일 멤버 1을 ungrouped로
    expect(out.find((w) => w.id === 1)!.groupId).toBeUndefined();
    expect(out.every((w) => (w.groupId ?? null) === null)).toBe(true); // 이제 전부 ungrouped
  });
  it("ungrouped↔group 왕복은 원래 위치로 복원 가능", () => {
    const start = [ws(1, 10), ws(2, 10), ws(3), ws(4)];
    const into = reorderWorkspace(start, 3, 10, 2, false); // 3→그룹10
    expect(into.find((w) => w.id === 3)!.groupId).toBe(10);
    const back = reorderWorkspace(into, 3, undefined, 4, false); // 3→ungrouped(4 뒤)
    expect(back.find((w) => w.id === 3)!.groupId).toBeUndefined();
    expect(ids(back)).toEqual([1, 2, 4, 3]);
  });
  it("anchor=null이면 대상 그룹 끝(마지막 동일그룹 멤버 뒤)에 삽입", () => {
    const list = [ws(1, 10), ws(2, 10), ws(3), ws(4)]; // 그룹10=[1,2]
    const out = reorderWorkspace(list, 4, 10, null, false);
    expect(out.find((w) => w.id === 4)!.groupId).toBe(10);
    expect(ids(out)).toEqual([1, 2, 4, 3]); // 2(그룹10 마지막) 뒤에 삽입
  });
});

describe("reorderWorkspace — pending 불변", () => {
  it("pending ws의 pending·groupId 속성은 이동에 영향받지 않는다", () => {
    const list = [ws(1), ws(9, 10, true), ws(2)];
    const out = reorderWorkspace(list, 2, undefined, 1, true); // 2를 1 앞으로
    const p = out.find((w) => w.id === 9)!;
    expect(p.pending).toBe(true);
    expect(p.groupId).toBe(10);
    expectSameIdSet(out, list);
  });
});

describe("reorderGroup — 같은 tier 순서 변경", () => {
  it("unpinned tier 안에서 순서 교체", () => {
    const gs = [grp(1, true), grp(2, true), grp(3, false), grp(4, false)];
    const out = reorderGroup(gs, 4, 3, true); // 4를 3 앞으로
    expect(ids(out)).toEqual([1, 2, 4, 3]);
    expectSameIdSet(out, gs);
  });
  it("pinned tier 안에서 뒤로 이동(before=false)", () => {
    const gs = [grp(1, true), grp(2, true), grp(3, false)];
    const out = reorderGroup(gs, 1, 2, false); // 1을 2 뒤로
    expect(ids(out)).toEqual([2, 1, 3]);
  });
  it("자기 위 드롭 no-op", () => {
    const gs = [grp(1, true), grp(2, false)];
    expect(ids(reorderGroup(gs, 2, 2, true))).toEqual([1, 2]);
  });
});

describe("reorderGroup — 다른 tier 드롭 시 pinned 플래그 불변 + tier 클램프", () => {
  it("unpinned 그룹을 pinned 앵커에 드롭 → unpinned tier 맨 위로 클램프(플래그 불변)", () => {
    const gs = [grp(1, true), grp(2, true), grp(3, false), grp(4, false)];
    const out = reorderGroup(gs, 4, 1, true); // unpinned 4를 pinned 1 위로
    expect(out.find((g) => g.id === 4)!.pinned).toBe(false); // 플래그 불변
    expect(ids(out)).toEqual([1, 2, 4, 3]); // unpinned tier 맨 위(첫 unpinned 3 앞)
  });
  it("pinned 그룹을 unpinned 앵커에 드롭 → pinned tier 맨 아래로 클램프(플래그 불변)", () => {
    const gs = [grp(1, true), grp(2, true), grp(3, false), grp(4, false)];
    const out = reorderGroup(gs, 1, 4, false); // pinned 1을 unpinned 4 아래로
    expect(out.find((g) => g.id === 1)!.pinned).toBe(true); // 플래그 불변
    expect(ids(out)).toEqual([2, 1, 3, 4]); // pinned tier 맨 아래(마지막 pinned 2 뒤)
  });
  it("anchor=null이면 자기 tier 끝에 추가", () => {
    const gs = [grp(1, true), grp(2, true), grp(3, false), grp(4, false)];
    expect(ids(reorderGroup(gs, 3, null, false))).toEqual([1, 2, 4, 3]); // unpinned 3을 unpinned 끝으로
  });
});
