#!/usr/bin/env python3
"""javis_org.py — 부서 자동 편성 브리지 (org-manifest → 검증·적용·착수확인·삭제).
설계: multi-master-ceo/2026-06-26-org-provisioning-design.md (v2)
하우스스타일: javis_manifest.py (--self-test 밀폐 검증)
exit: 0=성공 1=위반/실패 2=입출력 3=권한(CSO아님) 4=대상없음
"""
import argparse, json, os, sys, hashlib, fcntl, subprocess, tempfile, tarfile, time

HOME = os.path.expanduser("~")
CATALOG = os.environ.get("CYS_DEPT_CATALOG", f"{HOME}/.cys/dept-catalog.json")
DEPTS = os.environ.get("CYS_DEPTS_JSON", f"{HOME}/.cys/depts.json")
MISSIONS = os.environ.get("CYS_DEPT_MISSIONS", f"{HOME}/.cys/dept-missions")
ALLOWED_ROLES = ("worker", "reviewer", "cso")  # tasks[].to enum
MIN_QUOTE = 20  # source_quote 최소 길이(F3)

def expand(p): return os.path.expandvars(os.path.expanduser(p)) if p else p

def load_json(path, default=None):
    if not os.path.exists(path):
        if default is not None: return default
        raise FileNotFoundError(path)
    with open(path, encoding="utf-8") as f:
        return json.load(f)

def sha256_text(s): return hashlib.sha256(s.encode("utf-8")).hexdigest()
def sha256_file(path):
    with open(path, "rb") as f: return hashlib.sha256(f.read()).hexdigest()

def require_cso():
    if os.environ.get("CYS_ROLE") != "cso":
        sys.stderr.write("[javis_org] ★CSO 전용: apply/destroy는 CYS_ROLE=cso에서만(부서 mutation 단일소유). CSO에 위임하라.\n")
        sys.exit(3)

def self_test():
    failures = []
    def chk(name, cond, msg=""):
        if not cond: failures.append(f"{name}: {msg}")
    # Task별로 케이스가 여기 누적된다.
    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False))
    return 1 if failures else 0

def main():
    ap = argparse.ArgumentParser(description="부서 자동 편성 브리지")
    ap.add_argument("--self-test", action="store_true", help="결정론 자기검증")
    sub = ap.add_subparsers(dest="cmd")
    v = sub.add_parser("validate", help="org-manifest 검증 (0=준수 1=위반 2=입출력)")
    v.add_argument("manifest")
    a = sub.add_parser("apply", help="매니페스트 적용 (CSO 전용)")
    a.add_argument("manifest")
    s = sub.add_parser("status", help="부서 착수확인 집계")
    s.add_argument("manifest", nargs="?")
    d = sub.add_parser("destroy", help="부서 삭제 (CSO 전용)")
    d.add_argument("--dept"); d.add_argument("--all", action="store_true")
    d.add_argument("--purge", action="store_true")
    d.add_argument("--purge-workdir", action="store_true")
    args = ap.parse_args()
    if args.self_test: return self_test()
    if not args.cmd: ap.print_help(); return 2
    return 2  # 각 명령은 Task 4·6·7·9에서 배선

if __name__ == "__main__":
    sys.exit(main())
