#!/usr/bin/env python3
"""무중단(재시작 0) 팩 업데이트 실측 E2E — DESIGN-noshutdown-pack-update §5-2·§7-⑥.

라이브 cysd에 `cys pack-update --from <테스트팩>`을 가해 **반영 전/후 스냅샷 동등성**을 검증한다:

  | 생존 대상 | RPC(실측)         | 읽는 필드                     | 합격 조건            |
  |-----------|-------------------|-------------------------------|----------------------|
  | cysd      | system.identify   | daemon_pid                    | 전/후 동일           |
  | 세션      | surface.list      | surfaces[].surface_id·exited  | 집합 불변·전부 false |
  | 팩 반영   | 파일              | <pack_dir>/.pack-version      | 새 버전으로 범프     |

★불합격(hard fail): daemon_pid 변동 = 데몬 재시작 = 무중단 위반 = FAIL.
  (cys-app pid는 OS 프로세스 — 라이브 Tauri 앱이 있을 때만 의미. 여기선 cysd·세션 생존을 본다.)

라이브 데몬이 없거나 서명된 테스트 팩(--from)이 없으면 **graceful skip**(로직은 완성, exit 0).

실행:
  python3 docs/noshutdown_verify.py --from /path/to/testpack   # pack.tar.gz+manifest+.minisig
  CYS_SOCKET=... CYS_PACK_DIR=... python3 docs/noshutdown_verify.py --from ...
"""
import argparse
import json
import os
import socket
import subprocess
import sys


def default_socket():
    """src/lib.rs::socket_path()와 동일 규칙 — CYS_SOCKET 우선, 없으면 ~/.local/state/cys/cys.sock."""
    for k in ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET"):
        v = os.environ.get(k)
        if v:
            return v
    home = os.path.expanduser("~")
    return os.path.join(home, ".local", "state", "cys", "cys.sock")


def default_pack_dir():
    """src/pack.rs::pack_dir()와 동일 규칙 — CYS_PACK_DIR 우선, 없으면 ~/.cys/pack."""
    for k in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(k)
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys", "pack")


def find_cys_bin():
    """cys 바이너리 — target/release > target/debug > PATH."""
    here = os.path.dirname(os.path.abspath(__file__))
    for cand in (
        os.path.join(here, "..", "target", "release", "cys"),
        os.path.join(here, "..", "target", "debug", "cys"),
    ):
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return os.path.abspath(cand)
    return "cys"  # PATH 폴백


class DaemonUnavailable(Exception):
    pass


def rpc(sock_path, method, params, timeout=5.0):
    """Unix 소켓 1-shot JSON-RPC(개행 종단) — docs/*_e2e.py 패턴."""
    try:
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(timeout)
        s.connect(sock_path)
    except OSError as e:
        raise DaemonUnavailable(f"소켓 연결 실패 {sock_path}: {e}")
    try:
        s.sendall((json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        s.close()
    if not buf:
        raise DaemonUnavailable(f"빈 응답({method})")
    resp = json.loads(buf)
    if "error" in resp and resp["error"]:
        raise RuntimeError(f"RPC 오류({method}): {resp['error']}")
    return resp.get("result", resp)


def read_pack_version(pack_dir):
    p = os.path.join(pack_dir, ".pack-version")
    try:
        with open(p) as f:
            return f.read().strip()
    except OSError:
        return ""


def snapshot(sock_path, pack_dir):
    """무중단 불변식 스냅샷: daemon_pid + {surface_id: exited} + .pack-version."""
    ident = rpc(sock_path, "system.identify", {})
    daemon_pid = ident.get("daemon_pid")
    surfaces = rpc(sock_path, "surface.list", {}).get("surfaces", [])
    surf = {int(s["surface_id"]): bool(s.get("exited", False)) for s in surfaces if "surface_id" in s}
    return {
        "daemon_pid": daemon_pid,
        "surfaces": surf,
        "pack_version": read_pack_version(pack_dir),
    }


def parse_semver(v):
    """src/pack.rs::parse_semver와 동일 규칙 — 'v' 접두 제거, '-'/'+' suffix 분리, major 결측=None."""
    v = (v or "").strip().lstrip("v")
    parts = v.split(".")
    if not parts or parts[0] == "":
        return None
    out = []
    for i in range(3):
        seg = parts[i] if i < len(parts) else "0"
        seg = seg.split("-")[0].split("+")[0]
        if not seg.isdigit():
            if i == 0:
                return None
            seg = "0"
        out.append(int(seg))
    return tuple(out)


def version_bumped(before, after):
    """after가 before보다 strictly-newer(semver). 둘 다 파싱되면 비교, 아니면 문자열 상이로 폴백."""
    b, a = parse_semver(before), parse_semver(after)
    if a is not None and b is not None:
        return a > b
    return bool(after) and after != before


def main():
    ap = argparse.ArgumentParser(description="무중단 팩 업데이트 실측 E2E")
    ap.add_argument("--from", dest="from_dir",
                    help="테스트 팩 디렉터리(pack.tar.gz + pack-manifest.json + pack-manifest.json.minisig)")
    ap.add_argument("--socket", default=default_socket(), help="cysd Unix 소켓 경로")
    ap.add_argument("--pack-dir", default=default_pack_dir(), help="설치 팩 디렉터리(.pack-version 위치)")
    args = ap.parse_args()

    fails = []

    def check(name, cond, detail=""):
        tag = "PASS" if cond else "FAIL"
        print(f"[{tag}] {name}" + (f" — {detail}" if detail and not cond else ""))
        if not cond:
            fails.append(name)

    # 게이트 0: 라이브 데몬 — 없으면 graceful skip(로직 완성·exit 0).
    try:
        rpc(args.socket, "system.ping", {})
    except (DaemonUnavailable, OSError) as e:
        print(f"[SKIP] 라이브 cysd 없음({args.socket}): {e} — 무중단 실측 생략(로직은 완성).")
        return 0

    # 게이트 0': 서명된 테스트 팩 — 없으면 graceful skip.
    if not args.from_dir:
        print("[SKIP] --from 미지정 — 서명된 테스트 팩이 있어야 실측 가능(로직은 완성).")
        return 0
    needed = ["pack.tar.gz", "pack-manifest.json", "pack-manifest.json.minisig"]
    missing = [f for f in needed if not os.path.isfile(os.path.join(args.from_dir, f))]
    if missing:
        print(f"[SKIP] 테스트 팩 불완전({args.from_dir}) — 누락 {missing}. 실측 생략(로직은 완성).")
        return 0

    # 반영 전 스냅샷.
    before = snapshot(args.socket, args.pack_dir)
    print(f"[snap-before] daemon_pid={before['daemon_pid']} "
          f"surfaces={sorted(before['surfaces'])} pack_version={before['pack_version']!r}")
    # 반영 전에도 모든 세션이 살아있어야(exited:false) 비교 기준이 유효하다.
    check("반영 전 전 세션 생존(exited:false)",
          all(not e for e in before["surfaces"].values()),
          f"exited 세션 존재: {[s for s, e in before['surfaces'].items() if e]}")

    # pack-update 실행.
    cys = find_cys_bin()
    print(f"[run] {cys} pack-update --from {args.from_dir}")
    proc = subprocess.run(
        [cys, "pack-update", "--from", args.from_dir],
        capture_output=True, text=True,
        env=dict(os.environ, CYS_SOCKET=args.socket, CYS_PACK_DIR=args.pack_dir),
    )
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    check("pack-update 종료코드 0", proc.returncode == 0, f"exit={proc.returncode}")

    # 반영 후 스냅샷.
    after = snapshot(args.socket, args.pack_dir)
    print(f"[snap-after]  daemon_pid={after['daemon_pid']} "
          f"surfaces={sorted(after['surfaces'])} pack_version={after['pack_version']!r}")

    # ★무중단 불변식 검증.
    check("cysd 생존 — daemon_pid 동일(재시작 0)",
          before["daemon_pid"] is not None and before["daemon_pid"] == after["daemon_pid"],
          f"{before['daemon_pid']} != {after['daemon_pid']} (재시작 = 무중단 위반)")
    check("세션 집합 불변 — surface_id set 동일",
          set(before["surfaces"]) == set(after["surfaces"]),
          f"before={sorted(before['surfaces'])} after={sorted(after['surfaces'])}")
    check("반영 후 전 세션 생존(exited:false)",
          all(not e for e in after["surfaces"].values()),
          f"exited 세션 존재: {[s for s, e in after['surfaces'].items() if e]}")
    check("팩 반영 — .pack-version 범프",
          version_bumped(before["pack_version"], after["pack_version"]),
          f"{before['pack_version']!r} → {after['pack_version']!r} (범프 안 됨)")

    if fails:
        print(f"\n❌ FAIL {len(fails)}건: {fails}")
        return 1
    print("\n✅ 무중단 검증 통과 — cysd·세션 전부 생존, 팩만 갱신(재시작 0).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
