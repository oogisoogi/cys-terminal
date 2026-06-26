#!/usr/bin/env bash
# 격리 HOME E2E: compile더미→validate→(apply/status/destroy는 dry 검증). 라이브 데몬 무접촉.
set -euo pipefail
T=$(mktemp -d)
export CYS_DEPT_CATALOG="$T/catalog.json"
export CYS_DEPTS_JSON="$T/depts.json"
export CYS_DEPT_MISSIONS="$T/missions"
BIN="$(cd "$(dirname "$0")" && pwd)/javis_org.py"

# 더미 합의 md + 매니페스트
DOC="$T/org-design.md"
cat > "$DOC" <<'MD'
# 조직 설계
미래연구부는 모든 통찰의 원천 엔진으로 가동한다 충분히 길게.
첫 작업: 미래연구부는 종교·교회의 미래 환경 스캐닝을 수행한다 충분히 길게.
MD
SHA=$(python3 -c "import hashlib;print(hashlib.sha256(open('$DOC',encoding='utf-8').read().encode()).hexdigest())")
echo '{"version":1,"accounts":{"cysinsight":"x","ysfuture":"y"},"departments":{}}' > "$CYS_DEPT_CATALOG"
cat > "$T/m.json" <<JSON
{"manifest_version":1,"kind":"org-manifest","reconcile_mode":"additive",
 "source":{"design_doc":"$DOC","design_doc_sha256":"$SHA"},
 "departments":[{"key":"future-research","display":"미래연구부","account":"cysinsight",
   "cwd":"$T/Desktop/CYSjavis/미래연구부","mission_md":"# 미션","source_quote":"미래연구부는 모든 통찰의 원천 엔진으로 가동한다 충분히 길게."}],
 "tasks":[{"dept":"future-research","to":"worker","task":"환경스캐닝","scope":"_round/",
   "source_quote":"첫 작업: 미래연구부는 종교·교회의 미래 환경 스캐닝을 수행한다 충분히 길게."}]}
JSON

echo "== self-test =="; python3 "$BIN" --self-test
echo "== validate (PASS 기대) =="; python3 "$BIN" validate "$T/m.json"
echo "== validate 오귀속 (FAIL 기대) =="
python3 -c "import json;m=json.load(open('$T/m.json'));m['departments'][0]['key']='authoring';json.dump(m,open('$T/bad.json','w'))"
if python3 "$BIN" validate "$T/bad.json"; then echo "E2E FAIL: 오귀속이 통과됨"; exit 1; else echo "  → 기대대로 FAIL"; fi
echo "== apply CSO 게이트 (exit3 기대·CYS_ROLE 없음) =="
if CYS_ROLE= python3 "$BIN" apply "$T/m.json"; then echo "E2E FAIL: 비-CSO apply 통과"; exit 1; else echo "  → 기대대로 차단"; fi
echo "ALL E2E PASS (라이브 무접촉)"; rm -rf "$T"
