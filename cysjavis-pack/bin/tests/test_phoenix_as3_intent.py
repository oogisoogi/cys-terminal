#!/usr/bin/env python3
"""W2/A-S3 tombstone intent 저널 테스트(리포 커밋). 데몬 다운타임 폴백·observe 멱등 적용·절단 순서 계약
(TOCTOU: topology.json 디스크 영속 확인 후에만 절단)을 결정론 검증. 데몬 불요(cys/read_topology mock).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_as3_intent.py  (0=전건 PASS)
"""
import importlib.util, json, os, sys, tempfile, types

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

_results = []
def check(n, c, d=""):
    _results.append(c); print(("PASS " if c else "FAIL ") + n + (" | " + d if d else ""))

m.live_role_surfaces = lambda socket: {}
m._snapshot_roster_entries = lambda socket: {}


class _R:
    def __init__(self, rc=0, out=""):
        self.returncode = rc; self.stdout = out; self.stderr = ""


def main():
    td = tempfile.mkdtemp(prefix="phoenix-as3-")
    sd = os.path.join(td, "node"); ph = os.path.join(sd, "phoenix"); os.makedirs(ph, exist_ok=True)
    sock = os.path.join(sd, "cys.sock")
    ijp = os.path.join(ph, "tombstone-intents.jsonl")

    # ── A. 데몬 다운타임: cmd_tombstone RPC 실패 → intent 저널 append(desired 직접 쓰기 아님) ──
    m.CYS = os.path.join(td, "nonexistent-cys")  # cys() rc127 → RPC 실패
    args = types.SimpleNamespace(socket=sock, role="ghost", remove=False, dept=False)
    out = m.cmd_tombstone(args)
    check("A 다운타임 → intent 저널 폴백(via)", out.get("via") == "intent-journal", out.get("via"))
    ints = m._read_tombstone_intents(sock)
    check("A intent 저널에 add ghost 기록", any(i.get("op") == "add" and i.get("role") == "ghost" for i in ints),
          str(ints))

    # ── B. observe: 미소화 intent 를 replace 이전 멱등 적용(topology 에 아직 없어도 즉시 반영)·절단 보류 ──
    m.read_topology = lambda socket: {"schema_version": 1, "tombstones_rev": 3, "tombstones": [], "entries": []}
    m._ACTIVE_EPOCH = "sa:E1"
    # RPC 재동기도 실패(데몬 down) → 절단 보류. observe 가 intent(add ghost)를 tombstones 에 멱등 적용.
    roster, tombs = m.observe_and_persist_roster(sock)
    check("B observe 멱등 적용: ghost 폐역 반영", "ghost" in tombs and "ghost" not in roster,
          "tombs=%s" % sorted(tombs))
    check("B 절단 보류(데몬 down): intent 저널 잔존", os.path.exists(ijp))

    # ── C. 절단 순서 TOCTOU: RPC 는 성공하나 topology.json 이 아직 미반영(데몬 영속 전 크래시 모사) → 절단 금지 ──
    m.cys = lambda *a, **k: _R(0, "tombstone ghost set")  # RPC 성공
    m.read_topology = lambda socket: {"schema_version": 1, "tombstones_rev": 3, "tombstones": [], "entries": []}  # ghost 미반영
    truncated = m._resync_intents_if_daemon_up(sock, m._read_tombstone_intents(sock))
    check("C TOCTOU: RPC 성공이나 디스크 미영속 → 절단 금지", truncated is False and os.path.exists(ijp),
          "truncated=%s exists=%s" % (truncated, os.path.exists(ijp)))

    # ── D. 절단: RPC 성공 + topology.json 에 ghost 영속 확인 → intent 절단 ──
    m.read_topology = lambda socket: {"schema_version": 1, "tombstones_rev": 4, "tombstones": ["ghost"], "entries": []}
    truncated = m._resync_intents_if_daemon_up(sock, m._read_tombstone_intents(sock))
    check("D 디스크 영속 확인 후 절단", truncated is True and not os.path.exists(ijp),
          "truncated=%s exists=%s" % (truncated, os.path.exists(ijp)))

    # ── E. remove intent 도 대칭 동작(멱등) ──
    m.CYS = os.path.join(td, "nonexistent-cys")
    m.cys = m.__dict__.get("cys")  # restore 안 함 — remove 는 저널 경로만 확인
    # 직접 저널에 remove intent 기록 후 적용 검증(관대: 저널 append + apply)
    m._append_tombstone_intent(sock, "ghost", True)
    ts = set(["ghost", "keep"])
    m._apply_intents_to_tombstones(m._read_tombstone_intents(sock), ts)
    check("E remove intent 멱등 적용(ghost 제거·keep 보존)", "ghost" not in ts and "keep" in ts, str(sorted(ts)))

    import shutil; shutil.rmtree(td, ignore_errors=True)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
