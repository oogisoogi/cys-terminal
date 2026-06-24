//! cysjavis-pack/skills/ 전체를 컴파일 타임에 자동 임베드하는 매니페스트 생성기.
//!
//! 스킬 파일을 pack.rs PACK 목록에 손으로 추가하는 방식은 임베드 드리프트(소스 수정 후
//! 목록 누락 → 신규 머신에 구버전/누락 배포)의 원천이라, 디렉터리 스캔으로 결정론 환원한다.
//! 새 스킬은 cysjavis-pack/skills/<name>/ 에 두기만 하면 빌드가 자동 임베드한다.

use std::env;
use std::fs;
use std::path::Path;

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // 숨김 파일(.DS_Store 등)·테스트 디렉터리는 배포 대상이 아니다
        if name.starts_with('.') || name == "tests" || name == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            walk(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=cysjavis-pack/skills");
    let base = Path::new("cysjavis-pack/skills");
    let mut files = Vec::new();
    walk(base, base, &mut files);
    files.sort();

    let mut code = String::from(
        "/// build.rs 자동 생성 — cysjavis-pack/skills/ 전체 임베드 (수동 목록 드리프트 차단).\n\
         pub const PACK_SKILLS: &[(&str, &str)] = &[\n",
    );
    for rel in &files {
        code.push_str(&format!(
            "    (\"skills/{rel}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/cysjavis-pack/skills/{rel}\"))),\n"
        ));
    }
    code.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR 없음");
    fs::write(Path::new(&out_dir).join("pack_skills.rs"), code).expect("pack_skills.rs 생성 실패");

    // T1-2: 단일진실 enum → OUT_DIR/cys_kinds.json (스키마·검증기 파리티의 기준).
    // 기존 디렉터리스캔 코드젠 철학과 동형(손목록 드리프트 차단). enum 정의는 src/edit_kinds.rs가
    // 진실이나 build.rs는 컴파일 전이라 그 타입을 못 본다 → 리터럴 목록을 여기 둔다(serde_json
    // build-dep 불요 — 평문 JSON 문자열). edit_kinds.rs enum과 어긋나면 tests/round-trip이 fail
    // (이중 잠금: 한쪽만 고치면 빨개짐). 추가 인프라 0 — std fs::write만.
    println!("cargo:rerun-if-changed=src/edit_kinds.rs");
    let kinds_json = "{\n  \"edit_kind\": [\"avatar\", \"broll\", \"graphic\", \"caption\", \"audio\", \"music\"],\n  \"mode\": [\"fullscreen\", \"left-card\", \"rounded-crop-pip\"],\n  \"transition\": [\"cut\", \"dissolve\", \"slide\"]\n}\n";
    fs::write(Path::new(&out_dir).join("cys_kinds.json"), kinds_json).expect("cys_kinds.json 생성 실패");
}
