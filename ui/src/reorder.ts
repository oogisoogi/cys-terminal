// 사이드바 탭·그룹 순서 변경의 순수 배열 변형 로직 (DOM 무접촉 — main.ts의 드래그 핸들러가 호출).
//
// 렌더는 flat 배열을 filter로 컨테이너별(그룹/ungrouped·tier)로 모으므로, 한 컨테이너 안의
// 시각 순서 = 그 컨테이너 멤버들의 배열 상대 순서다. 따라서 "이동" = splice로 상대 순서를 바꾸는 것.
// 불변식: id 보존·중복 없음·pending 불변(pending은 드래그 대상·드롭 타깃이 아니므로 그 속성을 건드리지 않는다).

export interface ReorderWs {
  id: number;
  groupId?: number; // undefined = ungrouped
  pending?: boolean;
}

export interface ReorderGroup {
  id: number;
  pinned: boolean;
}

// groupId의 undefined/null을 ungrouped로 동일 취급(코드베이스가 둘 다 씀 — normalizeGroups는 != null).
const groupOf = (w: ReorderWs): number | null => w.groupId ?? null;

// srcId ws를 destGroupId 컨테이너로 옮기고 위치를 반영한다.
//   anchorId != null : 그 ws의 앞(before=true)/뒤(before=false)에 삽입 — 같은 컨테이너 멤버 기준.
//   anchorId == null : 대상 그룹의 마지막 멤버 뒤(그룹 끝에 추가). 멤버 없으면 배열 끝.
// destGroupId==undefined면 ungrouped로 이동. 그룹 이동일 때만 src를 groupId 교체 클론(그 외엔 객체 재사용).
export function reorderWorkspace<T extends ReorderWs>(
  list: T[],
  srcId: number,
  destGroupId: number | undefined,
  anchorId: number | null,
  before: boolean,
): T[] {
  const arr = list.slice();
  const srcIdx = arr.findIndex((w) => w.id === srcId);
  if (srcIdx < 0) return arr; // 없는 소스 → 무변경
  if (anchorId != null && srcId === anchorId) return arr; // 자기 자신 위 드롭 = no-op

  const src = arr[srcIdx];
  const moved: T =
    groupOf(src) === (destGroupId ?? null)
      ? src // 같은 그룹 내 재정렬 → groupId 그대로(객체 identity 유지)
      : ({ ...src, groupId: destGroupId } as T); // 그룹 이동 → groupId만 바꾼 클론
  arr.splice(srcIdx, 1);

  let at: number;
  if (anchorId != null) {
    const ai = arr.findIndex((w) => w.id === anchorId);
    at = ai < 0 ? arr.length : before ? ai : ai + 1; // 앵커 소실 시 끝에
  } else {
    // 대상 그룹의 마지막 동일그룹 멤버 뒤 = 그룹 끝. 멤버 없으면 배열 끝.
    let last = -1;
    for (let i = 0; i < arr.length; i++) if (groupOf(arr[i]) === (destGroupId ?? null)) last = i;
    at = last >= 0 ? last + 1 : arr.length;
  }
  arr.splice(at, 0, moved);
  return arr;
}

// srcId 그룹을 groups 배열에서 재정렬한다. tier(pinned/unpinned)는 분리 유지 — pinned 플래그는
// 절대 바꾸지 않고, 다른 tier에 드롭하면 자기 tier 경계로 클램프한다.
//   같은 tier 앵커 : 그 그룹의 앞/뒤.
//   다른 tier 앵커 : 자기 tier의 앵커 방향 끝(pinned가 unpinned로 향하면 pinned tier 맨 아래,
//                    unpinned가 pinned로 향하면 unpinned tier 맨 위).
//   anchorId==null : 자기 tier 끝에 추가.
// 렌더는 pinned filter → unpinned filter 순이라 tier 간 flat 위치는 표시에 영향 없다(같은 tier 상대 순서만 유효).
export function reorderGroup<T extends ReorderGroup>(
  groups: T[],
  srcId: number,
  anchorId: number | null,
  before: boolean,
): T[] {
  const arr = groups.slice();
  const srcIdx = arr.findIndex((g) => g.id === srcId);
  if (srcIdx < 0) return arr;
  if (anchorId != null && srcId === anchorId) return arr; // 자기 위 드롭 = no-op

  const src = arr[srcIdx];
  const anchor = anchorId != null ? arr.find((g) => g.id === anchorId) ?? null : null;
  arr.splice(srcIdx, 1);

  let at: number;
  if (anchor && anchor.pinned === src.pinned) {
    const ai = arr.findIndex((g) => g.id === anchorId);
    at = before ? ai : ai + 1;
  } else if (src.pinned) {
    // 다른 tier(anchor=unpinned) 또는 anchor 없음 → pinned tier 끝(마지막 pinned 뒤). pinned 없으면 맨 앞.
    let last = -1;
    for (let i = 0; i < arr.length; i++) if (arr[i].pinned) last = i;
    at = last + 1;
  } else if (anchor) {
    // 다른 tier(anchor=pinned) → unpinned tier 맨 위(첫 unpinned 앞). unpinned 없으면 끝.
    const first = arr.findIndex((g) => !g.pinned);
    at = first < 0 ? arr.length : first;
  } else {
    at = arr.length; // anchor 없음 + unpinned → unpinned tier 끝(배열 끝)
  }
  arr.splice(at, 0, src);
  return arr;
}
