#!/usr/bin/env python3
"""W6/E2 정기화 배선 검증(리포 커밋) — 임베드 schedule.json 에 phoenix 세대 스냅샷 6h + 주간 격리 드릴
잡이 정석 배선(데몬 핫리로드 스키마)돼 있는지 결정론 확인. cysd schedule.rs Job 스키마(serde default)와 정합.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_e2_schedule.py  (0=전건 PASS)
"""
import json, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SCHED = os.path.normpath(os.path.join(HERE, "..", "..", "schedule.json"))

_results = []
def check(n, c, d=""):
    _results.append(c); print(("PASS " if c else "FAIL ") + n + (" | " + d if d else ""))


def main():
    d = json.load(open(SCHED))
    jobs = {j["id"]: j for j in d.get("jobs", [])}

    # ① 6h 세대 스냅샷 정기화(P2-4) — snapshot 명령·6시간 주기.
    snap = jobs.get("phoenix-snapshot-6h")
    check("① phoenix-snapshot-6h 잡 존재", snap is not None)
    if snap:
        check("① 주기 6h(360분)", snap.get("every_minutes") == 360, str(snap.get("every_minutes")))
        check("① javis_state_snapshot snapshot 호출", "javis_state_snapshot.py" in (snap.get("text_command") or "")
              and "snapshot" in (snap.get("text_command") or ""))

    # ② 주간 격리 드릴 — self-test(라이브 무접촉)·7일 주기.
    drill = jobs.get("phoenix-drill-weekly")
    check("② phoenix-drill-weekly 잡 존재", drill is not None)
    if drill:
        check("② 주기 7일(10080분)", drill.get("every_minutes") == 10080, str(drill.get("every_minutes")))
        check("② self-test(격리·라이브 무접촉) 호출", "self-test" in (drill.get("text_command") or ""))

    # ③ 스키마 정합 — cysd Job(serde default): action·to·if_absent·every_minutes·text_command 만 사용(미지 키 없음).
    allowed = {"id", "time", "days", "in", "every_minutes", "action", "to", "text", "text_command",
               "if_absent", "launch"}
    for jid in ("phoenix-snapshot-6h", "phoenix-drill-weekly"):
        j = jobs.get(jid) or {}
        unknown = set(j.keys()) - allowed
        check("③ %s 키 스키마 정합(미지 키 0)" % jid, not unknown, "unknown=%s" % unknown)

    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
