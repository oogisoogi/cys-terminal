#!/usr/bin/env python3
"""W1 identity 3중 대조 테스트(리포 커밋 · embed 제외 tests/). 데몬 불요 — subprocess 를 mock 해
build_id·embedded_pack_hash·protocol_version 각 불일치→exit 6+필드명, legacy 필드부재→mismatch,
inconclusive→degraded 채택+저널 기록을 고정 검증한다.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_identity.py  (0=전건 PASS)
"""
import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))

spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

SELF = {"build_id": "abc123", "embedded_pack_hash": "H_SELF", "protocol_version": "1", "version": "0.12.20"}
FAKE_CYS = os.path.join(tempfile.gettempdir(), "fake-cys-identity-test")

_results = []


def check(name, cond, detail=""):
    _results.append((name, cond, detail))
    print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))


def _mk_run(self_id, daemon_id):
    """subprocess.run 대역 — phoenix-identity(self)와 status(daemon) 출력을 스크립트한다. None=미도달(빈 stdout)."""
    def _run(cmd, *a, **kw):
        class R:
            returncode = 0
            stderr = ""
        r = R()
        if "phoenix-identity" in cmd:
            r.stdout = json.dumps(self_id) if self_id is not None else ""
        elif "status" in cmd:
            r.stdout = json.dumps({"daemon": daemon_id}) if daemon_id is not None else ""
        else:
            r.stdout = ""
        return r
    return _run


@contextlib.contextmanager
def _mock_subprocess(self_id, daemon_id):
    orig = m.subprocess.run
    m.subprocess.run = _mk_run(self_id, daemon_id)
    try:
        yield
    finally:
        m.subprocess.run = orig


def _isolated_socket(tmp):
    """격리 소켓 경로 — 저널 write 가 tmp/phoenix 로 향하게(라이브 무접촉)."""
    return os.path.join(tmp, "cys.sock")


def _resolve_expect_exit6(socket, phoenix_cys, self_id, daemon_id):
    """PHOENIX_CYS 경로로 _resolve_cys 호출 → SystemExit(6) 기대. (code, stderr) 반환."""
    m._CYS_IDENTITY = None
    err = io.StringIO()
    old = os.environ.get("PHOENIX_CYS")
    os.environ["PHOENIX_CYS"] = phoenix_cys
    # X_OK/isfile 통과 강제(실 파일 없이도).
    oi, oa = m.os.path.isfile, m.os.access
    m.os.path.isfile = lambda p: True if p == phoenix_cys else oi(p)
    m.os.access = lambda p, mode: True if p == phoenix_cys else oa(p, mode)
    # 재시도 sleep 0(테스트 속도).
    m._IDENTITY_RETRY_SLEEP = 0.0
    code = None
    try:
        with _mock_subprocess(self_id, daemon_id), contextlib.redirect_stderr(err):
            m._resolve_cys(socket)
    except SystemExit as e:
        code = e.code
    finally:
        m.os.path.isfile, m.os.access = oi, oa
        if old is None:
            os.environ.pop("PHOENIX_CYS", None)
        else:
            os.environ["PHOENIX_CYS"] = old
    return code, err.getvalue()


def main():
    tmp = tempfile.mkdtemp(prefix="phoenix-identity-test-")
    socket = _isolated_socket(tmp)

    # ── A. _cys_identity_check 필드별 mismatch(순수 대조) ──
    with _mock_subprocess(SELF, dict(SELF)):
        st, fld, _ = m._cys_identity_check(FAKE_CYS, socket)
    check("A match", st == "match", "%s/%s" % (st, fld))
    for field, bad in [("build_id", "DIFF"), ("embedded_pack_hash", "H_OTHER"), ("protocol_version", "9")]:
        dmn = dict(SELF); dmn[field] = bad
        with _mock_subprocess(SELF, dmn):
            st, fld, _ = m._cys_identity_check(FAKE_CYS, socket)
        check("A mismatch %s" % field, st == "mismatch" and fld == field, "%s/%s" % (st, fld))
    # legacy: 데몬 JSON 에 필드 부재(구버전) → mismatch(필드 부재 검출)
    with _mock_subprocess(SELF, {"build_id": "abc123", "protocol_version": "1"}):  # embedded_pack_hash 없음
        st, fld, _ = m._cys_identity_check(FAKE_CYS, socket)
    check("A legacy 필드부재 → mismatch", st == "mismatch" and fld == "embedded_pack_hash", "%s/%s" % (st, fld))
    # inconclusive: self-report 실패 / 데몬 미도달
    with _mock_subprocess(None, dict(SELF)):
        st, _, _ = m._cys_identity_check(FAKE_CYS, socket)
    check("A inconclusive(self 실패)", st == "inconclusive", st)
    with _mock_subprocess(SELF, None):
        st, _, _ = m._cys_identity_check(FAKE_CYS, socket)
    check("A inconclusive(daemon 미도달)", st == "inconclusive", st)

    # ── B. _resolve_cys 필드별 mismatch → exit 6 + 필드명 stderr ──
    for field, bad in [("build_id", "DIFF"), ("embedded_pack_hash", "H_OTHER"), ("protocol_version", "9")]:
        dmn = dict(SELF); dmn[field] = bad
        code, err = _resolve_expect_exit6(socket, FAKE_CYS, SELF, dmn)
        check("B %s mismatch → exit 6 + 필드명" % field,
              code == 6 and field in err, "code=%s field_in_err=%s" % (code, field in err))
    # legacy 필드부재 → exit 6 + 필드명(embedded_pack_hash)
    code, err = _resolve_expect_exit6(socket, FAKE_CYS, SELF, {"build_id": "abc123", "protocol_version": "1"})
    check("B legacy 필드부재 → exit 6", code == 6 and "embedded_pack_hash" in err,
          "code=%s" % code)

    # ── C. inconclusive → degraded 채택(exit 없음) + cys_identity + 저널 기록 ──
    m._CYS_IDENTITY = None
    m._IDENTITY_RETRY_SLEEP = 0.0
    oi, oa = m.os.path.isfile, m.os.access
    m.os.path.isfile = lambda p: True if p == FAKE_CYS else oi(p)
    m.os.access = lambda p, mode: True if p == FAKE_CYS else oa(p, mode)
    os.environ["PHOENIX_CYS"] = FAKE_CYS
    err = io.StringIO()
    got = None
    try:
        with _mock_subprocess(SELF, None), contextlib.redirect_stderr(err):  # 데몬 미도달=inconclusive
            got = m._resolve_cys(socket)
    except SystemExit as e:
        got = "EXIT:%s" % e.code
    finally:
        m.os.path.isfile, m.os.access = oi, oa
        os.environ.pop("PHOENIX_CYS", None)
    check("C inconclusive → 채택(exit 없음)", got == FAKE_CYS, "got=%r" % got)
    check("C cys_identity=degraded-unverified", m._CYS_IDENTITY == "degraded-unverified", str(m._CYS_IDENTITY))
    check("C stderr degraded 명시", "degraded" in err.getvalue(), err.getvalue().strip()[-80:])
    # 저널 기록 확인
    rj = os.path.join(tmp, "phoenix", "journal-resolve.json")
    recorded = False
    if os.path.exists(rj):
        try:
            ev = json.load(open(rj)).get("events", [])
            recorded = any(e.get("stage") == "resolve_cys" and e.get("status") == "degraded" for e in ev)
        except Exception:
            recorded = False
    check("C degraded 저널 기록", recorded, "journal-resolve.json resolve_cys/degraded")

    # ── D. gate2 BLOCKING: _which('cys') PATH 후보도 identity 게이트를 우회하지 않는다 ──
    #    PHOENIX_CYS 없이 which 가 후보를 잡을 때, mismatch → exit 6(필드명). (과거엔 곧바로 return 으로 우회했다)
    os.environ.pop("PHOENIX_CYS", None)
    ow = m._which
    m._which = lambda name: FAKE_CYS if name == "cys" else ow(name)
    m._IDENTITY_RETRY_SLEEP = 0.0
    dmn = dict(SELF); dmn["build_id"] = "PATHDIFF"
    m._CYS_IDENTITY = None
    err = io.StringIO(); code = None
    try:
        with _mock_subprocess(SELF, dmn), contextlib.redirect_stderr(err):
            m._resolve_cys(socket)
    except SystemExit as e:
        code = e.code
    finally:
        m._which = ow
    check("D PATH(which) 후보 mismatch → exit 6 + 필드명", code == 6 and "build_id" in err.getvalue(),
          "code=%s field_in_err=%s" % (code, "build_id" in err.getvalue()))
    # PATH 후보 match → 채택+verified
    m._which = lambda name: FAKE_CYS if name == "cys" else ow(name)
    m._CYS_IDENTITY = None
    got = None
    try:
        with _mock_subprocess(SELF, dict(SELF)):
            got = m._resolve_cys(socket)
    except SystemExit as e:
        got = "EXIT:%s" % e.code
    finally:
        m._which = ow
    check("D PATH(which) 후보 match → 채택+verified", got == FAKE_CYS and m._CYS_IDENTITY == "verified",
          "got=%r identity=%s" % (got, m._CYS_IDENTITY))

    npass = sum(1 for _, c, _ in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
