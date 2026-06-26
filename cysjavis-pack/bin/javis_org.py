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

def v_schema(m):
    errs = []
    if not isinstance(m, dict): return ["매니페스트가 객체 아님"]
    if m.get("kind") != "org-manifest": errs.append("kind != 'org-manifest'")
    if m.get("manifest_version") != 1: errs.append("manifest_version != 1")
    if m.get("reconcile_mode", "additive") != "additive":
        errs.append("MVP reconcile_mode는 additive만(exact는 후속)")
    src = m.get("source") or {}
    if not src.get("design_doc"): errs.append("source.design_doc 누락")
    if not src.get("design_doc_sha256"): errs.append("source.design_doc_sha256 누락")
    for key in ("departments", "tasks"):
        if not isinstance(m.get(key), list): errs.append(f"필수 키 누락/배열아님: {key}")
    for i, d in enumerate(m.get("departments") or []):
        for f in ("key", "display", "account", "cwd", "mission_md", "source_quote"):
            if not d.get(f): errs.append(f"departments[{i}].{f} 누락")
    for i, t in enumerate(m.get("tasks") or []):
        for f in ("dept", "task", "scope", "source_quote"):
            if not t.get(f): errs.append(f"tasks[{i}].{f} 누락")
        if t.get("to", "worker") not in ALLOWED_ROLES:
            errs.append(f"tasks[{i}].to enum 위반: {t.get('to')}")
    return errs

def v_refs(depts, tasks):
    keys = {d.get("key") for d in depts}
    return [f"tasks[{i}].dept '{t.get('dept')}' 미존재(참조 무결성)"
            for i, t in enumerate(tasks) if t.get("dept") not in keys]

def v_sha256(actual, expected):
    return [] if actual == expected else [f"design_doc sha256 불일치(SOT 드리프트): 실제 {actual[:12]}… ≠ 매니페스트 {expected[:12]}…"]

def validate_manifest(m, doc_text=None, catalog=None):
    errs = v_schema(m)
    if errs: return errs  # 스키마 깨지면 이후 검사 무의미
    catalog = catalog if catalog is not None else load_json(CATALOG, {})
    if doc_text is None:
        dd = expand(m["source"]["design_doc"])
        doc_text = open(dd, encoding="utf-8").read() if os.path.exists(dd) else ""
        errs += v_sha256(sha256_text(doc_text) if doc_text else "", m["source"]["design_doc_sha256"])
    errs += v_quote_binding(m.get("departments", []), doc_text, catalog)
    errs += v_quote_binding(  # tasks도 동일 doc 존재성(정체결속은 dept만)
        [{"key":t.get("dept"),"display":"","account":"","cwd":"","source_quote":t.get("source_quote","")}
         for t in m.get("tasks", [])], doc_text, {"accounts":{},"departments":{}})
    errs += v_refs(m.get("departments", []), m.get("tasks", []))
    return errs

def cmd_validate(path):
    try:
        m = load_json(path)
    except Exception as e:
        sys.stderr.write(f"[validate] 매니페스트 로드 실패: {e}\n"); return 2
    errs = validate_manifest(m)
    if errs:
        sys.stderr.write("[validate] FAIL:\n" + "\n".join(f"  - {e}" for e in errs) + "\n")
        return 1
    print(json.dumps({"validate": "ok", "departments": len(m["departments"]),
                      "tasks": len(m["tasks"])}, ensure_ascii=False))
    return 0

def _norm(s): return " ".join((s or "").split())

def v_quote_binding(depts, doc_text, catalog):
    """F1: quote 존재+정체결속+key↔account+cwd패턴+span유일+길이/고유성."""
    errs = []
    ndoc = _norm(doc_text)
    accounts = (catalog.get("accounts") or {}).keys()
    cat_depts = catalog.get("departments") or {}
    seen_quotes = {}
    for i, d in enumerate(depts):
        key, disp, acct = d.get("key",""), d.get("display",""), d.get("account","")
        q = _norm(d.get("source_quote", ""))
        tag = f"departments[{i}]({key})"
        # 길이(F3) → 존재 → 고유성(F3) 순 (존재성을 고유성보다 먼저 — 부재 분기 도달 보장)
        if len(q) < MIN_QUOTE:
            errs.append(f"{tag}: source_quote 길이<{MIN_QUOTE}")
        elif q not in ndoc:
            errs.append(f"{tag}: source_quote 부재(doc에 없음)")
        elif ndoc.count(q) != 1:
            errs.append(f"{tag}: source_quote 고유성 위반(doc 내 {ndoc.count(q)}회)")
        else:
            # 정체 결속: quote가 display 또는 key 변별토큰 포함
            if disp not in q and key not in q:
                errs.append(f"{tag}: quote가 부서 정체(display/key) 미포함 — 결속 실패")
            # span 유일성
            if q in seen_quotes:
                errs.append(f"{tag}: source_quote가 {seen_quotes[q]}와 동일 span 재사용")
            seen_quotes[q] = tag
        # key↔account 일관성
        if key in cat_depts:
            want = cat_depts[key].get("account")
            if want and acct != want:
                errs.append(f"{tag}: account 오배정({acct}≠catalog {want})")
        elif acct not in accounts:
            errs.append(f"{tag}: 신규 key의 account '{acct}'가 승인 accounts에 없음(박사님 승인 필요)")
        # cwd 규약 경로
        cwd = d.get("cwd", "")
        if disp and not cwd.replace("\\", "/").endswith(f"Desktop/CYSjavis/{disp}"):
            errs.append(f"{tag}: cwd가 규약 경로($HOME/Desktop/CYSjavis/{disp}) 불일치")
    return errs

def self_test():
    failures = []
    def chk(name, cond, msg=""):
        if not cond: failures.append(f"{name}: {msg}")
    # Task별로 케이스가 여기 누적된다.
    # --- Task2: v_schema ---
    good_dept = {"key":"future-research","display":"미래연구부","account":"cysinsight",
                 "cwd":"$HOME/Desktop/CYSjavis/미래연구부","mission_md":"# m","source_quote":"x"}
    m_ok = {"manifest_version":1,"kind":"org-manifest","reconcile_mode":"additive",
            "source":{"design_doc":"/d","design_doc_sha256":"a"},
            "departments":[good_dept],"tasks":[]}
    chk("schema-ok", v_schema(m_ok) == [], f"errs={v_schema(m_ok)}")
    chk("schema-bad-kind", any("kind" in e for e in v_schema({**m_ok,"kind":"x"})), "kind 위반 미검출")
    chk("schema-bad-to", any("to" in e for e in v_schema({**m_ok,
        "tasks":[{"dept":"future-research","to":"ceo","task":"t","scope":"s","source_quote":"q"}]})),
        "to enum 위반 미검출")
    chk("schema-miss-field", any("departments" in e for e in v_schema({k:v for k,v in m_ok.items() if k!="departments"})),
        "필수키 누락 미검출")
    # --- Task3: v_quote_binding (F1) ---
    doc = ("미래연구부는 모든 통찰의 원천 엔진이다. "
           "저술콘텐츠부는 통찰을 칼럼과 책으로 대중에 전파한다. "
           "이 문장은 부서 정체 토큰을 포함하지 않는 충분히 긴 고유 문장이다.")
    cat = {"accounts":{"cysinsight":"x","ysfuture":"y"},
           "departments":{
             "future-research":{"display":"미래연구부","account":"cysinsight"},
             "authoring":{"display":"저술콘텐츠부","account":"ysfuture"}}}
    d_ok = {"key":"future-research","display":"미래연구부","account":"cysinsight",
            "cwd":"$HOME/Desktop/CYSjavis/미래연구부",
            "source_quote":"미래연구부는 모든 통찰의 원천 엔진이다."}
    chk("f1-ok", v_quote_binding([d_ok], doc, cat) == [], f"errs={v_quote_binding([d_ok],doc,cat)}")
    # 오귀속: 실재(미래연구부) 문장을 엉뚱한 key/account(authoring/cysinsight)에 붙임 → FAIL
    d_mis = {**d_ok, "key":"authoring", "display":"미래연구부", "account":"cysinsight"}
    chk("f1-misattr", v_quote_binding([d_mis], doc, cat) != [], "오귀속 미검출")
    # key↔account 불일치 (authoring은 ysfuture여야)
    d_acct = {"key":"authoring","display":"저술콘텐츠부","account":"cysinsight",
              "cwd":"$HOME/Desktop/CYSjavis/저술콘텐츠부",
              "source_quote":"저술콘텐츠부는 통찰을 칼럼과 책으로 대중에 전파한다."}
    chk("f1-account", any("account" in e for e in v_quote_binding([d_acct], doc, cat)), "계정 오배정 미검출")
    # quote가 부서정체 토큰 미포함
    d_noid = {**d_ok, "source_quote":"이 문장은 부서 정체 토큰을 포함하지 않는 충분히 긴 고유 문장이다."}
    chk("f1-noident", any("정체" in e for e in v_quote_binding([d_noid], doc, cat)), "정체 결속 미검출")
    # quote가 doc에 없음
    d_fab = {**d_ok, "source_quote":"존재하지 않는 긴 문장 어쩌고저쩌고 일이삼사오육칠팔."}
    chk("f1-absent", any("부재" in e for e in v_quote_binding([d_fab], doc, cat)), "부재 quote 미검출")
    # span 재사용: 두 부서가 같은 quote
    d_dup1 = {**d_ok}
    d_dup2 = {**d_ok, "key":"authoring", "display":"미래연구부"}
    chk("f1-span", any("재사용" in e for e in v_quote_binding([d_dup1,d_dup2], doc, cat)), "span 재사용 미검출")
    # 짧은 quote
    d_short = {**d_ok, "source_quote":"미래연구부"}
    chk("f1-short", any("길이" in e or "고유" in e for e in v_quote_binding([d_short], doc, cat)), "짧은 quote 미검출")
    # --- Task4: v_refs / v_sha256 / validate_manifest ---
    chk("refs-ok", v_refs([good_dept],[{"dept":"future-research","to":"worker","task":"t","scope":"s","source_quote":"q"}])==[], "정상 참조 오류")
    chk("refs-bad", v_refs([good_dept],[{"dept":"no-such","to":"worker","task":"t","scope":"s","source_quote":"q"}])!=[], "붕뜬 task 미검출")
    chk("sha-ok", v_sha256("abc","abc")==[], "sha 일치 오탐")
    chk("sha-bad", v_sha256("abc","def")!=[], "sha 불일치 미검출")
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
    if args.cmd == "validate": return cmd_validate(args.manifest)
    return 2  # apply·status·destroy는 Task 6·7·9에서 배선

if __name__ == "__main__":
    sys.exit(main())
