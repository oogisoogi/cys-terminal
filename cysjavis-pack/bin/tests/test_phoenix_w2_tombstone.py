#!/usr/bin/env python3
"""W2 옵션A(A-S1) 묘비 조건부 replace 테스트(리포 커밋). observe_and_persist_roster 의 topology→desired
조건부 replace 를 결정론 검증 — read_topology/live/snapshot 을 monkeypatch 해 데몬 불요.

시나리오: ① 옵션A replace(그대로 대입) · ② 데몬 un-tombstone 자동 반영(제3겹 소멸) · ③ rev 역행 가드
(부분절단/조작→replace 생략·desired 보존) · ④ rebase(epoch 변경/rev0→정당 역행 replace) · ⑤ legacy(마커
부재→add-merge·부활 진행).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_w2_tombstone.py  (0=전건 PASS)
"""
import importlib.util, json, os, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

_results = []
def check(n, c, d=""):
    _results.append(c); print(("PASS " if c else "FAIL ") + n + (" | " + d if d else ""))

# 데몬/스냅샷/라이브 무접촉 — 전부 빈 관측으로 고정(묘비 로직만 격리).
m.live_role_surfaces = lambda socket: {}
m._snapshot_roster_entries = lambda socket: {}
m.CYS = "/nonexistent-cys"


def _run(td, name, prev_desired, topo, epoch="sa:E1"):
    """desired_roster.json=prev_desired 로 시드 후 topo(read_topology 대역)로 observe 실행 → 영속본 반환."""
    sd = os.path.join(td, name)
    ph = os.path.join(sd, "phoenix")
    os.makedirs(ph, exist_ok=True)
    sock = os.path.join(sd, "cys.sock")
    with open(os.path.join(ph, "desired_roster.json"), "w") as f:
        json.dump(prev_desired, f)
    m.read_topology = lambda socket: dict(topo)
    m._ACTIVE_EPOCH = epoch
    m.observe_and_persist_roster(sock)
    return json.load(open(os.path.join(ph, "desired_roster.json")))


def main():
    td = tempfile.mkdtemp(prefix="phoenix-w2-")

    # ① 옵션A replace: prev tombstones={old}, rev5 · topology 마커 rev6 tombstones={new}
    #    → desired 는 topology 그대로 대입({new}·add-merge 아님), rev6.
    out = _run(td, "s1", {"roster": {}, "tombstones": ["old"], "tombstones_rev": 5, "daemon_epoch": "sa:E1"},
               {"schema_version": 1, "tombstones_rev": 6, "tombstones": ["new"], "entries": []})
    check("① 옵션A replace: topology 그대로 대입(add-merge 아님)",
          set(out.get("tombstones", [])) == {"new"} and out.get("tombstones_rev") == 6,
          "tombstones=%s rev=%s" % (out.get("tombstones"), out.get("tombstones_rev")))

    # ② un-tombstone 자동 반영: prev {worker} rev5 · topology 마커 rev6 tombstones=[] (데몬 해제)
    #    → desired tombstones=[] (worker 부활 가능·제3겹 소멸).
    out = _run(td, "s2", {"roster": {"worker": {"role": "worker"}}, "tombstones": ["worker"],
                          "tombstones_rev": 5, "daemon_epoch": "sa:E1"},
               {"schema_version": 1, "tombstones_rev": 6, "tombstones": [], "entries": []})
    check("② 데몬 un-tombstone 자동 반영(제3겹 소멸)",
          out.get("tombstones", []) == [] and "worker" in (out.get("roster") or {}),
          "tombstones=%s roster=%s" % (out.get("tombstones"), list((out.get('roster') or {}).keys())))

    # ③ rev 역행 가드(부분절단/조작): prev rev5 epoch E1 · topology 마커 rev3 tombstones=[](같은 epoch)
    #    → 정당근거 없는 역행 → replace 생략, desired 보존({worker}·rev5 유지).
    out = _run(td, "s3", {"roster": {"worker": {"role": "worker"}}, "tombstones": ["worker"],
                          "tombstones_rev": 5, "daemon_epoch": "sa:E1"},
               {"schema_version": 1, "tombstones_rev": 3, "tombstones": [], "entries": []}, epoch="sa:E1")
    check("③ rev 역행 가드: 정당근거 없는 역행→replace 생략·desired 보존",
          set(out.get("tombstones", [])) == {"worker"} and out.get("tombstones_rev") == 5,
          "tombstones=%s rev=%s" % (out.get("tombstones"), out.get("tombstones_rev")))

    # ④ rebase: prev rev5 epoch E1 · topology 마커 rev3 tombstones={fresh}, epoch 변경(E2)
    #    → 정당 역행(epoch 변경) → 강제 rebase 후 replace({fresh}·rev3).
    out = _run(td, "s4", {"roster": {}, "tombstones": ["worker"], "tombstones_rev": 5, "daemon_epoch": "sa:E1"},
               {"schema_version": 1, "tombstones_rev": 3, "tombstones": ["fresh"], "entries": []}, epoch="sa:E2")
    check("④ rebase: epoch 변경 정당 역행→replace(rebase)",
          set(out.get("tombstones", [])) == {"fresh"} and out.get("tombstones_rev") == 3,
          "tombstones=%s rev=%s" % (out.get("tombstones"), out.get("tombstones_rev")))

    # ⑤ legacy: topology 마커 부재 → add-merge(기존 desired 보존 + topology 묘비 병합)·부활 진행.
    out = _run(td, "s5", {"roster": {}, "tombstones": ["old"], "tombstones_rev": 5, "daemon_epoch": "sa:E1"},
               {"tombstones": ["legacyx"], "entries": []})  # schema_version 없음
    check("⑤ legacy(마커 부재)→add-merge·부활 진행", set(out.get("tombstones", [])) == {"old", "legacyx"},
          "tombstones=%s" % out.get("tombstones"))

    import shutil; shutil.rmtree(td, ignore_errors=True)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
