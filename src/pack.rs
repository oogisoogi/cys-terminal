//! CYSJavis Pack: cys 터미널에 임베드된 멀티에이전트 운영체계 템플릿.

use std::path::PathBuf;

pub const ENV_PACK_DIR: &str = "CYS_PACK_DIR";
/// cys 전용 CLAUDE_CONFIG_DIR 오버라이드(주로 테스트 격리용). 미설정 시 pack_dir 형제(~/.cys/claude).
pub const ENV_CONFIG_DIR: &str = "CYS_CONFIG_DIR";

/// (상대경로, 내용) — init-jarvis 가 설치한다.
pub const PACK: &[(&str, &str)] = &[
    ("README.md", include_str!("../cysjavis-pack/README.md")),
    ("soul.md", include_str!("../cysjavis-pack/soul.md")),
    (
        "directives/MASTER_DIRECTIVE.md",
        include_str!("../cysjavis-pack/directives/MASTER_DIRECTIVE.md"),
    ),
    (
        "directives/WORKER_DIRECTIVE.md",
        include_str!("../cysjavis-pack/directives/WORKER_DIRECTIVE.md"),
    ),
    (
        "directives/CSO_DIRECTIVE.md",
        include_str!("../cysjavis-pack/directives/CSO_DIRECTIVE.md"),
    ),
    (
        "directives/REVIEWER_DIRECTIVE.md",
        include_str!("../cysjavis-pack/directives/REVIEWER_DIRECTIVE.md"),
    ),
    (
        "CLAUDE.md.template",
        include_str!("../cysjavis-pack/CLAUDE.md.template"),
    ),
    (
        "memory/MEMORY.md",
        include_str!("../cysjavis-pack/memory/MEMORY.md"),
    ),
    (
        "memory/feedback_autonomous-pilot-mandate.md",
        include_str!("../cysjavis-pack/memory/feedback_autonomous-pilot-mandate.md"),
    ),
    (
        "round/SESSION_STATE.md",
        include_str!("../cysjavis-pack/round/SESSION_STATE.md"),
    ),
    (
        "round/RECOVERY.md",
        include_str!("../cysjavis-pack/round/RECOVERY.md"),
    ),
    ("agents.json", include_str!("../cysjavis-pack/agents.json")),
    ("acl.json", include_str!("../cysjavis-pack/acl.json")),
    (
        "schedule.json",
        include_str!("../cysjavis-pack/schedule.json"),
    ),
    (
        "hooks/session-start.sh",
        include_str!("../cysjavis-pack/hooks/session-start.sh"),
    ),
    (
        "hooks/cys-statusline.sh",
        include_str!("../cysjavis-pack/hooks/cys-statusline.sh"),
    ),
    (
        "hooks/appbuild-gate.sh",
        include_str!("../cysjavis-pack/hooks/appbuild-gate.sh"),
    ),
    (
        "bin/javis_preflight.py",
        include_str!("../cysjavis-pack/bin/javis_preflight.py"),
    ),
    (
        "bin/javis_report.py",
        include_str!("../cysjavis-pack/bin/javis_report.py"),
    ),
    (
        "bin/javis_route.py",
        include_str!("../cysjavis-pack/bin/javis_route.py"),
    ),
    (
        "bin/route_triggers.json",
        include_str!("../cysjavis-pack/bin/route_triggers.json"),
    ),
    (
        "bin/javis_memory.py",
        include_str!("../cysjavis-pack/bin/javis_memory.py"),
    ),
    (
        "bin/javis_orchestra.py",
        include_str!("../cysjavis-pack/bin/javis_orchestra.py"),
    ),
    (
        "bin/javis_boot_node.py",
        include_str!("../cysjavis-pack/bin/javis_boot_node.py"),
    ),
    (
        "bin/javis_adr.py",
        include_str!("../cysjavis-pack/bin/javis_adr.py"),
    ),
    (
        "bin/javis_docsdiff.py",
        include_str!("../cysjavis-pack/bin/javis_docsdiff.py"),
    ),
    (
        "bin/javis_reflect.py",
        include_str!("../cysjavis-pack/bin/javis_reflect.py"),
    ),
    (
        "hooks/inject-context.sh",
        include_str!("../cysjavis-pack/hooks/inject-context.sh"),
    ),
    (
        "hooks/save-state.sh",
        include_str!("../cysjavis-pack/hooks/save-state.sh"),
    ),
    (
        "hooks/reflect-scan.sh",
        include_str!("../cysjavis-pack/hooks/reflect-scan.sh"),
    ),
    (
        "hooks/commit-memory-nudge.sh",
        include_str!("../cysjavis-pack/hooks/commit-memory-nudge.sh"),
    ),
];

// skills/ 전체는 build.rs가 디렉터리 스캔으로 자동 임베드한다 (PACK_SKILLS).
// 새 스킬은 cysjavis-pack/skills/<name>/ 에 추가하면 끝 — 수동 목록 갱신 불필요.
include!(concat!(env!("OUT_DIR"), "/pack_skills.rs"));

/// 설치 위치: $CYS_PACK_DIR (구 JAVIS_PACK_DIR·AITERM_JARVIS_DIR 폴백) 또는 ~/.cys/pack
pub fn pack_dir() -> PathBuf {
    if let Some(d) = crate::env_compat(ENV_PACK_DIR) {
        return PathBuf::from(d);
    }
    for legacy in ["JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"] {
        if let Ok(d) = std::env::var(legacy) {
            if !d.is_empty() {
                return PathBuf::from(d);
            }
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys/pack")
}

/// cys 전용 CLAUDE_CONFIG_DIR — 사용자 ~/.claude(외부 터미널 체계·구 마스터지침 오염 가능)와 **격리**한다.
/// cys가 띄우는 claude는 이 디렉터리만 읽으므로, 사용자 프로필이 오염돼 있어도 영향받지 않고
/// 사용자 프로필을 건드리지도(읽지도·지우지도) 않는다. macOS 인증은 계정 단위 Keychain이라
/// 격리해도 로그인이 유지된다(우리 DMG는 macOS 전용). pack_dir 형제(~/.cys/claude).
pub fn config_dir() -> PathBuf {
    if let Some(d) = crate::env_compat(ENV_CONFIG_DIR) {
        return PathBuf::from(d);
    }
    pack_dir()
        .parent()
        .map(|p| p.join("claude"))
        .unwrap_or_else(|| PathBuf::from(".cys/claude"))
}

/// 격리 config dir 셋업: cys 라우터(CLAUDE.md)와 SessionStart hook(settings.json)을 설치한다.
/// ★보존 모드 — 기존 파일은 덮지 않는다(사용자 커스터마이즈 불가침). best-effort(실패해도
/// pack 설치 자체는 유효). 사용자 ~/.claude 는 절대 건드리지 않는다(격리의 핵심).
fn setup_isolated_config_dir() {
    let cfg = config_dir();
    if std::fs::create_dir_all(&cfg).is_err() {
        return;
    }
    // 라우터: 임베드 CLAUDE.md.template → <cfg>/CLAUDE.md (없을 때만 — 역할선언→~/.cys/pack 라우팅)
    let claude_md = cfg.join("CLAUDE.md");
    if !claude_md.exists() {
        if let Some((_, tmpl)) = PACK.iter().find(|(rel, _)| *rel == "CLAUDE.md.template") {
            let _ = std::fs::write(&claude_md, tmpl);
        }
    }
    // hook: <cfg>/settings.json 에 SessionStart → session-start.sh (없을 때만)
    let settings = cfg.join("settings.json");
    if !settings.exists() {
        let hook = format!(
            "sh {}",
            pack_dir().join("hooks/session-start.sh").display()
        );
        let json = serde_json::json!({
            "hooks": { "SessionStart": [ { "hooks": [ { "type": "command", "command": hook } ] } ] }
        });
        if let Ok(s) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(&settings, s);
        }
    }
}

/// 설치 매니페스트: rel → 설치 당시 내용의 sha256. "지금 디스크에 있는 파일이 우리가
/// 설치한 그대로인가(=사용자 비수정)"를 판정하는 유일한 근거다.
const INSTALL_MANIFEST: &str = ".install-manifest.json";

fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// PACK 템플릿 설치 (CLI init-pack과 데몬 첫 기동 자동 설치의 공용 코어).
/// force=false: 사용자 수정 파일 불가침 + **비수정 파일은 임베드 신버전으로 자동 갱신**
/// (설치 매니페스트의 설치-당시 해시와 현재 파일 해시가 일치 = 비수정). 매니페스트가
/// 없는 구설치본 파일은 종전대로 보존한다(안전측). 반환: (written, kept).
pub fn install(force: bool) -> Result<(usize, usize), String> {
    let dir = pack_dir();
    let manifest_path = dir.join(INSTALL_MANIFEST);
    let mut manifest: std::collections::BTreeMap<String, String> = std::fs::read_to_string(
        &manifest_path,
    )
    .ok()
    .and_then(|s| serde_json::from_str(&s).ok())
    .unwrap_or_default();
    let mut written = 0;
    let mut kept = 0;
    for (rel, content) in PACK.iter().chain(PACK_SKILLS.iter()) {
        let path = dir.join(rel);
        if path.exists() && !force {
            match std::fs::read_to_string(&path) {
                Ok(existing) if existing == *content => {
                    // 디스크 = 임베드: 최신. 매니페스트 공백(구설치본)이면 채택 기록해
                    // 다음 버전부터 자동 갱신 대상이 되게 한다.
                    manifest
                        .entry(rel.to_string())
                        .or_insert_with(|| content_hash(content));
                    kept += 1;
                    continue;
                }
                Ok(existing)
                    if manifest.get(*rel).map(String::as_str)
                        == Some(content_hash(&existing).as_str()) =>
                {
                    // 설치-당시 해시 그대로(사용자 비수정) + 임베드가 더 새 버전 → 갱신.
                }
                _ => {
                    // 사용자 수정본·매니페스트 부재·읽기 실패 — 전부 보존(안전측).
                    kept += 1;
                    continue;
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, content)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        manifest.insert(rel.to_string(), content_hash(content));
        written += 1;
    }
    // 매니페스트 영속은 최선노력 — 실패해도 설치 자체는 유효하고, 다음 판정은
    // 보존(안전측)으로 떨어진다.
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = std::fs::write(&manifest_path, json);
    }
    // cys 전용 CLAUDE_CONFIG_DIR 격리 셋업(박사님 2026-06-15) — 사용자 ~/.claude 오염으로부터
    // cys 마스터를 분리한다. best-effort·보존 모드라 깨끗한 환경에서도 회귀 0.
    setup_isolated_config_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 실행권한은 임베드 내용의 shebang으로 결정한다 — 고정 목록은 스킬 스크립트
        // 추가 시 드리프트(fs::write가 exec 비트를 만들지 않아 직접 실행 스킬·hook
        // 등록이 신규 머신에서 깨짐)의 원천이었다. kept 파일에도 적용해 기존 설치본을
        // 복구한다.
        for (rel, content) in PACK.iter().chain(PACK_SKILLS.iter()) {
            if !content.starts_with("#!") {
                continue;
            }
            let p = dir.join(rel);
            if p.exists() {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
        }
    }
    Ok((written, kept))
}

pub fn role_directive_path(role: &str) -> Option<PathBuf> {
    // 접두 일치: reviewer-gemini / worker-2 같은 변형 역할도 표준 지침을 받는다
    let file = match role {
        "master" => "MASTER_DIRECTIVE.md",
        r if r.starts_with("worker") => "WORKER_DIRECTIVE.md",
        r if r.starts_with("cso") => "CSO_DIRECTIVE.md",
        r if r.starts_with("reviewer") => "REVIEWER_DIRECTIVE.md",
        _ => return None,
    };
    Some(pack_dir().join("directives").join(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 역할 → 디렉티브 파일명만 검증 (pack_dir 절대경로는 env 의존이라 비교하지 않음).
    fn dir_file(role: &str) -> Option<String> {
        role_directive_path(role)
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
    }

    #[test]
    fn role_directive_exact_master() {
        // master는 정확 일치만 — 'masterful' 같은 변형은 매핑 없음
        assert_eq!(dir_file("master").as_deref(), Some("MASTER_DIRECTIVE.md"));
        assert_eq!(dir_file("masterful"), None);
    }

    #[test]
    fn role_directive_prefix_variants_map_to_standard() {
        // 접두 일치: 변형 역할(worker-2·reviewer-gemini·cso-1)도 표준 지침을 받는다
        // — 디렉티브 주입(각성)이 변형 역할에서 누락되지 않게 하는 핵심 불변식.
        for (role, file) in [
            ("worker", "WORKER_DIRECTIVE.md"),
            ("worker-2", "WORKER_DIRECTIVE.md"),
            ("workerbee", "WORKER_DIRECTIVE.md"),
            ("cso", "CSO_DIRECTIVE.md"),
            ("cso-1", "CSO_DIRECTIVE.md"),
            ("reviewer", "REVIEWER_DIRECTIVE.md"),
            ("reviewer-gemini", "REVIEWER_DIRECTIVE.md"),
            ("reviewer-codex", "REVIEWER_DIRECTIVE.md"),
        ] {
            assert_eq!(dir_file(role).as_deref(), Some(file), "role={role}");
        }
    }

    #[test]
    fn role_directive_unknown_and_empty_are_none() {
        // 미지의 역할·빈 문자열은 None (잘못된 지침 주입 방지)
        assert_eq!(dir_file(""), None);
        assert_eq!(dir_file("gemini"), None);
        assert_eq!(dir_file("admin"), None);
        // 대소문자 민감 — 'Worker'는 'worker' 접두에 불일치
        assert_eq!(dir_file("Worker"), None);
    }

    #[test]
    fn role_directive_path_is_under_directives_dir() {
        // 경로 구조: <pack_dir>/directives/<FILE> — 부모 디렉터리가 'directives'
        let p = role_directive_path("master").unwrap();
        assert_eq!(
            p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().into_owned()),
            Some("directives".to_string())
        );
    }

    // pack_dir()이 읽는 env 키는 고정명이라 set/remove 윈도를 직렬화해야 cargo 병렬
    // 러너에서 서로 덮어쓰지 않는다 (R4 발견: 프로세스 전역 env 경합 차단 패턴).
    static PACK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ★불변식 박제: build.rs 자동 임베드가 오너 채택 스킬 14종(2026-06-12 k-skill 감사)
    /// + 기본 2종 + harness-creator + work management 2종(절대지침 5차 앵커 4규칙 b·c:
    /// hallucination-guard·grill-me) + 출처 고지를 전부 포함하고, 모든 SKILL.md가
    /// compose_directive의 색인 파서(첫 10줄 name:)에 잡히는 형식이어야 한다 —
    /// 어긋나면 노드 색인에서 누락된다.
    #[test]
    fn pack_skills_embed_adopted_set_and_indexable() {
        let names: Vec<&str> = PACK_SKILLS.iter().map(|(p, _)| *p).collect();
        for skill in [
            "korean-humanizer", "korean-spell-check", "korean-character-count",
            "naver-blog-research", "kosis-stats", "hwp", "rhwp-edit",
            "joseon-sillok-search", "geeknews-search", "k-dart", "korean-patent-search",
            "korean-stock-search", "daishin-report-search", "library-book-search",
            "skill-writing", "self-correction-loops", "harness-creator",
            "hallucination-guard", "grill-me",
            // superpowers A+B 9종 (2026-06-12 오너 채택 · 핀 6fd4507)
            "systematic-debugging", "test-driven-development",
            "subagent-driven-development", "dispatching-parallel-agents",
            "verification-before-completion", "brainstorming",
            "receiving-code-review", "writing-plans", "using-git-worktrees",
            // mattpocock A+B+집필3 9종 (2026-06-12 오너 채택 · 핀 694fa30)
            "git-guardrails-claude-code", "grill-with-docs", "prototype",
            "improve-codebase-architecture", "zoom-out", "handoff",
            "writing-fragments", "writing-beats", "writing-shape",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "임베드 누락: {skill}");
        }
        // cys-video-creator 영상 자동제작 스킬 32종(오너 제작 · preflight C26 VIDEO_SKILLS와
        // 동기) — pack 임베드로 기본 배포됨을 박제. 새 스킬 추가 시 양쪽을 함께 갱신한다.
        for skill in [
            "youtube-video-pipeline", "suite-runtime-keys", "cost-preview-confirm",
            "script-writer", "script-writer-research", "script-writer-structure",
            "script-writer-factcheck", "script-writer-voice-prep",
            "voice-clone-elevenlabs", "voice-clone-elevenlabs-chunk",
            "voice-clone-elevenlabs-synth-qc",
            "heygen-avatar-render", "heygen-avatar-render-api", "heygen-avatar-render-gate",
            "media-gen", "media-gen-image", "media-gen-edit", "media-gen-video",
            "media-gen-upscale", "media-gen-thumbnail",
            "video-stitch", "video-stitch-compositing", "video-stitch-broll",
            "video-stitch-captions",
            "audio-post", "audio-post-music", "audio-post-mix",
            "video-verify", "video-verify-visual", "video-verify-timing",
            "video-verify-audio-sync", "video-verify-final-gate",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "영상 스킬 임베드 누락: {skill}");
        }
        // appbuild 웹/앱 빌드 스킬 20종(오너 제작 · 워커 필수 · preflight C27 APPBUILD_SKILLS와
        // 동기) — 스펙 기반 기획→감독관 검증→자율빌드. pack 임베드 기본 배포 박제.
        for skill in [
            "appbuild", "appbuild-plan", "appbuild-plan-interview",
            "appbuild-plan-debate", "appbuild-plan-quick",
            "appbuild-screen-spec", "appbuild-screen-spec-flow", "appbuild-screen-spec-detail",
            "appbuild-tasks", "appbuild-tasks-slice", "appbuild-tasks-order",
            "appbuild-supervisor", "appbuild-supervisor-collect", "appbuild-supervisor-verify",
            "appbuild-supervisor-fix", "appbuild-supervisor-gate",
            "appbuild-orchestrate", "appbuild-orchestrate-delegate",
            "appbuild-orchestrate-verify", "appbuild-orchestrate-route",
        ] {
            let want = format!("skills/{skill}/SKILL.md");
            assert!(names.iter().any(|p| *p == want), "appbuild 스킬 임베드 누락: {skill}");
        }
        // appbuild 코드선행 금지 hook이 임베드돼야 C27이 설치·등록할 수 있다.
        let pack_names: Vec<&str> = PACK.iter().map(|(p, _)| *p).collect();
        assert!(pack_names.contains(&"hooks/appbuild-gate.sh"), "appbuild-gate hook 임베드 누락");
        assert!(names.contains(&"skills/THIRD_PARTY.md"), "외부 유래 출처 고지(MIT) 누락");
        for (path, content) in PACK_SKILLS.iter() {
            if path.ends_with("/SKILL.md") {
                // 실파서(compose_directive)는 name 값이 비어있으면 색인에서 제외한다 —
                // 존재만 보면 빈 name이 거짓 통과한다(적대 검증 R1).
                let indexable = content
                    .lines()
                    .take(10)
                    .any(|l| l.strip_prefix("name:").is_some_and(|v| !v.trim().is_empty()));
                assert!(indexable, "{path}: 첫 10줄에 유효한 name: 부재 — 스킬 색인에서 누락된다");
            }
        }
    }

    /// ★불변식 박제: 빈 디렉터리(신규 머신)에 install()만으로 코어 pack + 채택 스킬이
    /// 전부 설치된다 — "cysjavis 설치 = 기본 스킬 자동 설치" 계약의 기계 검증.
    #[test]
    fn install_writes_core_and_skills_to_fresh_dir() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-install-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let cfgdir = td.join("cysclaude"); // 격리 config dir(테스트 밀폐 — td와 함께 정리)
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, &cfgdir);
        let result = install(false);
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
        let (written, kept) = result.expect("install 실패");
        assert_eq!(kept, 0, "빈 디렉터리인데 kept>0");
        assert_eq!(written, PACK.len() + PACK_SKILLS.len(), "임베드 전수 설치 아님");
        // ★격리 config dir 셋업(박사님 2026-06-15): cys 라우터+hook이 전용 dir에 설치되고,
        // 사용자 ~/.claude 와 분리된다. 라우터는 ~/.cys/pack 디렉티브로 라우팅해야 한다.
        let router = std::fs::read_to_string(cfgdir.join("CLAUDE.md")).expect("격리 CLAUDE.md 미설치");
        assert!(router.contains("~/.cys/pack/directives"), "격리 라우터가 pack 디렉티브로 안 보냄");
        assert!(router.contains("외부 터미널 체계 아님") || router.contains("외부 터미널 체계"), "격리 라우터에 cys 환경선언 부재");
        let cfg_settings = std::fs::read_to_string(cfgdir.join("settings.json")).expect("격리 settings.json 미설치");
        assert!(cfg_settings.contains("SessionStart") && cfg_settings.contains("session-start.sh"),
                "격리 settings.json에 SessionStart hook 부재");
        for probe in [
            "skills/korean-humanizer/SKILL.md",
            "skills/kosis-stats/scripts/run_kosis_stats.py",
            "skills/THIRD_PARTY.md",
            "bin/javis_route.py",
            "directives/MASTER_DIRECTIVE.md",
        ] {
            assert!(td.join(probe).is_file(), "설치 누락: {probe}");
        }
        // ★불변식 박제: shebang 임베드 파일은 설치 직후 실행 가능해야 한다 —
        // 스킬이 scripts/x.sh 직접 실행·hook 등록을 전제하므로 exec 비트 소실은
        // 신규 머신에서 해당 기능 전체가 깨지는 결함이다(전수조사 발견 A).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut shebang_seen = 0;
            for (rel, content) in PACK.iter().chain(PACK_SKILLS.iter()) {
                if !content.starts_with("#!") {
                    continue;
                }
                shebang_seen += 1;
                let mode = std::fs::metadata(td.join(rel))
                    .unwrap_or_else(|_| panic!("설치 누락: {rel}"))
                    .permissions()
                    .mode();
                assert!(mode & 0o111 != 0, "{rel}: shebang인데 실행권한 없음 (mode={mode:o})");
            }
            // 회귀 가드: 스킬 스크립트가 규칙에 실제로 잡히는지 (bin 6종 + 스킬 7종 이상)
            assert!(shebang_seen >= 13, "shebang 파일이 {shebang_seen}개뿐 — 임베드 누락 의심");
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★불변식 박제: force=false 업그레이드 의미론 (전수조사 발견 B 보완).
    /// ① 사용자 비수정 파일(설치-당시 해시 일치) → 임베드 신버전으로 자동 갱신
    /// ② 사용자 수정 파일 → 불가침 보존
    /// ③ 매니페스트 부재(구설치본) + 내용 상이 → 보존(안전측)
    /// ④ 디스크=임베드인 구설치본 → 매니페스트 채택 기록(다음 버전부터 자동 갱신)
    #[test]
    fn install_upgrades_unmodified_keeps_user_modified() {
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let saved = std::env::var(ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pack-upgrade-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::env::set_var(ENV_PACK_DIR, &td);
        std::env::set_var(ENV_CONFIG_DIR, td.join("cysclaude")); // 격리(밀폐)

        let (rel_a, content_a) = PACK[0]; // 비수정·구버전 → 갱신 대상
        let (rel_b, content_b) = PACK[1]; // 사용자 수정 → 보존 대상
        let (rel_c, content_c) = PACK[2]; // 구설치본(매니페스트 없음)·내용 상이 → 보존
        std::fs::create_dir_all(&td).unwrap();
        for (rel, stale) in [(rel_a, "OLD-INSTALLED"), (rel_b, "USER-MODIFIED"), (rel_c, "LEGACY-STALE")] {
            let p = td.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, stale).unwrap();
        }
        // 매니페스트: a는 설치-당시 해시 = 현재 디스크 해시(비수정 증명),
        // b는 설치-당시 해시 ≠ 현재 디스크 해시(수정 증명), c는 항목 자체가 없음.
        let manifest = serde_json::json!({
            rel_a: content_hash("OLD-INSTALLED"),
            rel_b: content_hash("(다른 내용으로 설치됐었음)"),
        });
        std::fs::write(td.join(INSTALL_MANIFEST), manifest.to_string()).unwrap();

        install(false).expect("install 실패");

        assert_eq!(
            std::fs::read_to_string(td.join(rel_a)).unwrap(),
            content_a,
            "①비수정 구버전이 임베드 신버전으로 갱신되지 않음"
        );
        assert_eq!(
            std::fs::read_to_string(td.join(rel_b)).unwrap(),
            "USER-MODIFIED",
            "②사용자 수정본 불가침 위반"
        );
        assert_eq!(
            std::fs::read_to_string(td.join(rel_c)).unwrap(),
            "LEGACY-STALE",
            "③매니페스트 부재 파일은 보존(안전측)이어야 함"
        );
        // ④ 채택 기록: 디스크=임베드(이번에 신규 설치된 나머지 파일들)는 매니페스트에 등재
        let m: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(td.join(INSTALL_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(m.get(rel_a), Some(&content_hash(content_a)), "갱신 후 매니페스트 미반영");
        assert!(
            m.get(rel_b) != Some(&content_hash(content_b)),
            "수정본을 임베드 해시로 기록하면 다음 기동에서 덮어써진다"
        );
        // 재실행 멱등: 두 번째 install은 아무것도 다시 쓰지 않아야 한다 (b·c 보존 유지)
        let (w2, _) = install(false).unwrap();
        match saved {
            Some(v) => std::env::set_var(ENV_PACK_DIR, v),
            None => std::env::remove_var(ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(ENV_CONFIG_DIR, v),
            None => std::env::remove_var(ENV_CONFIG_DIR),
        }
        assert_eq!(w2, 0, "멱등 위반: 재실행이 {w2}개를 다시 씀");
        assert_eq!(std::fs::read_to_string(td.join(rel_b)).unwrap(), "USER-MODIFIED");
        let _ = std::fs::remove_dir_all(&td);
    }

    #[test]
    fn pack_dir_env_precedence_and_legacy_fallbacks() {
        // ★불변식 박제: pack_dir의 4단 폴백 우선순위.
        //   1) CYS_PACK_DIR (env_compat: CYS_ → JAVIS_ → AITERM_PACK_DIR 까지 본다)
        //   2) JAVIS_PACK_DIR (명시 레거시 루프)
        //   3) AITERM_JARVIS_DIR (명시 레거시 루프 — env_compat은 AITERM_PACK_DIR를
        //      만들지 AITERM_JARVIS_DIR가 아니므로 '오직 이 루프'로만 도달 가능)
        //   4) ~/.cys/pack (기본)
        // 마이그레이션 경로라 순서가 뒤집히면 구 설치본을 조용히 못 찾는다.
        let _g = PACK_ENV_LOCK.lock().unwrap();
        let keys = [
            "CYS_PACK_DIR",
            "JAVIS_PACK_DIR",
            "AITERM_PACK_DIR",
            "AITERM_JARVIS_DIR",
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }

        // 셋 다 없으면 기본 ~/.cys/pack (홈 끝 2요소가 .cys/pack)
        let def = pack_dir();
        assert!(
            def.ends_with(".cys/pack"),
            "기본 경로는 .cys/pack: {def:?}"
        );

        // AITERM_JARVIS_DIR만 → 3순위로 도달 (env_compat이 못 만드는 키, 루프 전용 경로)
        std::env::set_var("AITERM_JARVIS_DIR", "/legacy/aiterm");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/aiterm"));

        // JAVIS_PACK_DIR 추가 → AITERM_JARVIS_DIR보다 우선 (2순위)
        std::env::set_var("JAVIS_PACK_DIR", "/legacy/javis");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/javis"));

        // CYS_PACK_DIR 추가(env_compat primary) → 최우선 (1순위)
        std::env::set_var("CYS_PACK_DIR", "/modern/cys");
        assert_eq!(pack_dir(), PathBuf::from("/modern/cys"));

        // env_compat 폴백: CYS_PACK_DIR 비우면 JAVIS_PACK_DIR로(=2순위와 동일 키지만
        // env_compat 경로) — 빈 문자열은 미설정 취급이라 다음 후보로 넘어간다
        std::env::set_var("CYS_PACK_DIR", "");
        assert_eq!(pack_dir(), PathBuf::from("/legacy/javis"));

        // 복원
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}
