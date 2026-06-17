//! cys — the CYSJavis terminal CLI client. 모든 pane 안의 AI가 이 CLI로 동등 노드가 된다.
//! 예: cys send --surface surface:31 "..." ; cys send-key --surface surface:31 Return

use clap::{Parser, Subcommand};
use cys::{key_to_bytes, parse_surface_ref, socket_path, surface_ref, ENV_SURFACE_ID};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};

#[derive(Parser)]
#[command(
    name = "cys",
    version,
    about = "cys — the CYSJavis terminal CLI (bidirectional socket, multi-agent OS)"
)]
struct Cli {
    /// Socket path override (default: AITERM_SOCKET or platform default)
    #[arg(long, global = true)]
    socket: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ping the daemon
    Ping,
    /// Identify daemon + caller (uses AITERM_SURFACE_ID env when inside a surface)
    Identify,
    /// Create a new surface (PTY session). Prints its surface ref.
    NewSurface {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        cmd: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Register this surface under a role (master/worker/cso/reviewer/...)
        #[arg(long)]
        role: Option<String>,
        #[arg(long, default_value_t = 35)]
        rows: u16,
        #[arg(long, default_value_t = 120)]
        cols: u16,
    },
    /// List surfaces
    List,
    /// Inject text into a surface's stdin (no trailing newline; follow with send-key Return)
    Send {
        #[arg(long)]
        surface: Option<String>,
        /// Address by role name instead of surface ref (e.g. --to master, --to 'reviewer-*')
        #[arg(long)]
        to: Option<String>,
        /// Followup mode: deliver when the target goes quiet (daemon queues + auto-injects with Return)
        #[arg(long)]
        queued: bool,
        /// 입력 버퍼 선정리(Ctrl-U) — launch-agent 등록 에이전트 pane 한정 (TUI별 의미 상이)
        #[arg(long)]
        clear_first: bool,
        /// Text to inject (multiple args are joined with spaces)
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Inject a named key (Return, Tab, C-c, Up, ...) into a surface's stdin
    SendKey {
        #[arg(long)]
        surface: Option<String>,
        /// Role name; supports glob (e.g. --to 'reviewer-*')
        #[arg(long)]
        to: Option<String>,
        /// Queue the key for quiet-time delivery (Return/Enter only) — typing-guard safe
        #[arg(long)]
        queued: bool,
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// T1-1 자기보고: 이 에이전트의 상태·컨텍스트%·작업을 데몬에 신고 (화면 파싱 대체)
    SetStatus {
        /// working | waiting | blocked | done
        #[arg(long, default_value = "working")]
        state: String,
        /// 컨텍스트 사용률 % (0-100)
        #[arg(long)]
        context: Option<u8>,
        /// 현재 작업 한 줄
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// T5 사용량 관측: 이 세션의 트랜스크립트 경로를 pane에 등록 (SessionStart hook 전용 plumbing)
    UsageRegister {
        /// 세션 트랜스크립트 절대경로 (.jsonl)
        #[arg(long)]
        transcript: String,
        #[arg(long)]
        surface: Option<String>,
    },
    /// T5 Phase 2-A: claude statusline stdin JSON을 읽어 usage.report로 push (cys-statusline.sh 전용 plumbing)
    UsageReportStdin {
        #[arg(long)]
        surface: Option<String>,
        /// push만 하고 사람용 statusline 한 줄을 출력하지 않는다 (기존 statusline 체인 보존 시).
        #[arg(long)]
        quiet: bool,
    },
    /// T1-2 통합 관제 보드: 전 노드 상태를 1콜로 (read-screen 폴링 대체)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// T4-15 kill-switch: 큐 배달·스케줄 발화 동결 (직접 send는 통과 — '신경 차단'이지 행동 정지가 아님)
    Pause {
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// kill-switch 해제 — 동결된 큐·스케줄 재개
    Resume,
    /// preflight 게이트: exit 0 = running, 4 = paused (자율주행 매 action 전 확인용)
    GateCheck,
    /// 미배달 큐 검사·철회 (kill-switch의 짝)
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// T2-4 컨텍스트 60% 사이클 집행기: 저장 지시→파일 검증→clear→지침 재주입→재개 포인터
    CycleAgent {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// 2-phase handshake 검증자 역할 — master cycle엔 필수 (self-clear 금지)
        #[arg(long)]
        verifier: Option<String>,
        /// 저장 검증 파일 (반복 가능; 기본: <cwd>/_round/SESSION_STATE.md 자동 탐지)
        #[arg(long = "save-file")]
        save_files: Vec<String>,
        /// clear 명령 override (기본: agents.json clear_cmd)
        #[arg(long)]
        clear_cmd: Option<String>,
        /// 재개 포인터 텍스트 override
        #[arg(long)]
        resume_text: Option<String>,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// 저장 파일 검증 없이 진행 (위험 — 명시 opt-out)
        #[arg(long)]
        force_no_verify: bool,
    },
    /// T2-5 죽은 에이전트를 같은 surface에서 재기동 + 지침 재주입 + 복원 포인터
    NodeRecover {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        role: Option<String>,
    },
    /// T2-6 조직 복원: 토폴로지 스냅샷의 죽은 역할들을 일괄 재기동·재주입 (작업 재개는 master 판단)
    Restore {
        #[arg(long)]
        cwd: Option<String>,
        /// master 역할도 재기동 대상에 포함 (기본 제외 — restore 실행자가 보통 master)
        #[arg(long)]
        include_master: bool,
        /// 에이전트 resume 플래그(agents.json resume_arg) 미사용
        #[arg(long)]
        no_resume: bool,
    },
    /// T2-7 디렉티브 재주입 (+--check: 각성 핑으로 드리프트 감지 후 필요 시에만 재주입)
    Reinject {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// 각성 확인 핑 먼저 — 응답 없을 때만 재주입
        #[arg(long)]
        check: bool,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// T3-14 완료 대기: scrollback 라인이 regex에 매칭될 때까지 블로킹 (plain-line 마커 규약)
    Watch {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// 대기할 regex 패턴
        #[arg(long)]
        until: String,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// 이 라인 커서 이후부터 감시 (기본: 호출 시점 이후)
        #[arg(long)]
        since: Option<u64>,
    },
    /// T4-18 트랜스크립트 해시체인: pin(평가자 외부 보관) / verify(사후 변조 대조)
    Attest {
        #[command(subcommand)]
        action: AttestAction,
    },
    /// 온보딩③: 데몬 상시 가동 등록 — 재부팅 후에도 24/365 (macOS launchd / Windows 작업 스케줄러)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Read a surface's screen (vt100-accurate) or last N scrollback lines
    ReadScreen {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        lines: Option<u64>,
        /// T3-14 델타 읽기: 이 라인 커서 이후의 새 라인만 (stderr에 next_cursor 출력)
        #[arg(long)]
        since: Option<u64>,
        #[arg(long, default_value_t = 2000)]
        max_lines: u64,
    },
    /// Resize a surface
    Resize {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        rows: u16,
        #[arg(long)]
        cols: u16,
    },
    /// Close a surface and force-kill its entire descendant process tree
    CloseSurface { surface: String },
    /// Subscribe to the daemon event stream (push; no polling)
    Events {
        #[arg(long)]
        after_seq: Option<u64>,
        #[arg(long = "name")]
        names: Vec<String>,
        #[arg(long = "category")]
        categories: Vec<String>,
        /// Auto-reconnect on connection loss
        #[arg(long)]
        reconnect: bool,
    },
    /// Mirror a surface's raw output to stdout (read-only tail)
    Attach { surface: String },
    /// Run a command in a new process group, registered in the daemon's process ledger.
    /// On exit the whole group is force-killed — 서버 생명주기 강제 종료.
    Run {
        #[arg(long)]
        surface: Option<String>,
        /// Command and arguments (after --)
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
    /// Show the process ledger (registered/scoped processes)
    Ps,
    /// Kill a ledger-registered process (group) by pid
    Kill { pid: u32 },
    /// Add a health rule (regex matched against every output line; fires health.alert)
    AddHealthRule {
        name: String,
        pattern: String,
        /// T4-17 조치 바인딩 (opt-in): pause-queue — 60초 창 threshold회 매칭 시 queued 배달 일시정지
        #[arg(long)]
        action: Option<String>,
        #[arg(long, default_value_t = 3)]
        threshold: u32,
        #[arg(long, default_value_t = 300)]
        pause_secs: u64,
    },
    /// List health rules
    HealthRules,
    /// Approval feed — 워커 승인 요청을 한 곳에 모아 처리
    Feed {
        #[command(subcommand)]
        action: FeedAction,
    },
    /// Install the CYSJavis Pack (multi-agent operating system templates) to ~/.cys/pack
    #[command(name = "init-pack", alias = "init-jarvis")]
    InitPack {
        /// Overwrite existing files (default: preserve user edits)
        #[arg(long)]
        force: bool,
        /// (기본 동작이 됨 — 하위호환용 no-op) SessionStart hook 등록
        #[arg(long, hide = true)]
        install_hook: bool,
        /// SessionStart hook 등록을 건너뛴다 (기본: ~/.claude*/settings.json 자동 탐색·등록)
        #[arg(long)]
        no_install_hook: bool,
        /// Claude settings.json 경로 명시 (생략 시 자동 탐색, 없으면 ~/.claude/settings.json 생성)
        #[arg(long)]
        claude_settings: Option<String>,
    },
    /// Search the persistent transcript memory of ALL agents' terminal activity (FTS)
    Recall {
        /// Search text (substring matching via trigram FTS)
        query: String,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// Only results from the last N days
        #[arg(long)]
        days: Option<f64>,
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Skill library — 경험을 스킬로 영속하고 재사용 (쓸수록 똑똑해지는 루프)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Heartbeat scheduler — 정해진 시각에 반복 업무를 자동 발화 (24/365 상주 데몬)
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// Register the current (or given) surface under a role — for sessions started without launch-agent
    ClaimRole {
        /// Role: master / worker / cso / reviewer
        role: String,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Launch an AI agent in a new role surface and auto-inject its directive
    LaunchAgent {
        /// Role: master / worker / cso / reviewer
        #[arg(long)]
        role: String,
        /// Agent: claude / gemini(=Antigravity CLI agy) / codex / grok (defined in agents.json)
        #[arg(long)]
        agent: String,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Boot the standard node set — 설치된 CLI만 자동 감지·기동·지침 주입 (worker claude + reviewer agy/codex/grok). CSO는 3단 정책상 띄우지 않는다
    Boot {
        /// Working directory for launched nodes
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Print (creating if absent) this surface's role-specific TODO file path — 복수 워커가 같은 파일을 공유하지 않도록 역할별 고유 경로를 결정론적으로 산출
    TodoPath,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// 로그인 시 자동 기동 + 죽으면 자동 재기동(launchd KeepAlive) 등록
    Install {
        /// 가동 중인 기존 데몬을 정지하고 launchd에 소유권 이관 (세션 소멸 — 주의)
        #[arg(long)]
        takeover: bool,
    },
    /// 등록 해제 (가동 중인 데몬도 정지)
    Uninstall,
    /// 등록·가동 상태 확인
    Status,
}

#[derive(Subcommand)]
enum QueueAction {
    /// List undelivered queued messages (all surfaces or one)
    List {
        #[arg(long)]
        surface: Option<String>,
    },
    /// Drop all undelivered queued messages for a surface
    Clear { surface: String },
}

#[derive(Subcommand)]
enum AttestAction {
    /// Print the current chain pin "count:hash" — 평가자가 SESSION_STATE 등 외부에 보관
    Pin {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// Verify a previously saved pin against the stored transcript (exit 0=match, 2=mismatch)
    Verify {
        /// "count:hash" (pin 출력 그대로)
        pin: String,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// Create a new skill from experience (SKILL.md, 4-칸 본문 템플릿)
    New {
        /// kebab-case skill name
        name: String,
        #[arg(long)]
        description: String,
    },
    /// List skill covers (name + description)
    List,
    /// Print a skill's full SKILL.md
    Show { name: String },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum ScheduleAction {
    /// Add a job to ~/.cys/pack/schedule.json (daemon hot-reloads)
    Add {
        #[arg(long)]
        id: String,
        /// "HH:MM" local time (반복 job — --in/--every와 택일)
        #[arg(long)]
        time: Option<String>,
        /// 주기 발화 간격(분) — 마지막 발화 후 N분마다 반복 (예: 5 = 5분 주기 보고 하트비트)
        #[arg(long)]
        every: Option<u64>,
        /// T3-10 원샷: 상대시간 후 1회 발화하고 job 자동 삭제 (예: 90s, 20m, 2h, 1h30m)
        #[arg(long = "in")]
        in_dur: Option<String>,
        /// fresh surface를 발화 후 N초 뒤 자동 close (원샷+fresh 누수 차단; --fresh 전용)
        #[arg(long)]
        close_after: Option<u64>,
        /// Comma-separated days (mon,tue,...). Omit for every day.
        #[arg(long)]
        days: Option<String>,
        /// Push this text to a role's stdin at the scheduled time
        #[arg(long)]
        text: Option<String>,
        /// Target role for --text (e.g. master)
        #[arg(long)]
        to: Option<String>,
        /// Run a shell command instead of pushing text
        #[arg(long)]
        command: Option<String>,
        /// If the target role is absent, launch it first (requires --agent)
        #[arg(long)]
        if_absent_launch: bool,
        /// Launch a NEW surface for every fire (isolation; requires --agent)
        #[arg(long)]
        fresh: bool,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// List jobs and last-fired times
    List,
    /// Remove a job by id
    Remove { id: String },
    /// Fire a job immediately (verification; does not affect the schedule)
    RunNow { id: String },
}

#[derive(Subcommand)]
enum FeedAction {
    /// Push an item. --wait blocks until a decision arrives (exit 0=allow, 2=deny, 3=timeout)
    Push {
        #[arg(long, default_value = "permission")]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
    },
    /// List feed items
    List {
        #[arg(long)]
        status: Option<String>,
    },
    /// Resolve a pending item (decision: allow / deny / free text)
    Reply {
        request_id: String,
        decision: String,
    },
}

fn main() {
    // 파이프(head 등)로 출력이 끊겨도 패닉하지 않도록 SIGPIPE 기본 동작 복원
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    if let Some(s) = &cli.socket {
        std::env::set_var(cys::ENV_SOCKET, s);
    }
    // 순수 프로브 명령은 자동 기동 금지 — "데몬이 떠 있는가"라는 질문의 답을 바꾸면 안 된다
    if matches!(
        cli.command,
        Command::Ping
            | Command::Daemon {
                action: DaemonAction::Status
            }
    ) {
        AUTOSTART.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let code = run(cli.command);
    std::process::exit(code);
}

fn target_surface(explicit: &Option<String>, to_role: &Option<String>) -> Result<u64, String> {
    if let Some(role) = to_role {
        let r = request("system.resolve_role", json!({"role": role}))?;
        return r["surface_id"]
            .as_u64()
            .ok_or_else(|| format!("role '{role}' resolved to invalid surface"));
    }
    if let Some(s) = explicit {
        return parse_surface_ref(s).ok_or_else(|| format!("invalid surface ref: {s}"));
    }
    if let Ok(env) = cys::env_compat(ENV_SURFACE_ID).ok_or(std::env::VarError::NotPresent) {
        if let Some(id) = parse_surface_ref(&env) {
            return Ok(id);
        }
    }
    Err("no --surface/--to given and CYS_SURFACE_ID is not set".into())
}

/// 명시된 --surface가 잘못된 형식이면 에러. 미지정(None)은 그대로 통과시켜
/// 호출처가 의미를 정한다 (env 폴백 또는 전체 검색).
fn parse_explicit_surface(surface: &Option<String>) -> Result<Option<u64>, String> {
    match surface {
        Some(s) => parse_surface_ref(s)
            .map(Some)
            .ok_or_else(|| format!("invalid surface ref: {s}")),
        None => Ok(None),
    }
}

/// T3-11 역할 글롭: '*'만 와일드카드 (reviewer-* 등)
fn cli_glob_match(pattern: &str, value: &str) -> bool {
    fn inner(p: &[char], v: &[char]) -> bool {
        match p.first() {
            None => v.is_empty(),
            Some('*') => {
                (0..=v.len()).any(|i| inner(&p[1..], &v[i..]))
            }
            Some(c) => v.first() == Some(c) && inner(&p[1..], &v[1..]),
        }
    }
    inner(
        &pattern.chars().collect::<Vec<_>>(),
        &value.chars().collect::<Vec<_>>(),
    )
}

/// T3-11: --to에 글롭이 오면 매칭되는 살아있는 역할 전부로 확장 (브로드캐스트)
fn resolve_targets(explicit: &Option<String>, to: &Option<String>) -> Result<Vec<u64>, String> {
    if let Some(role_pat) = to {
        if role_pat.contains('*') {
            let r = request("surface.list", json!({}))?;
            let ids: Vec<u64> = r["surfaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter(|s| !s["exited"].as_bool().unwrap_or(true))
                .filter(|s| {
                    s["role"]
                        .as_str()
                        .map(|x| cli_glob_match(role_pat, x))
                        .unwrap_or(false)
                })
                .filter_map(|s| s["surface_id"].as_u64())
                .collect();
            if ids.is_empty() {
                return Err(format!("no live roles match '{role_pat}'"));
            }
            return Ok(ids);
        }
    }
    target_surface(explicit, to).map(|sid| vec![sid])
}

/// surface.list에서 한 surface의 항목 조회 (agent 메타·role·cwd 확인용)
fn surface_entry(sid: u64) -> Result<Value, String> {
    let r = request("surface.list", json!({}))?;
    r["surfaces"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|s| s["surface_id"].as_u64() == Some(sid))
                .cloned()
        })
        .ok_or_else(|| format!("surface {sid} not found"))
}

/// 지침·과업 텍스트의 표준 주입: bracketed paste → 0.8s → Return
fn inject_text(sid: u64, text: &str) -> Result<(), String> {
    let wrapped = format!("\x1b[200~{text}\x1b[201~");
    request(
        "surface.send_text",
        json!({"surface_id": sid, "text": wrapped, "quiet": true}),
    )?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    request(
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return"}),
    )?;
    Ok(())
}

/// "90s" / "20m" / "2h" / "1h30m" → 초
fn parse_duration_secs(s: &str) -> Result<u64, String> {
    let mut total: u64 = 0;
    let mut num = String::new();
    let mut any = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("invalid duration '{s}'"))?;
            num.clear();
            any = true;
            // checked 산술: 거대한 입력(예: 9999999999999999d)이 debug에서 패닉,
            // release에서 silent wrap(엉뚱한 발화 시각)으로 새는 경로를 차단한다.
            let mult = match ch {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                _ => return Err(format!("invalid duration unit '{ch}' in '{s}'")),
            };
            let add = n
                .checked_mul(mult)
                .ok_or_else(|| format!("duration overflow in '{s}'"))?;
            total = total
                .checked_add(add)
                .ok_or_else(|| format!("duration overflow in '{s}'"))?;
        }
    }
    if !num.is_empty() || !any {
        return Err(format!(
            "invalid duration '{s}' (expected e.g. 90s, 20m, 2h, 1h30m)"
        ));
    }
    Ok(total)
}

fn sha256_file(path: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    std::fs::read(path).ok().map(|b| {
        let mut h = Sha256::new();
        h.update(&b);
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    })
}

// ---------- transport ----------

#[cfg(unix)]
fn connect_raw() -> Result<std::os::unix::net::UnixStream, String> {
    let path = socket_path();
    std::os::unix::net::UnixStream::connect(&path)
        .map_err(|e| format!("cannot connect to cysd at {}: {e}", path.display()))
}

#[cfg(windows)]
fn connect_raw() -> Result<std::fs::File, String> {
    let path = socket_path();
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot connect to cysd pipe {}: {e}", path.display()))
}

/// 온보딩④: 자동 기동 허용 — ping(순수 프로브)·daemon status는 main()에서 끈다.
static AUTOSTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// 한 CLI 실행에서 spawn 시도는 1회만
static AUTOSTART_TRIED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn sibling_daemon_path() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|d| d.join(name))
        .filter(|p| p.exists())
}

/// 데몬을 분리 세션으로 기동 — CLI가 Ctrl-C로 죽어도 데몬은 살아남는다.
fn spawn_detached_daemon(path: &std::path::Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    cmd.spawn().map(|_| ())
}

/// 온보딩④: 연결 실패 시 형제 cysd를 자동 기동 후 재시도 — 신규 머신 zero-setup.
/// 옵트아웃: CYS_NO_AUTOSTART=1. (데몬 중복 기동은 cysd 자체의 flock이 차단)
fn connect() -> Result<ConnStream, String> {
    match connect_raw() {
        Ok(s) => Ok(s),
        Err(first) => {
            let opted_out = cys::env_compat("CYS_NO_AUTOSTART")
                .map(|v| v == "1")
                .unwrap_or(false);
            if opted_out
                || !AUTOSTART.load(std::sync::atomic::Ordering::Relaxed)
                || AUTOSTART_TRIED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(first);
            }
            let Some(daemon) = sibling_daemon_path() else {
                return Err(format!("{first} (no sibling cysd to autostart)"));
            };
            eprintln!("[cys] cysd not running — autostarting {}", daemon.display());
            if spawn_detached_daemon(&daemon).is_err() {
                return Err(first);
            }
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if let Ok(s) = connect_raw() {
                    return Ok(s);
                }
            }
            Err(format!("{first} (autostarted cysd did not come up within 4s)"))
        }
    }
}

#[cfg(unix)]
type ConnStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type ConnStream = std::fs::File;

fn request(method: &str, params: Value) -> Result<Value, String> {
    let mut stream = connect()?;
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    reader
        .read_line(&mut resp_line)
        .map_err(|e| e.to_string())?;
    let resp: Value = serde_json::from_str(resp_line.trim()).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(format!(
            "{}: {}",
            resp["error"]["code"].as_str().unwrap_or("error"),
            resp["error"]["message"].as_str().unwrap_or("unknown error")
        ))
    }
}

// ---------- commands ----------

fn run(command: Command) -> i32 {
    let result = match command {
        Command::Ping => request("system.ping", json!({})).map(|r| println!("{}", r.as_str().unwrap_or("pong"))),

        Command::Identify => {
            let caller = cys::env_compat(ENV_SURFACE_ID).ok_or(std::env::VarError::NotPresent)
                .ok()
                .and_then(|s| parse_surface_ref(&s))
                .map(|id| json!({"surface_id": id, "surface_ref": surface_ref(id)}))
                .unwrap_or(Value::Null);
            request("system.identify", json!({"caller": caller}))
                .map(|r| println!("{}", serde_json::to_string_pretty(&r).unwrap()))
        }

        Command::NewSurface { cwd, cmd, title, role, rows, cols } => {
            request(
                "surface.create",
                json!({"cwd": cwd, "cmd": cmd, "title": title, "role": role, "rows": rows, "cols": cols}),
            )
            .map(|r| println!("{}", r["surface_ref"].as_str().unwrap_or("?")))
        }

        Command::List => request("surface.list", json!({})).map(|r| {
            for s in r["surfaces"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}\trole={}\tpid={}\texited={}\t{}\t{}",
                    s["surface_ref"].as_str().unwrap_or("?"),
                    s["role"].as_str().unwrap_or("-"),
                    s["pid"],
                    s["exited"],
                    s["title"].as_str().unwrap_or(""),
                    s["cwd"].as_str().unwrap_or(""),
                );
            }
        }),

        Command::Send { surface, to, queued, clear_first, text } => {
            resolve_targets(&surface, &to).and_then(|sids| {
                let from = cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s));
                let multi = sids.len() > 1;
                for sid in sids {
                    // T3-13 권위 전달: clear_first는 데몬이 원자적으로(Ctrl-U 선정리 → paste → CR)
                    // 집행한다. 클라측 C-u·150ms sleep·게이트는 제거 — 비원자 split·race를 없앤다.
                    // agent 등록 pane 게이트는 데몬 send_text가 집행(clear_first_unsupported).
                    let r = request(
                        "surface.send_text",
                        json!({"surface_id": sid, "text": text.join(" "), "from": from, "queued": queued, "clear_first": clear_first}),
                    )?;
                    let tag = if multi { format!(" → surface:{sid}") } else { String::new() };
                    if queued {
                        println!("QUEUED (depth {}){tag}", r["depth"]);
                    } else {
                        println!("OK{tag}");
                    }
                }
                Ok(())
            })
        }

        Command::SendKey { surface, to, queued, keys } => {
            resolve_targets(&surface, &to).and_then(|sids| {
                for key in &keys {
                    if key_to_bytes(key).is_none() {
                        return Err(format!("unknown key: {key}"));
                    }
                    if queued && !matches!(key.as_str(), "Return" | "Enter") {
                        return Err(format!(
                            "--queued supports only Return/Enter (got: {key}) — \
                             다른 키는 quiet-time 텍스트 큐에 실을 수 없다"
                        ));
                    }
                }
                let multi = sids.len() > 1;
                for sid in sids {
                    for key in &keys {
                        let r = request(
                            "surface.send_key",
                            json!({"surface_id": sid, "key": key, "queued": queued}),
                        )?;
                        if queued {
                            match r["depth"].as_u64() {
                                Some(d) => println!("QUEUED (depth {d})"),
                                // 구 데몬은 queued 파라미터를 모르고 즉시 주입한다 —
                                // "QUEUED"로 오표시하지 않는다(skew의 결정론 신호).
                                None => eprintln!(
                                    "[send-key] 경고: 데몬이 --queued를 지원하지 않아 \
                                     직접 주입됨(구버전 cysd — 재기동으로 갱신하라)"
                                ),
                            }
                        }
                    }
                    if multi {
                        println!("OK → surface:{sid}");
                    }
                }
                if !multi && !queued {
                    println!("OK");
                }
                Ok(())
            })
        }

        Command::SetStatus { state, context, task, surface } => {
            target_surface(&surface, &None).and_then(|sid| {
                request(
                    "status.set",
                    json!({"surface_id": sid, "state": state, "context": context, "task": task}),
                )
                .map(|_| println!("OK"))
            })
        }

        Command::UsageRegister { transcript, surface } => {
            target_surface(&surface, &None).and_then(|sid| {
                request(
                    "usage.register",
                    json!({"surface_id": sid, "transcript": transcript}),
                )
                .map(|_| println!("OK"))
            })
        }

        Command::UsageReportStdin { surface, quiet } => {
            return run_usage_report_stdin(&surface, quiet)
        }

        Command::Status { json: as_json } => return run_status(as_json),

        Command::Pause { reason } => request("system.pause", json!({"reason": reason}))
            .map(|_| println!("PAUSED — 큐 배달·스케줄 발화 동결 (이미 실행 중인 에이전트 행동은 계속된다; cys resume로 해제)")),

        Command::Resume => request("system.resume", json!({}))
            .map(|_| println!("RESUMED — 동결된 큐·스케줄 재개")),

        Command::GateCheck => {
            return match request("system.gate_check", json!({})) {
                Ok(r) => {
                    if r["paused"].as_bool() == Some(true) {
                        println!("PAUSED (reason: {})", r["reason"].as_str().unwrap_or(""));
                        4
                    } else {
                        println!("running");
                        0
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Queue { action } => {
            return match action {
                QueueAction::List { surface } => parse_explicit_surface(&surface)
                    .and_then(|sid| request("queue.list", json!({"surface_id": sid})))
                    .map(|r| {
                        let entries = r["entries"].as_array().cloned().unwrap_or_default();
                        if entries.is_empty() {
                            println!("(queue empty)");
                        }
                        for e in entries {
                            println!(
                                "{}\t[{}]\t{}B\t{}",
                                e["surface_ref"].as_str().unwrap_or("?"),
                                e["index"],
                                e["bytes"],
                                e["preview"].as_str().unwrap_or(""),
                            );
                        }
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                QueueAction::Clear { surface } => parse_surface_ref(&surface)
                    .ok_or_else(|| format!("invalid surface ref: {surface}"))
                    .and_then(|sid| request("queue.clear", json!({"surface_id": sid})))
                    .map(|r| {
                        println!("cleared {} queued message(s)", r["cleared"]);
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
            };
        }

        Command::CycleAgent {
            role,
            surface,
            verifier,
            save_files,
            clear_cmd,
            resume_text,
            timeout,
            force_no_verify,
        } => {
            return run_cycle_agent(
                role, surface, verifier, save_files, clear_cmd, resume_text, timeout,
                force_no_verify,
            )
        }

        Command::NodeRecover { surface, role } => return run_node_recover(surface, role),

        Command::Restore { cwd, include_master, no_resume } => {
            return run_restore(cwd, include_master, no_resume)
        }

        Command::Reinject { role, surface, check, timeout } => {
            return run_reinject(role, surface, check, timeout)
        }

        Command::Watch { surface, to, until, timeout, since } => {
            return match target_surface(&surface, &to).and_then(|sid| {
                request(
                    "surface.wait_for",
                    json!({"surface_id": sid, "pattern": until,
                           "timeout_secs": timeout, "since_line": since}),
                )
            }) {
                Ok(r) => {
                    if r["matched"].as_bool() == Some(true) {
                        println!("{}", r["line"].as_str().unwrap_or(""));
                        eprintln!("[matched line {} — next_cursor={}]", r["line_no"], r["next_cursor"]);
                        0
                    } else {
                        eprintln!("[no match: {} — next_cursor={}]",
                            r["reason"].as_str().unwrap_or("?"), r["next_cursor"]);
                        3
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Daemon { action } => return run_daemon_cmd(action),

        Command::Attest { action } => {
            return match action {
                AttestAction::Pin { surface, to } => target_surface(&surface, &to)
                    .and_then(|sid| request("attest.pin", json!({"surface_id": sid})))
                    .map(|r| {
                        println!("{}:{}", r["count"], r["hash"].as_str().unwrap_or("?"));
                        eprintln!("[이 pin을 SESSION_STATE 등 외부에 보관하라 — 검증 지평: anchor {} 이후]",
                            r["verification_horizon"]["anchor_count"]);
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                AttestAction::Verify { pin, surface, to } => {
                    let Some((count_s, hash)) = pin.split_once(':') else {
                        eprintln!("error: pin must be \"count:hash\"");
                        return 1;
                    };
                    let Ok(count) = count_s.parse::<u64>() else {
                        eprintln!("error: bad count in pin");
                        return 1;
                    };
                    match target_surface(&surface, &to).and_then(|sid| {
                        request(
                            "attest.verify",
                            json!({"surface_id": sid, "hash": hash, "count": count}),
                        )
                    }) {
                        Ok(r) => {
                            if r["match"].as_bool() == Some(true) {
                                println!("MATCH — transcript intact ({} lines)", count);
                                0
                            } else {
                                println!(
                                    "MISMATCH — {}",
                                    r["reason"].as_str().unwrap_or("hash differs (변조 또는 유실)")
                                );
                                2
                            }
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            1
                        }
                    }
                }
            };
        }

        Command::ReadScreen { surface, to, lines, since, max_lines } => {
            target_surface(&surface, &to).and_then(|sid| {
                if let Some(s) = since {
                    return request(
                        "surface.read_text",
                        json!({"surface_id": sid, "since_line": s, "max_lines": max_lines}),
                    )
                    .map(|r| {
                        let text = r["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            println!("{text}");
                        }
                        eprintln!(
                            "[next_cursor={} latest={} truncated={}]",
                            r["next_cursor"], r["latest_cursor"], r["truncated"]
                        );
                    });
                }
                request("surface.read_text", json!({"surface_id": sid, "lines": lines}))
                    .map(|r| println!("{}", r["text"].as_str().unwrap_or("")))
            })
        }

        Command::InitPack { force, install_hook: _, no_install_hook, claude_settings } => {
            return run_init_pack(force, no_install_hook, claude_settings);
        }

        Command::ClaimRole { role, surface } => target_surface(&surface, &None).and_then(|sid| {
            request("system.claim_role", json!({"role": role, "surface_id": sid}))
                .map(|r| println!("registered: {} → surface:{}", r["role"].as_str().unwrap_or("?"), sid))
        }),

        Command::LaunchAgent { role, agent, cwd } => return run_launch_agent(&role, &agent, cwd),
        Command::Boot { cwd } => return run_boot(cwd),
        Command::TodoPath => return run_todo_path(),

        Command::Resize { surface, rows, cols } => target_surface(&surface, &None).and_then(|sid| {
            request("surface.resize", json!({"surface_id": sid, "rows": rows, "cols": cols}))
                .map(|_| println!("OK"))
        }),

        Command::CloseSurface { surface } => parse_surface_ref(&surface)
            .ok_or_else(|| format!("invalid surface ref: {surface}"))
            .and_then(|sid| {
                request("surface.close", json!({"surface_id": sid})).map(|r| {
                    println!("closed {} (descendants killed)", surface);
                    let _ = r;
                })
            }),

        Command::Events { after_seq, names, categories, reconnect } => {
            stream_events(after_seq, names, categories, reconnect)
        }

        Command::Attach { surface } => parse_surface_ref(&surface)
            .ok_or_else(|| format!("invalid surface ref: {surface}"))
            .and_then(attach),

        Command::Run { surface, command } => {
            // 자식의 종료 코드를 그대로 프로세스 exit code로 전달
            return match run_scoped(surface, command) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Ps => request("ledger.list", json!({})).map(|r| {
            let entries = r["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                println!("(ledger empty)");
            }
            for e in entries {
                println!(
                    "pid={}\tpgid={}\tscoped={}\tsurface={}\t{}",
                    e["pid"],
                    e["pgid"],
                    e["scoped"],
                    e["surface_id"],
                    e["cmd"].as_str().unwrap_or("")
                );
            }
        }),

        Command::Kill { pid } => {
            request("ledger.kill", json!({"pid": pid})).map(|_| println!("killed {pid}"))
        }

        Command::AddHealthRule { name, pattern, action, threshold, pause_secs } => {
            request(
                "health.add_rule",
                json!({"name": name, "pattern": pattern, "action": action,
                       "threshold": threshold, "pause_secs": pause_secs}),
            )
            .map(|_| println!("OK"))
        }

        Command::HealthRules => request("health.list_rules", json!({})).map(|r| {
            for rule in r["rules"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}\t{}",
                    rule["name"].as_str().unwrap_or("?"),
                    rule["pattern"].as_str().unwrap_or("")
                );
            }
        }),

        Command::Feed { action } => return run_feed(action),

        Command::Schedule { action } => return run_schedule(action),

        Command::Recall { query, role, surface, days, limit } => {
            parse_explicit_surface(&surface)
                .and_then(|sid| request(
                    "recall.search",
                    json!({"query": query, "role": role, "surface_id": sid, "days": days, "limit": limit}),
                ))
            .map(|r| {
                let matches = r["matches"].as_array().cloned().unwrap_or_default();
                if matches.is_empty() {
                    println!("(no matches — indexed lines: {})", r["indexed_lines"]);
                }
                for m in matches {
                    let ts = m["ts"].as_f64().unwrap_or(0.0) as i64;
                    let when = chrono_fmt(ts);
                    println!(
                        "[{}] surface:{}({}) {} | {}",
                        when,
                        m["surface_id"],
                        m["role"].as_str().unwrap_or("-"),
                        m["title"].as_str().unwrap_or(""),
                        m["line"].as_str().unwrap_or(""),
                    );
                }
            })
        }

        Command::Skill { action } => return run_skill(action),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_feed(action: FeedAction) -> i32 {
    let result: Result<i32, String> = match action {
        FeedAction::Push { kind, title, body, surface, request_id, wait, timeout_secs } => {
            parse_explicit_surface(&surface)
                .and_then(|explicit| {
                    let sid = explicit
                        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));
                    request(
                        "feed.push",
                        json!({"kind": kind, "title": title, "body": body, "surface_id": sid,
                               "request_id": request_id, "wait": wait, "timeout_secs": timeout_secs}),
                    )
                })
            .map(|r| {
                if wait {
                    let status = r["status"].as_str().unwrap_or("");
                    let decision = r["decision"].as_str().unwrap_or("");
                    println!("{}", if status == "timeout" { "timeout" } else { decision });
                    match (status, decision) {
                        ("timeout", _) => 3,
                        (_, "allow") | (_, "yes") | (_, "approve") => 0,
                        _ => 2,
                    }
                } else {
                    println!("{}", r["request_id"].as_str().unwrap_or("?"));
                    0
                }
            })
        }
        FeedAction::List { status } => request("feed.list", json!({"status": status})).map(|r| {
            let items = r["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("(feed empty)");
            }
            for i in items {
                println!(
                    "{}\t[{}]\t{}\t{}\tdecision={}",
                    i["request_id"].as_str().unwrap_or("?"),
                    i["status"].as_str().unwrap_or("?"),
                    i["kind"].as_str().unwrap_or("?"),
                    i["title"].as_str().unwrap_or(""),
                    i["decision"].as_str().unwrap_or("-"),
                );
            }
            0
        }),
        FeedAction::Reply { request_id, decision } => {
            request("feed.reply", json!({"request_id": request_id, "decision": decision}))
                .map(|_| {
                    println!("OK");
                    0
                })
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Subscribe to the push event stream and print NDJSON lines.
fn stream_events(
    after_seq: Option<u64>,
    names: Vec<String>,
    categories: Vec<String>,
    reconnect: bool,
) -> Result<(), String> {
    let mut last_seq = after_seq;
    loop {
        let attempt = (|| -> Result<(), String> {
            let mut stream = connect()?;
            let req = json!({
                "id": 1, "method": "events.stream",
                "params": {"after_seq": last_seq, "names": names, "categories": categories},
            });
            let mut line = serde_json::to_string(&req).unwrap();
            line.push('\n');
            stream
                .write_all(line.as_bytes())
                .map_err(|e| e.to_string())?;
            let reader = BufReader::new(stream);
            for read in reader.lines() {
                let l = read.map_err(|e| e.to_string())?;
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v["type"] == "event" {
                        if let Some(seq) = v["seq"].as_u64() {
                            last_seq = Some(seq);
                        }
                    } else if v["type"] == "ack" && last_seq.is_none() {
                        // 첫 이벤트 수신 전 끊겨도 재접속이 구체적 커서로 replay 경로를 타게 시드
                        last_seq = v["latest_seq"].as_u64();
                    }
                }
                println!("{l}");
            }
            Err("event stream closed".into())
        })();
        match attempt {
            Err(e) if reconnect => {
                eprintln!("[events] {e}; reconnecting in 1s...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            other => return other,
        }
    }
}

/// Mirror raw PTY output to stdout.
fn attach(sid: u64) -> Result<(), String> {
    let mut stream = connect()?;
    let req = json!({"id": 1, "method": "surface.attach", "params": {"surface_id": sid}});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    // First line is the JSON ack; everything after is raw bytes.
    let mut reader = BufReader::new(stream);
    let mut ack = String::new();
    reader.read_line(&mut ack).map_err(|e| e.to_string())?;
    let ack_v: Value = serde_json::from_str(ack.trim()).unwrap_or(Value::Null);
    if ack_v["ok"].as_bool() != Some(true) {
        return Err(format!("attach failed: {}", ack.trim()));
    }
    eprintln!("[attached surface:{sid} — Ctrl-C to detach]");
    let mut stdout = std::io::stdout();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                stdout.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                stdout.flush().ok();
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn chrono_fmt(epoch: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(epoch.max(0) as u64);
    // 로컬 포맷은 데몬이 epoch만 주므로 간단 표기 (ISO-ish, 로컬 오프셋 미적용 시 UTC)
    match std::process::Command::new("date")
        .args(["-r", &epoch.to_string(), "+%m-%d %H:%M"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => format!("{:?}", dt),
    }
}

/// 스킬 라이브러리: jarvis/skills/<name>/SKILL.md (frontmatter 표지 + 4칸 본문).
fn run_skill(action: SkillAction) -> i32 {
    let skills_dir = cys::pack::pack_dir().join("skills");
    let result: Result<(), String> = match action {
        SkillAction::New { name, description } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            let dir = skills_dir.join(&name);
            let path = dir.join("SKILL.md");
            if path.exists() {
                return Err(format!("skill '{name}' already exists: {}", path.display()));
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let body = format!(
                "---\nname: {name}\ndescription: {description}\n---\n\n\
                 # {name}\n\n\
                 ## 언제 쓰나\n- \n\n\
                 ## 순서\n1. \n\n\
                 ## 주의할 점 (함정 — 겪을 때마다 한 줄씩 누적하라)\n- \n\n\
                 ## 확인하는 방법 (검증 — 겪을 때마다 한 줄씩 누적하라)\n- \n"
            );
            std::fs::write(&path, body).map_err(|e| e.to_string())?;
            println!("created {}", path.display());
            println!("(4칸을 채우고, master 승인이 필요하면 feed push로 보고하라)");
            Ok(())
        })(),
        SkillAction::List => (|| {
            let entries = std::fs::read_dir(&skills_dir).map_err(|_| {
                format!(
                    "no skills dir: {} (run cys init-pack)",
                    skills_dir.display()
                )
            })?;
            let mut count = 0;
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                let Ok(content) = std::fs::read_to_string(&skill_md) else {
                    continue;
                };
                let (mut name, mut desc) = (String::new(), String::new());
                for line in content.lines().take(10) {
                    if let Some(v) = line.strip_prefix("name:") {
                        name = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("description:") {
                        desc = v.trim().to_string();
                    }
                }
                if !name.is_empty() {
                    println!("{name}\t{desc}");
                    count += 1;
                }
            }
            if count == 0 {
                println!("(no skills yet — `cys skill new <name> --description \"...\"`)");
            }
            Ok(())
        })(),
        SkillAction::Show { name } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            let path = skills_dir.join(&name).join("SKILL.md");
            let content = std::fs::read_to_string(&path)
                .map_err(|_| format!("no skill '{name}' ({})", path.display()))?;
            println!("{content}");
            Ok(())
        })(),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Heartbeat 스케줄 관리: schedule.json은 CLI가 직접 편집(데몬 핫 리로드), 조회·즉발은 RPC.
fn run_schedule(action: ScheduleAction) -> i32 {
    let path = cys::pack::pack_dir().join("schedule.json");
    let result: Result<(), String> = match action {
        ScheduleAction::Add {
            id,
            time,
            every,
            in_dur,
            close_after,
            days,
            text,
            to,
            command,
            if_absent_launch,
            fresh,
            agent,
            cwd,
        } => {
            (|| {
                if text.is_some() == command.is_some() {
                    return Err("exactly one of --text(+--to) or --command is required".into());
                }
                if text.is_some() && to.is_none() {
                    return Err("--text requires --to <role>".into());
                }
                if (if_absent_launch || fresh) && agent.is_none() {
                    return Err("--if-absent-launch/--fresh requires --agent".into());
                }
                if command.is_some()
                    && (to.is_some()
                        || if_absent_launch
                        || fresh
                        || agent.is_some()
                        || cwd.is_some())
                {
                    return Err("--command cannot be combined with --to/--if-absent-launch/--fresh/--agent/--cwd (these apply only to --text push jobs)".into());
                }
                // --time(반복)·--in(원샷)·--every(주기) 정확히 하나
                let mode_count = time.is_some() as u8 + in_dur.is_some() as u8 + every.is_some() as u8;
                if mode_count != 1 {
                    return Err("exactly one of --time (반복) / --in (원샷) / --every (주기) is required".into());
                }
                if let Some(m) = every {
                    if m == 0 {
                        return Err("--every must be >= 1 (minutes)".into());
                    }
                }
                if every.is_some() && days.is_some() {
                    return Err("--every(주기)는 --days와 함께 쓸 수 없다".into());
                }
                if in_dur.is_some() && days.is_some() {
                    return Err("--in(원샷)은 --days와 함께 쓸 수 없다".into());
                }
                if close_after.is_some() && !fresh {
                    return Err("--close-after는 --fresh 전용 (fresh surface TTL)".into());
                }
                // 데몬과 동일 규칙으로 add 시점에 검증 — 잘못된 값이 무음 무발화로 이어지는 것을 차단
                if let Some(t) = &time {
                    chrono::NaiveTime::parse_from_str(t, "%H:%M")
                        .map_err(|_| format!("invalid --time '{t}' (expected HH:MM)"))?;
                }
                let at: Option<i64> = match &in_dur {
                    Some(d) => {
                        let secs = parse_duration_secs(d)?;
                        Some(chrono::Local::now().timestamp() + secs as i64)
                    }
                    None => None,
                };
                let mut root: Value = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_else(|| json!({"jobs": []}));
                let jobs = root
                    .as_object_mut()
                    .ok_or("schedule.json root is not an object")?
                    .entry("jobs")
                    .or_insert(json!([]));
                let arr = jobs.as_array_mut().ok_or("'jobs' is not an array")?;
                if arr.iter().any(|j| j["id"].as_str() == Some(id.as_str())) {
                    return Err(format!("job '{id}' already exists (remove first)"));
                }
                let days_vec: Vec<String> = days
                    .map(|d| d.split(',').map(|s| s.trim().to_lowercase()).collect())
                    .unwrap_or_default();
                const DOW: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
                if let Some(bad) = days_vec.iter().find(|d| !DOW.contains(&d.as_str())) {
                    return Err(format!(
                        "invalid --days token '{bad}' (allowed: mon,tue,wed,thu,fri,sat,sun)"
                    ));
                }
                let mut job = match (&time, at, every) {
                    (Some(t), _, _) => json!({"id": id, "time": t, "days": days_vec}),
                    (None, Some(at), _) => json!({"id": id, "at": at, "once": true}),
                    (None, None, Some(m)) => json!({"id": id, "every_minutes": m}),
                    _ => unreachable!(),
                };
                if let Some(ttl) = close_after {
                    job["close_after_secs"] = json!(ttl);
                }
                if let Some(t) = text {
                    job["action"] = json!("push");
                    job["to"] = json!(to.unwrap());
                    job["text"] = json!(t);
                    if if_absent_launch || fresh {
                        if if_absent_launch {
                            job["if_absent"] = json!("launch");
                        }
                        if fresh {
                            job["fresh"] = json!(true);
                        }
                        job["launch"] =
                            json!({"role": job["to"], "agent": agent.unwrap(), "cwd": cwd});
                    }
                } else {
                    job["action"] = json!("command");
                    job["command"] = json!(command.unwrap());
                }
                arr.push(job);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap())
                    .map_err(|e| e.to_string())?;
                println!(
                    "job added to {} (daemon hot-reloads within 30s)",
                    path.display()
                );
                Ok(())
            })()
        }
        ScheduleAction::List => request("schedule.status", json!({})).map(|r| {
            let jobs = r["jobs"].as_array().cloned().unwrap_or_default();
            if jobs.is_empty() {
                println!(
                    "(no jobs — {} )",
                    r["schedule_path"].as_str().unwrap_or("?")
                );
            }
            for j in jobs {
                let lf = r["last_fired"][j["id"].as_str().unwrap_or("")].as_i64();
                let when = j["time"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| j["at"].as_i64().map(|a| format!("once@{}", chrono_fmt(a))))
                    .unwrap_or_else(|| "?".into());
                println!(
                    "{}\t{} {}\t{}\t{}\tlast_fired={}",
                    j["id"].as_str().unwrap_or("?"),
                    when,
                    j["days"]
                        .as_array()
                        .map(|d| if d.is_empty() {
                            "daily".to_string()
                        } else {
                            d.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default(),
                    j["action"].as_str().unwrap_or("?"),
                    j["text"].as_str().or(j["command"].as_str()).unwrap_or(""),
                    lf.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
                );
            }
        }),
        ScheduleAction::Remove { id } => (|| {
            let mut root: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let arr = root["jobs"]
                .as_array_mut()
                .ok_or("'jobs' is not an array")?;
            let before = arr.len();
            arr.retain(|j| j["id"].as_str() != Some(id.as_str()));
            if arr.len() == before {
                return Err(format!("no job '{id}'"));
            }
            std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap())
                .map_err(|e| e.to_string())?;
            println!("removed {id}");
            Ok(())
        })(),
        ScheduleAction::RunNow { id } => {
            request("schedule.run_now", json!({"job_id": id})).map(|_| println!("fired {id}"))
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// CYSJavis Pack 설치: 임베드된 템플릿을 ~/.cys/pack 에 기록 (기존 파일 보존이 기본).
/// SessionStart hook 등록도 기본 동작이다(절대지침 — 터미널 작동 순간부터 활성화).
/// --no-install-hook으로만 끌 수 있다.
fn run_init_pack(force: bool, no_install_hook: bool, claude_settings: Option<String>) -> i32 {
    let dir = cys::pack::pack_dir();
    let (written, kept) = match cys::pack::install(force) {
        Ok(wk) => wk,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!(
        "CYSJavis Pack installed at {} ({} written, {} preserved{})",
        dir.display(),
        written,
        kept,
        if force { ", forced" } else { "" }
    );
    println!("다음: cys launch-agent --role master --agent claude  (역할 지침 자동 주입)");

    if no_install_hook {
        return 0;
    }
    let targets = match claude_settings {
        Some(p) => vec![p],
        None => {
            let found = discover_claude_settings();
            if found.is_empty() {
                // 신규 머신: Claude Code 기본 경로에 생성해 "켜는 순간부터 활성화"를 보장.
                vec![dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".claude/settings.json")
                    .to_string_lossy()
                    .into_owned()]
            } else {
                found
            }
        }
    };
    let mut rc = 0;
    for settings_path in targets {
        if let Some(parent) = std::path::Path::new(&settings_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match install_claude_hook(&settings_path, &dir) {
            Ok(msg) => println!("hook[{settings_path}]: {msg}"),
            Err(e) => {
                eprintln!("error: hook install failed for {settings_path}: {e}");
                rc = 1;
            }
        }
    }
    rc
}

/// Claude Code 설정 파일 자동 탐색: $HOME 직하의 `.claude*` 디렉터리에 있는 settings.json 전부.
/// (멀티 프로필 환경 — 예: .claude / .claude-* — 을 한 번에 커버.)
/// 결정론: 존재하는 파일만, 사전순 정렬로 반환한다.
fn discover_claude_settings() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(&home) else {
        return vec![];
    };
    let mut found: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n == ".claude" || n.starts_with(".claude-"))
                .unwrap_or(false)
        })
        .map(|e| e.path().join("settings.json"))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// SessionStart hook으로 등록할 명령 문자열을 OS별로 조립한다 (순수 함수 — 회귀 핀).
///
/// Unix: 기존과 동일하게 `sh <path>/session-start.sh`.
/// Windows: 바닐라 Windows 셸(cmd/PowerShell)은 `.sh`를 인터프리터 없이 실행하지 못하고
///   "open with" 대화상자를 띄운다(anthropics/claude-code #21847·#24097). Claude Code가
///   Windows에서 찾는 인터프리터는 Git Bash의 `bash`이므로, 바 셸이 해석할 수 있도록
///   `bash` 명령으로 명시 호출한다(맨 이름 `sh`는 Git Bash가 `bash.exe`만 보장하므로 회피).
///   `/clear` 후 SessionStart 자동 재주입(autopilot 축2)이 Windows에서도 발동하게 하는 핵심.
fn hook_command(pack_dir: &std::path::Path) -> String {
    let script = pack_dir.join("hooks/session-start.sh");
    if cfg!(windows) {
        format!("bash {}", script.display())
    } else {
        format!("sh {}", script.display())
    }
}

/// Claude Code settings.json에 SessionStart hook을 등록한다 (백업 생성, 중복 등록 방지).
fn install_claude_hook(settings_path: &str, pack_dir: &std::path::Path) -> Result<String, String> {
    // symlink 거부 — 링크 너머 실파일을 덮어쓰는 TOCTOU 부류 차단(preflight와 동일 규약).
    if std::fs::symlink_metadata(settings_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!("{settings_path} is a symlink — refusing to write"));
    }
    let hook_cmd = hook_command(pack_dir);
    let mut root: Value = match std::fs::read_to_string(settings_path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("settings parse error: {e}"))?,
        // 파일 없음일 때만 빈 설정으로 시작 — 권한 등 다른 읽기 에러를 무시하면
        // 기존 settings.json이 hooks만 남은 JSON으로 대체될 수 있다
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("settings read error: {e}")),
    };
    // backup
    if std::path::Path::new(settings_path).exists() {
        let backup = format!("{settings_path}.bak-cys");
        std::fs::copy(settings_path, &backup).map_err(|e| e.to_string())?;
    }
    let hooks = root
        .as_object_mut()
        .ok_or("settings root is not an object")?
        .entry("hooks")
        .or_insert(json!({}));
    let session_start = hooks
        .as_object_mut()
        .ok_or("hooks is not an object")?
        .entry("SessionStart")
        .or_insert(json!([]));
    let arr = session_start
        .as_array_mut()
        .ok_or("SessionStart is not an array")?;
    let already = arr.iter().any(|m| {
        m["hooks"]
            .as_array()
            .map(|hs| {
                hs.iter()
                    .any(|h| h["command"].as_str() == Some(hook_cmd.as_str()))
            })
            .unwrap_or(false)
    });
    if already {
        return Ok("hook already installed (skipped)".into());
    }
    arr.push(json!({"hooks": [{"type": "command", "command": hook_cmd}]}));
    std::fs::write(settings_path, serde_json::to_string_pretty(&root).unwrap())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "SessionStart hook registered in {settings_path} (backup: .bak-cys)"
    ))
}

/// 표준 노드 일괄 부트: 설치된 CLI만 자동 감지해 워커+리뷰어를 기동·지침 주입한다.
/// 마스터 부트 시퀀스 ④의 결정론적 구현 — 모델 재량("필요할 때 띄우자")에 맡기지 않는다.
/// '~/'-시작 경로를 홈으로 확장 (그 외는 그대로) — boot의 경로형 cmd 설치 판정용.
fn expand_tilde(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(p)
}

/// 절대지침 앵커4-1: 프로젝트 시작 시 CSO·worker·agy·codex 4개 노드를 의무 기동한다
/// (LLM orchestrating 상주 편성). grok은 설치돼 있으면 추가 리뷰어로 띄운다(미설치 skip).
fn run_boot(cwd: Option<String>) -> i32 {
    // (역할, 에이전트) 표준 편성 — 4차 의무 4종 + 선택 grok. 순서: CSO 먼저(감독).
    const PLAN: &[(&str, &str)] = &[
        ("cso", "claude"),
        ("worker", "claude"),
        ("reviewer-gemini", "gemini"),
        ("reviewer-codex", "codex"),
        ("reviewer-grok", "grok"),
    ];
    let agents: Value = std::fs::read_to_string(cys::pack::pack_dir().join("agents.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    // 이미 가동 중인 역할은 중복 기동하지 않는다
    let live_roles: std::collections::HashSet<String> = request("surface.list", json!({}))
        .ok()
        .and_then(|r| r["surfaces"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|s| !s["exited"].as_bool().unwrap_or(true))
        .filter_map(|s| s["role"].as_str().map(|x| x.to_string()))
        .collect();
    let mut launched = 0;
    let mut failed = 0;
    println!("cys boot — LLM orchestrating 편성 점검 (CSO·worker·agy·codex 4종 의무 + grok 선택)");
    for (role, agent) in PLAN {
        let bin = agents
            .get(*agent)
            .and_then(|a| a["cmd"].as_str())
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or(agent)
            .to_string();
        // 경로형 cmd('~/'·'/' 포함 — 예: agy 절대경로)는 which/where가 틸드를 확장하지
        // 않아 '미설치'로 오판한다 → 파일 존재로 판정 (실행은 셸 -lc 경유라 틸드 확장됨)
        let found = if bin.starts_with('~') || bin.contains('/') {
            expand_tilde(&bin).exists()
        } else {
            #[cfg(windows)]
            let ok = std::process::Command::new("where")
                .arg(&bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            #[cfg(not(windows))]
            let ok = std::process::Command::new("which")
                .arg(&bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            ok
        };
        if !found {
            println!("· {agent}: CLI '{bin}' 미설치 — 건너뜀");
            continue;
        }
        if live_roles.contains(*role) {
            println!("· {agent}: 역할 '{role}' 이미 가동 중 — 건너뜀");
            continue;
        }
        println!("· {agent}: 기동 시작 (role={role})…");
        if run_launch_agent(role, agent, cwd.clone()) == 0 {
            launched += 1;
        } else {
            failed += 1;
            println!("· {agent}: 기동 실패 — 나머지 노드는 계속 진행");
        }
    }
    println!(
        "boot 완료: 신규 기동 {launched} · 실패 {failed} · 현황은 `cys list`로 확인 (role 열)"
    );
    if failed > 0 {
        1
    } else {
        0
    }
}

/// agents.json에서 어댑터 스펙 로드
fn load_agent_spec(agent: &str) -> Result<Value, String> {
    let agents_path = cys::pack::pack_dir().join("agents.json");
    let agents: Value = std::fs::read_to_string(&agents_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| {
            format!(
                "agents.json not found at {} — run `cys init-pack` first",
                agents_path.display()
            )
        })?;
    agents
        .get(agent)
        .cloned()
        .ok_or_else(|| format!("unknown agent '{agent}' (agents.json에 정의 필요)"))
}

/// 역할 디렉티브 + soul.md + 장기메모리 색인 + 스킬 색인 조립 (launch/reinject/cycle 공용)
fn compose_directive(role: &str) -> Result<String, String> {
    let dir = cys::pack::pack_dir();
    // 표준 4역할 외(임시 역할 — fresh heartbeat의 scan-bot 등)는 WORKER 지침으로 폴백
    let directive_path = cys::pack::role_directive_path(role).unwrap_or_else(|| {
        eprintln!("[directive] non-standard role '{role}' — falling back to WORKER_DIRECTIVE");
        dir.join("directives/WORKER_DIRECTIVE.md")
    });
    let mut directive = std::fs::read_to_string(&directive_path)
        .map_err(|e| format!("cannot read {}: {e}", directive_path.display()))?;
    let soul_path = dir.join("soul.md");
    if let Ok(soul) = std::fs::read_to_string(&soul_path) {
        directive.push_str("\n\n■ soul.md (운영 헌장)\n");
        directive.push_str(&soul);
    }
    // 장기메모리 색인 동봉 — 본문(1파일 1사실)은 필요 시 해당 파일을 읽어 점진 로드.
    // 헤더에 절대경로를 박는다: 노드가 본문 읽기·증류 쓰기 위치를 추론하지 않게(결정론).
    let memory_path = dir.join("memory/MEMORY.md");
    if let Ok(memory) = std::fs::read_to_string(&memory_path) {
        directive.push_str(&format!(
            "\n\n■ 장기메모리 색인 ({} — 노드 공유 의미 기억 · 증류는 bin/javis_memory.py add)\n",
            memory_path.display()
        ));
        directive.push_str(&memory);
    }
    // 스킬 색인(표지) 동봉 — 본문은 필요 시 `cys skill show <name>`으로 점진 로드
    let mut index = String::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("skills")) {
        for entry in entries.flatten() {
            if let Ok(content) = std::fs::read_to_string(entry.path().join("SKILL.md")) {
                let (mut name, mut desc) = (String::new(), String::new());
                for line in content.lines().take(10) {
                    if let Some(v) = line.strip_prefix("name:") {
                        name = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("description:") {
                        desc = v.trim().to_string();
                    }
                }
                if !name.is_empty() {
                    index.push_str(&format!("- {name}: {desc}\n"));
                }
            }
        }
    }
    if !index.is_empty() {
        directive.push_str("\n\n■ 보유 스킬 색인 (본문: `cys skill show <name>`)\n");
        directive.push_str(&index);
    }
    Ok(directive)
}

/// 화면 마지막 비공백 줄이 셸 프롬프트로 끝나는지 판정 — marker 없는 에이전트의 시간 폴백
/// 직전 검사다. TUI가 떴다면 끝줄이 셸 프롬프트일 수 없다; 셸 프롬프트가 남아 있으면
/// 에이전트가 조용히 즉시 종료(에러 문구 없이)한 것이므로 주입하면 zsh로 들어간다.
fn screen_tail_is_shell_prompt(text: &str) -> bool {
    let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false; // 화면 비어 있음 — 판단 보류(시간 폴백 유지)
    };
    let t = last.trim_end();
    // zsh "...%" / bash·sh "...$" / root "#" / powerlevel10k·starship "❯" —
    // 끝문자 기준(프롬프트 커스텀의 공통 꼬리). 오탐 효과는 '대기 후 명시 Err'(안전측).
    t.ends_with('%') || t.ends_with('$') || t.ends_with('#') || t.ends_with('❯')
}

/// 기동 화면(공백 제거 평탄화 문자열)에 "명령을 못 찾았다"는 셸 오류가 떴는지 판정.
/// readiness 폴링이 죽은 셸에 지침을 주입하는 것을 막는 사망 감지의 핵심 술어다.
/// Unix sh/zsh/bash뿐 아니라 Windows PowerShell·cmd.exe의 표현까지 덮어
/// 크로스플랫폼으로 동일하게 기동 실패를 잡는다(`hook_command` OS 대칭화와 짝).
fn screen_shows_launch_failure(flat: &str) -> bool {
    // Unix: sh/zsh/bash "command not found" / 직접 실행 시 "No such file or directory" / "not found in PATH"
    flat.contains("commandnotfound")
        || flat.contains("notfoundinPATH")
        || flat.contains("Nosuchfileordirectory")
        // Windows PowerShell: "... is not recognized as the name of a cmdlet, function, ..."
        || flat.contains("isnotrecognizedasthenameofacmdlet")
        // Windows cmd.exe: "... is not recognized as an internal or external command, ..."
        || flat.contains("isnotrecognizedasaninternalorexternalcommand")
}

/// 살아있는 surface 위에서: 에이전트 기동 → 준비 폴링 → 지침 주입 → 메타 등록.
/// launch-agent(새 surface)와 node-recover(기존 surface 재기동)가 공유한다.
fn boot_agent_on_surface(
    sid: u64,
    role: &str,
    agent: &str,
    spec: &Value,
    resume: bool,
) -> Result<(), String> {
    let mut cmd = spec["cmd"].as_str().ok_or("agent cmd missing")?.to_string();
    if resume {
        if let Some(arg) = spec["resume_arg"].as_str() {
            // T2-6 resume 어댑터: 대화 기억 복원 플래그 (예: claude --continue)
            cmd.push(' ');
            cmd.push_str(arg);
        }
    }
    let delay = spec["inject_delay_secs"].as_u64().unwrap_or(12);
    let directive = compose_directive(role)?;

    // 1) 에이전트 기동
    request(
        "surface.send_text",
        json!({"surface_id": sid, "text": cmd, "quiet": true}),
    )?;
    request(
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return"}),
    )?;
    eprintln!(
        "[launch-agent] {agent} starting… (polling readiness, max {}s)",
        delay.max(30) * 2
    );

    // 2) 준비 감지 폴링: 폴더 신뢰 프롬프트는 자동 확인, ready_marker가 보이면 주입 단계로
    let ready_marker = spec["ready_marker"].as_str().map(|s| s.to_string());
    let max_wait_secs = delay.max(30) * 2;
    let mut waited = 0u64;
    let mut ready = false;
    let mut last_screen = String::new();
    while waited < max_wait_secs {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        waited += 2; // ~2.5s per tick (보수적 집계)
        let screen = request("surface.read_text", json!({"surface_id": sid}))?;
        let text = screen["text"].as_str().unwrap_or("");
        last_screen = text.to_string();
        let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if screen_shows_launch_failure(&flat) {
            return Err(format!(
                "agent '{agent}' failed to start (command error on screen) — check cmd in agents.json"
            ));
        }
        if flat.contains("trustthisfolder") || flat.contains("Doyoutrust") {
            eprintln!("[launch-agent] folder-trust prompt detected → confirming");
            request(
                "surface.send_key",
                json!({"surface_id": sid, "key": "Return"}),
            )?;
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        }
        match &ready_marker {
            Some(m) if text.contains(m.as_str()) => {
                ready = true;
                break;
            }
            // marker 미정의 에이전트(codex 등)의 시간 폴백 — 단 화면 끝이 여전히
            // 셸 프롬프트(%·$)면 에이전트(TUI)가 안 뜬 것이다(조용한 즉시 종료 등):
            // 시간만 믿고 주입하면 디렉티브가 zsh로 들어간다(맹주입 잔존 경로 차단).
            None if waited >= delay => {
                if screen_tail_is_shell_prompt(text) {
                    continue; // 아직 셸 — max_wait까지 더 기다린다(못 뜨면 아래 Err)
                }
                ready = true;
                break;
            }
            _ => {}
        }
    }
    if !ready {
        // 준비 미확인 주입 금지: 에이전트가 안 떠 있으면 디렉티브가 맨 셸(zsh)로 들어가
        // 첫 단어가 명령으로 실행된다("zsh: command not found: 는" — 2026-06-12 실측).
        // 주의: launch 경로 호출자가 실패 surface를 정리(close)하므로, 진단 증거(화면 꼬리)는
        // 여기서 에러 본문에 동봉한다 — "read-screen으로 확인하라"는 안내는 close 후 거짓이 된다.
        let tail: Vec<&str> = last_screen
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let tail = tail
            .iter()
            .rev()
            .take(5)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "agent '{agent}' readiness not confirmed in {max_wait_secs}s — directive injection \
             aborted (셸 오주입 차단). 실패 surface는 정리된다. 마지막 화면 꼬리:\n{tail}\n\
             → agents.json의 cmd를 점검하고 `cys launch-agent --role <role> --agent {agent}`로 \
             재시도하라"
        ));
    }
    // marker 감지 직후 TUI 입력 활성화까지 약간의 여유
    std::thread::sleep(std::time::Duration::from_secs(2));

    // 3) 지침 주입 — bracketed paste로 감싸 단일 입력으로 전달
    inject_text(sid, &directive)?;

    // 4) 주입 확인: 화면에 지침 머리말이 나타났는지 검사 (실패 시 경고)
    std::thread::sleep(std::time::Duration::from_secs(3));
    let screen = request(
        "surface.read_text",
        json!({"surface_id": sid, "lines": 200}),
    )?;
    let flat: String = screen["text"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if flat.contains("ABSOLUTEDIRECTIVE") || flat.contains("절대지침") {
        eprintln!(
            "[launch-agent] directive injected & visible on screen ({} bytes)",
            directive.len()
        );
    } else {
        eprintln!("[launch-agent] warning: directive not visible on screen — verify with `cys read-screen --surface {}`", surface_ref(sid));
    }

    // 5) T2-5 에이전트 메타 등록 — 사망 감지·status 보드·approval 스캔의 기반
    let bin = cmd.split_whitespace().next().unwrap_or(agent).to_string();
    request(
        "surface.set_meta",
        json!({"surface_id": sid, "agent": agent, "agent_bin": bin}),
    )?;
    Ok(())
}

/// 에이전트 기동 + 역할 지침 자동 주입 (어댑터: agents.json).
/// 워커 todo 경로 결정론 산출: 자기 surface의 (데몬 권위) 역할 → `<pack>/round/<ROLE>_TODO.md`.
/// 역할은 데몬 roles 맵(dedup된 worker-N 포함)에서 읽으므로 LLM 치환·env 스냅샷에 의존하지 않는다.
/// 복수 워커는 각자 distinct 역할 → distinct 파일 → 충돌 0. 파일이 없으면 골격을 만들어 둔다.
fn run_todo_path() -> i32 {
    let Some(sref) = cys::env_compat(ENV_SURFACE_ID) else {
        eprintln!("CYS_SURFACE_ID 없음 — 데몬이 띄운 pane 안에서만 동작한다");
        return 1;
    };
    let Some(my_sid) = parse_surface_ref(&sref) else {
        eprintln!("CYS_SURFACE_ID 파싱 실패: {sref}");
        return 1;
    };
    let role = match request("surface.list", json!({})) {
        Ok(r) => r["surfaces"].as_array().and_then(|arr| {
            arr.iter()
                .find(|s| s["surface_id"].as_u64() == Some(my_sid))
                .and_then(|s| s["role"].as_str().map(|x| x.to_string()))
        }),
        Err(e) => {
            eprintln!("surface.list 실패: {e}");
            return 1;
        }
    };
    let Some(role) = role else {
        eprintln!("이 surface에 역할 미등록 — todo-path는 역할 노드(claim-role/launch-agent) 전용");
        return 1;
    };
    let pack = cys::env_compat("CYS_PACK_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cys/pack")))
        .unwrap_or_else(|| std::path::PathBuf::from(".cys/pack"));
    let round = pack.join("round");
    if let Err(e) = std::fs::create_dir_all(&round) {
        eprintln!("round 디렉터리 생성 실패: {e}");
        return 1;
    }
    let fname = format!("{}_TODO.md", role.to_uppercase().replace('-', "_"));
    let path = round.join(&fname);
    if !path.exists() {
        let _ = std::fs::write(&path, format!("# {role} TODO — 영속 todo (절대지침 7)\n\n"));
    }
    println!("{}", path.display());
    0
}

fn run_launch_agent(role: &str, agent: &str, cwd: Option<String>) -> i32 {
    run_launch_agent_opts(role, agent, cwd, false)
}

/// 절대지침(앵커1-b): 탭(타이틀) = 워크플로우 폴더명 — "{role}-{agent} · {폴더}".
/// 폴더를 알 수 없으면(루트 등) 역할-에이전트만. 순수 함수 — 회귀 핀.
/// `/`·`\`를 모두 구분자로 취급해 플랫폼과 무관하게 마지막 컴포넌트를 폴더명으로 쓴다
/// (std::path::Path는 Unix에서 `\`를 구분자로 보지 않아 Windows 경로가 통째로 잡힌다 —
/// 데몬·클라이언트가 OS를 교차할 수 있으므로 수동 분할이 결정론적·이식 가능하다).
fn workflow_title(role: &str, agent: &str, cwd: &Option<String>) -> String {
    cwd.as_deref()
        .map(|s| s.trim_end_matches(['/', '\\']))
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
        // Windows 드라이브 루트(`C:\` → 트림 후 `C:`)는 폴더명이 아니다 — 폴백.
        .filter(|f| !(f.len() == 2 && f.ends_with(':') && f.as_bytes()[0].is_ascii_alphabetic()))
        .map(|folder| format!("{role}-{agent} · {folder}"))
        .unwrap_or_else(|| format!("{role}-{agent}"))
}

fn run_launch_agent_opts(role: &str, agent: &str, cwd: Option<String>, resume: bool) -> i32 {
    // 절대지침(앵커1-b): 워커는 워크플로우 폴더에서 산다 — cwd 미지정이면 호출 폴더가
    // 워크플로우 폴더다 (데몬 기본값 home에 맡기지 않는다. 명시 --cwd는 그대로 우선).
    // 빈 문자열은 None으로 정규화 — 구버전 topology의 "cwd": "" 가 PTY 생성을 깨거나
    // 잘못된 타이틀을 만드는 것을 차단(restore 경로 방어).
    let cwd = cwd.filter(|s| !s.is_empty()).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    // 기동 실패 시 정리용 — 만들어 둔 surface가 role을 점유한 채 남으면 재기동이 차단된다
    let mut created: Option<u64> = None;
    let result = (|| -> Result<(), String> {
        let spec = load_agent_spec(agent)?;
        let r = request(
            "surface.create",
            json!({"cwd": cwd, "title": workflow_title(role, agent, &cwd), "role": role,
                   "rows": 40, "cols": 140}),
        )?;
        let sid = r["surface_id"].as_u64().ok_or("create returned no id")?;
        created = Some(sid);
        eprintln!("[launch-agent] {} created (role={role})", surface_ref(sid));
        boot_agent_on_surface(sid, role, agent, &spec, resume)?;
        println!("{}", surface_ref(sid));
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            if let Some(sid) = created {
                let _ = request("surface.close", json!({"surface_id": sid}));
                eprintln!(
                    "[launch-agent] failed surface {} closed (role 점유 해제)",
                    surface_ref(sid)
                );
            }
            1
        }
    }
}

// ---------- 온보딩③: 상시 가동 등록 (launchd / Task Scheduler) ----------

#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "com.cysjavis.cysd";

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"))
}

fn run_daemon_cmd(action: DaemonAction) -> i32 {
    let result: Result<(), String> = (|| {
        #[cfg(target_os = "macos")]
        {
            match action {
                DaemonAction::Install { takeover } => {
                    let daemon = sibling_daemon_path()
                        .ok_or("cysd binary not found next to cys (같은 폴더에 동봉 필요)")?;
                    let running = connect_raw().is_ok();
                    if running && !takeover {
                        return Err(
                            "데몬이 이미 가동 중 — 등록만 하면 launchd 인스턴스가 flock에 막혀 재시도 루프가 된다.\n\
                             기존 데몬을 정지하고 소유권을 이관하려면: cys daemon install --takeover\n\
                             (주의: 가동 중인 세션이 소멸한다 — `cys list`로 먼저 확인)"
                                .into(),
                        );
                    }
                    let log = cys::socket_path()
                        .parent()
                        .map(|d| d.join("cysd.log"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/cysd.log"));
                    let plist = format!(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
                        daemon.display(),
                        log = log.display(),
                    );
                    let path = launchd_plist_path();
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&path, plist).map_err(|e| e.to_string())?;
                    if running && takeover {
                        // 소유권 이관: 기존 데몬 정상 종료 (SIGTERM — scoped 정리·소켓 제거)
                        eprintln!("[daemon] 기존 데몬 정지 중 (소유권 이관)…");
                        let _ = std::process::Command::new("pkill")
                            .args(["-TERM", "-x", "cysd"])
                            .output();
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    let _ = std::process::Command::new("launchctl")
                        .args(["unload", "-w"])
                        .arg(&path)
                        .output(); // 재등록 대비 (실패 무시)
                    let out = std::process::Command::new("launchctl")
                        .args(["load", "-w"])
                        .arg(&path)
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(format!(
                            "launchctl load failed: {}",
                            String::from_utf8_lossy(&out.stderr).trim()
                        ));
                    }
                    // 기동 확인
                    let mut up = false;
                    for _ in 0..40 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if connect_raw().is_ok() {
                            up = true;
                            break;
                        }
                    }
                    println!(
                        "launchd 등록 완료: {} (로그인 자동 기동 + 사망 시 자동 재기동)",
                        path.display()
                    );
                    println!("데몬 가동: {}", if up { "확인됨" } else { "미확인 — log 확인" });
                    println!("⚠ 이후 nohup 수동 기동과 병행 금지 (flock 충돌 — launchd가 단독 소유)");
                    Ok(())
                }
                DaemonAction::Uninstall => {
                    let path = launchd_plist_path();
                    let _ = std::process::Command::new("launchctl")
                        .args(["unload", "-w"])
                        .arg(&path)
                        .output();
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    }
                    println!("launchd 등록 해제 완료 (데몬 정지됨 — 세션도 함께 종료)");
                    Ok(())
                }
                DaemonAction::Status => {
                    let path = launchd_plist_path();
                    let registered = path.exists();
                    let loaded = std::process::Command::new("launchctl")
                        .args(["list", LAUNCHD_LABEL])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    let alive = connect_raw().is_ok();
                    println!(
                        "registered={} loaded={} socket_alive={}",
                        registered, loaded, alive
                    );
                    if alive && !loaded {
                        println!("(데몬은 살아있지만 launchd 소유가 아님 — 수동/앱 기동 인스턴스)");
                    }
                    Ok(())
                }
            }
        }
        #[cfg(windows)]
        {
            const TASK: &str = "cysd";
            match action {
                DaemonAction::Install { takeover: _ } => {
                    let daemon = sibling_daemon_path()
                        .ok_or("cysd.exe not found next to cys.exe")?;
                    let out = std::process::Command::new("schtasks")
                        .args([
                            "/Create", "/TN", TASK, "/TR",
                            &format!("\"{}\"", daemon.display()),
                            "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
                        ])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                    }
                    println!("작업 스케줄러 등록 완료 (로그온 시 자동 기동). 사망 시 자동 재기동은 미지원 — CLI 자동기동이 보완한다.");
                    Ok(())
                }
                DaemonAction::Uninstall => {
                    let out = std::process::Command::new("schtasks")
                        .args(["/Delete", "/TN", TASK, "/F"])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                    }
                    println!("작업 스케줄러 등록 해제 완료");
                    Ok(())
                }
                DaemonAction::Status => {
                    let registered = std::process::Command::new("schtasks")
                        .args(["/Query", "/TN", TASK])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    let alive = connect_raw().is_ok();
                    println!("registered={registered} socket_alive={alive}");
                    Ok(())
                }
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = action;
            Err("이 OS에서는 미지원 (macOS launchd / Windows 작업 스케줄러만)".into())
        }
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn fmt_secs(s: u64) -> String {
    if s >= 3600 {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// T1-2 관제 보드 렌더링: org.status 1콜 → 사람/AI 모두 읽는 표
/// statusline stdin JSON에서 usage.report 파라미터(surface 제외)를 추출한다 — 순수 함수(테스트 핀).
/// `context_window.used_percentage`(서버 진실 ctx%)·`context_window_size`·`current_usage` 합(ctx_tokens,
/// input+cache_creation+cache_read = Phase 1 transcript 공식과 동일)·`rate_limits.five_hour/seven_day`
/// → rate 배열. 누락 필드는 안전하게 생략(rate 부재=무료/세션 첫 응답 전이면 빈 벡터).
fn statusline_to_report_params(v: &Value) -> Value {
    let cw = v.get("context_window");
    let ctx_pct = cw
        .and_then(|c| c.get("used_percentage"))
        .and_then(|x| x.as_f64());
    let ctx_window = cw
        .and_then(|c| c.get("context_window_size"))
        .and_then(|x| x.as_u64());
    let ctx_tokens = cw
        .and_then(|c| c.get("current_usage"))
        .map(|cu| {
            let g = |k: &str| cu.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            g("input_tokens") + g("cache_creation_input_tokens") + g("cache_read_input_tokens")
        })
        .filter(|&t| t > 0)
        .or_else(|| {
            cw.and_then(|c| c.get("total_input_tokens"))
                .and_then(|x| x.as_u64())
        });
    let mut rate = Vec::new();
    if let Some(rl) = v.get("rate_limits") {
        for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            if let Some(used) = rl
                .get(key)
                .and_then(|w| w.get("used_percentage"))
                .and_then(|x| x.as_f64())
            {
                let mut entry = json!({"label": label, "used_pct": used});
                if let Some(r) = rl
                    .get(key)
                    .and_then(|w| w.get("resets_at"))
                    .and_then(|x| x.as_f64())
                {
                    entry["resets_at"] = json!(r);
                }
                rate.push(entry);
            }
        }
    }
    let mut params = json!({ "rate": rate });
    if let Some(p) = ctx_pct {
        params["ctx_pct"] = json!(p);
    }
    if let Some(t) = ctx_tokens {
        params["ctx_tokens"] = json!(t);
    }
    if let Some(w) = ctx_window {
        params["ctx_window"] = json!(w);
    }
    params
}

/// statusline JSON → 사람이 읽는 한 줄 (`<model> · CTX n% · 5h n% · 7d n%`). rate는 있을 때만.
/// claude UI statusline에 그대로 표시된다(pane 헤더 배지와 별개·추가 표면).
fn statusline_human_line(v: &Value) -> String {
    let model = v
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|x| x.as_str())
        .unwrap_or("claude");
    let mut parts = vec![model.to_string()];
    if let Some(p) = v
        .get("context_window")
        .and_then(|c| c.get("used_percentage"))
        .and_then(|x| x.as_f64())
    {
        parts.push(format!("CTX {p:.0}%"));
    }
    if let Some(rl) = v.get("rate_limits") {
        for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            if let Some(u) = rl
                .get(key)
                .and_then(|w| w.get("used_percentage"))
                .and_then(|x| x.as_f64())
            {
                parts.push(format!("{label} {u:.0}%"));
            }
        }
    }
    parts.join(" · ")
}

/// cys-statusline.sh 래퍼 전용 — stdin의 claude statusline JSON을 읽어 usage.report로 push하고,
/// (quiet가 아니면) 사람용 statusline 한 줄을 stdout으로 출력한다.
/// ★불변: statusline 경로는 **절대 claude를 막지 않는다** — 빈 입력·파싱 실패·surface 미해결·
/// 데몬 부재 전부 exit 0으로 무해하게 흘린다.
fn run_usage_report_stdin(surface: &Option<String>, quiet: bool) -> i32 {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return 0;
    }
    let Ok(v) = serde_json::from_str::<Value>(&buf) else {
        return 0;
    };
    // push (surface 미해결·데몬 부재는 조용히 스킵 — 사람용 줄은 여전히 출력한다)
    if let Ok(sid) = target_surface(surface, &None) {
        let mut params = statusline_to_report_params(&v);
        params["surface_id"] = json!(sid);
        let _ = request("usage.report", params);
    }
    if !quiet {
        println!("{}", statusline_human_line(&v));
    }
    0
}

fn run_status(as_json: bool) -> i32 {
    let r = match request("org.status", json!({})) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if as_json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        return 0;
    }
    if r["paused"].as_bool() == Some(true) {
        println!(
            "⛔ PAUSED — {} (cys resume로 해제; 큐·스케줄 동결 중, 실행 중 에이전트 행동은 계속)",
            r["pause_info"]["reason"].as_str().unwrap_or("")
        );
    }
    let header = format!(
        "{:<14} {:<12} {:<8} {:<9} {:>4} {:>7} {:>5}  {}",
        "ROLE", "SURFACE", "AGENT", "STATE", "CTX", "IDLE", "QUEUE", "TASK/TITLE"
    );
    println!("{header}");
    for s in r["surfaces"].as_array().cloned().unwrap_or_default() {
        let exited = s["exited"].as_bool().unwrap_or(false);
        let state = if exited {
            "exited!".to_string()
        } else if s["agent_alive"].as_bool() == Some(false) {
            "agent✗".to_string()
        } else {
            s["status"]["state"].as_str().unwrap_or("-").to_string()
        };
        let ctx = s["status"]["context_pct"]
            .as_u64()
            .map(|v| format!("{v}%"))
            .unwrap_or_else(|| "-".into());
        let task = s["status"]["task"]
            .as_str()
            .filter(|t| !t.is_empty())
            .or(s["title"].as_str())
            .unwrap_or("");
        let queue_mark = if s["queue_paused"].as_bool() == Some(true) {
            format!("{}⏸", s["queue_depth"].as_u64().unwrap_or(0))
        } else {
            s["queue_depth"].as_u64().unwrap_or(0).to_string()
        };
        println!(
            "{:<14} {:<12} {:<8} {:<9} {:>4} {:>7} {:>5}  {}",
            s["role"].as_str().unwrap_or("-"),
            s["surface_ref"].as_str().unwrap_or("?"),
            s["agent"].as_str().unwrap_or("-"),
            state,
            ctx,
            fmt_secs(s["idle_secs"].as_u64().unwrap_or(0)),
            queue_mark,
            task.chars().take(40).collect::<String>(),
        );
    }
    let pending = r["feed"]["pending"].as_u64().unwrap_or(0);
    if pending > 0 {
        println!(
            "feed: {pending} pending (oldest {}) — `cys feed list --status pending`",
            fmt_secs(r["feed"]["oldest_pending_age_secs"].as_u64().unwrap_or(0))
        );
    }
    let health = r["health_recent"].as_array().cloned().unwrap_or_default();
    if !health.is_empty() {
        println!("health (최근 {}건):", health.len().min(5));
        for h in health.iter().take(5) {
            println!(
                "  surface:{} [{}] {}",
                h["surface_id"],
                h["rule"].as_str().unwrap_or("?"),
                h["line"].as_str().unwrap_or("").chars().take(80).collect::<String>(),
            );
        }
    }
    if let Some(todo) = r["todo"].as_object() {
        if !todo.is_empty() {
            println!("todo:");
            for (path, v) in todo {
                let name = path.rsplit('/').next().unwrap_or(path);
                println!(
                    "  {name}: {}/{} (updated {} ago)",
                    v["done"],
                    v["total"],
                    fmt_secs(v["age_secs"].as_u64().unwrap_or(0))
                );
            }
        }
    }
    0
}

/// role 우선, 없으면 --surface, 없으면 env 폴백으로 대상 결정 (cycle/recover/reinject 공용)
fn resolve_role_or_surface(
    role: &Option<String>,
    surface: &Option<String>,
) -> Result<u64, String> {
    if role.is_some() {
        return target_surface(&None, role);
    }
    let explicit = parse_explicit_surface(surface)?;
    match explicit {
        Some(sid) => Ok(sid),
        None => Err("need --role or --surface".into()),
    }
}

/// T2-4 컨텍스트 사이클 집행기 — 게이트는 화면 마커가 아니라 파일 mtime+해시.
#[allow(clippy::too_many_arguments)]
fn run_cycle_agent(
    role: Option<String>,
    surface: Option<String>,
    verifier: Option<String>,
    save_files: Vec<String>,
    clear_cmd: Option<String>,
    resume_text: Option<String>,
    timeout: u64,
    force_no_verify: bool,
) -> i32 {
    let result = (|| -> Result<(), String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        if entry["exited"].as_bool() == Some(true) {
            return Err(format!("surface:{sid} 이미 종료됨"));
        }
        let role_name = entry["role"].as_str().unwrap_or("worker").to_string();
        // soul 축2: master self-clear 금지 — 검증자 없는 master cycle 거부
        if role_name == "master" && verifier.is_none() {
            return Err(
                "master cycle엔 --verifier <role>이 필수 (self-clear 금지 — 2-phase handshake)"
                    .into(),
            );
        }
        // clear 명령 선확정 — 저장만 시키고 clear 못하는 어정쩡한 상태 방지
        let agent = entry["agent"].as_str().map(String::from);
        let clear = match clear_cmd {
            Some(c) => c,
            None => {
                let a = agent
                    .clone()
                    .ok_or("agent 메타 없음 — --clear-cmd 명시 필요")?;
                load_agent_spec(&a)?["clear_cmd"]
                    .as_str()
                    .ok_or_else(|| {
                        format!("agents.json '{a}'에 clear_cmd 없음 — --clear-cmd 명시 필요")
                    })?
                    .to_string()
            }
        };
        // 저장 검증 파일 확정 (기본: <cwd>/_round/SESSION_STATE.md + *_TODO.md 자동 탐지)
        let cwd = entry["live_cwd"]
            .as_str()
            .or(entry["cwd"].as_str())
            .unwrap_or(".")
            .to_string();
        let files: Vec<String> = if !save_files.is_empty() {
            save_files
        } else {
            // 기본 탐지: <cwd>/_round 전체 + pack/round의 '대상 역할 소유분'만 — 절대지침이
            // todo·SESSION_STATE 정본을 pack/round로 통일했으므로(앵커5·6) 거기 저장분도
            // 검증 대상이다. 단 pack/round는 전 노드 공유 디렉터리라 다른 노드의 갱신이
            // 저장 게이트를 거짓 통과시킬 수 있어(타이밍 의존) 대상 역할 파일로 한정한다.
            let mut v = Vec::new();
            let cwd_round = std::path::PathBuf::from(format!("{cwd}/_round"));
            let ss = cwd_round.join("SESSION_STATE.md");
            if ss.exists() {
                v.push(ss.to_string_lossy().into_owned());
            }
            if let Ok(entries) = std::fs::read_dir(&cwd_round) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.ends_with("_TODO.md") {
                        v.push(e.path().to_string_lossy().into_owned());
                    }
                }
            }
            let pack_round = cys::pack::pack_dir().join("round");
            let role_todo = format!(
                "{}_TODO.md",
                role_name.to_uppercase().replace('-', "_")
            );
            let pt = pack_round.join(&role_todo);
            if pt.exists() {
                v.push(pt.to_string_lossy().into_owned());
            }
            // SESSION_STATE(pack 정본)는 master 소관 — master cycle일 때만 게이트에 포함
            if role_name == "master" {
                let pss = pack_round.join("SESSION_STATE.md");
                if pss.exists() {
                    v.push(pss.to_string_lossy().into_owned());
                }
            }
            v
        };
        if files.is_empty() && !force_no_verify {
            return Err(
                "저장 검증 파일 없음 — --save-file로 지정하거나 --force-no-verify(위험)".into(),
            );
        }
        let start_time = std::time::SystemTime::now();
        let baseline: Vec<(String, Option<String>)> = files
            .iter()
            .map(|f| (f.clone(), sha256_file(f)))
            .collect();

        // 1) 저장 지시
        eprintln!("[cycle 1/5] 저장 지시 주입 → surface:{sid} ({role_name})");
        inject_text(sid, "[CYCLE] 컨텍스트 순환 절차 개시. 지금 즉시: ① 자기 TODO 파일(~/.cys/pack/round/<역할>_TODO.md)과 SESSION_STATE(_round/ 또는 pack round/ 정본)에 현재 작업 상태·미해결 게이트·다음 액션을 저장하라. ② 저장 완료 후 다른 출력 없이 plain 한 줄로 CYCLE-SAVED 를 출력하라.")?;

        // 2) 파일 변화 게이트 (화면 마커는 참고 신호일 뿐 — reward-hack·stale 마커 차단)
        if !baseline.is_empty() {
            eprintln!("[cycle 2/5] 저장 파일 검증 대기 (mtime+해시, 최대 {timeout}s)");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let mut verified = false;
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_secs(2));
                for (f, base_hash) in &baseline {
                    let mtime_ok = std::fs::metadata(f)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .map(|t| t > start_time)
                        .unwrap_or(false);
                    if mtime_ok && sha256_file(f) != *base_hash {
                        verified = true;
                        break;
                    }
                }
                if verified {
                    break;
                }
            }
            if !verified {
                return Err(format!(
                    "저장 검증 실패 — {timeout}s 내 파일 갱신 없음. cycle 중단 (clear 미실행)"
                ));
            }
            eprintln!("[cycle] 저장 검증 통과");
        } else {
            eprintln!("[cycle 2/5] ⚠ 파일 검증 생략 (--force-no-verify)");
        }

        // 3) 2-phase handshake — 검증자 부재 시 clear 금지 (soul 규칙)
        if let Some(v) = &verifier {
            eprintln!("[cycle 3/5] 검증자 '{v}' handshake");
            let vr = request("system.resolve_role", json!({"role": v}))
                .map_err(|e| format!("검증자 '{v}' 부재 — clear 금지 (self-clear 차단): {e}"))?;
            let vsid = vr["surface_id"].as_u64().ok_or("bad verifier resolve")?;
            let body: String = baseline
                .iter()
                .map(|(f, _)| format!("{f} (sha256: {})", sha256_file(f).unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("\n");
            let push = request(
                "feed.push",
                json!({"kind": "cycle-verify",
                       "title": format!("[CYCLE-VERIFY] {role_name} 저장 검증 요청"),
                       "body": body, "surface_id": sid, "wait": false}),
            )?;
            let req_id = push["request_id"].as_str().unwrap_or("").to_string();
            inject_text(vsid, &format!("[CYCLE-VERIFY] role '{role_name}'(surface:{sid})의 컨텍스트 순환 전 저장 검증 요청. SESSION_STATE/TODO 파일이 방금 갱신되었는지 확인하고 `cys feed reply {req_id} allow` 또는 `cys feed reply {req_id} deny`로 판정하라."))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let decision = loop {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                let items = request("feed.list", json!({}))?;
                let found = items["items"]
                    .as_array()
                    .and_then(|a| {
                        a.iter()
                            .find(|i| i["request_id"].as_str() == Some(req_id.as_str()))
                            .cloned()
                    });
                if let Some(item) = found {
                    if item["status"].as_str() == Some("resolved") {
                        break item["decision"].as_str().map(String::from);
                    }
                }
            };
            match decision.as_deref() {
                Some("allow") | Some("yes") | Some("approve") => {
                    eprintln!("[cycle] 검증자 승인 — clear 진행")
                }
                Some(d) => return Err(format!("검증자 거부({d}) — cycle 중단")),
                None => return Err("검증자 응답 없음 (timeout) — clear 중단".into()),
            }
        } else {
            eprintln!("[cycle 3/5] (검증자 미지정 — handshake 생략)");
        }

        // 4) 입력 버퍼 정리 + clear
        eprintln!("[cycle 4/5] 입력 버퍼 정리 + '{clear}'");
        request("surface.send_key", json!({"surface_id": sid, "key": "C-u"}))?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        request(
            "surface.send_text",
            json!({"surface_id": sid, "text": clear, "quiet": true}),
        )?;
        request(
            "surface.send_key",
            json!({"surface_id": sid, "key": "Return"}),
        )?;
        std::thread::sleep(std::time::Duration::from_secs(4));

        // 5) 디렉티브 재주입 + 재개 포인터
        eprintln!("[cycle 5/5] 디렉티브 재주입 + 재개 포인터");
        let directive = compose_directive(&role_name)?;
        inject_text(sid, &directive)?;
        std::thread::sleep(std::time::Duration::from_secs(2));
        let resume = resume_text.unwrap_or_else(|| {
            "[RESUME] 컨텍스트 순환 완료. _round/SESSION_STATE.md와 자기 TODO를 읽고 직전 작업을 이어가라.".into()
        });
        inject_text(sid, &resume)?;
        println!("cycle complete → surface:{sid} ({role_name})");
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// T2-5 노드 복구: 죽은 에이전트를 같은 surface에서 재기동 + 지침 재주입 + 복원 포인터
fn run_node_recover(surface: Option<String>, role: Option<String>) -> i32 {
    let result = (|| -> Result<(), String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        if entry["exited"].as_bool() == Some(true) {
            return Err(format!(
                "surface:{sid} 셸 자체가 종료됨 — `cys restore`로 재기동하라"
            ));
        }
        let agent = entry["agent"]
            .as_str()
            .ok_or("agent 메타 없음 (launch-agent로 기동된 pane만 복구 가능)")?
            .to_string();
        if entry["agent_alive"].as_bool() == Some(true) {
            return Err(format!(
                "agent '{agent}'가 살아있는 것으로 보임 — 강제 재기동은 close-surface 후 launch-agent"
            ));
        }
        let role_name = entry["role"].as_str().unwrap_or("worker").to_string();
        let spec = load_agent_spec(&agent)?;
        eprintln!("[node-recover] surface:{sid} 위에 {agent} 재기동 (role={role_name})");
        // 셸 입력 잔재 정리 후 기동 (resume 플래그로 대화 기억 복원 시도)
        request("surface.send_key", json!({"surface_id": sid, "key": "C-u"}))?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        boot_agent_on_surface(sid, &role_name, &agent, &spec, true)?;
        inject_text(sid, "[RECOVER] 너는 방금 재기동되었다. _round/SESSION_STATE.md와 자기 TODO 파일을 읽어 작업 기억을 복원한 뒤 master에게 복귀를 1줄 push로 보고하라. 작업 재개는 master 지시를 따른다.")?;
        println!("recovered surface:{sid} ({agent})");
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// T2-6 조직 복원: 토폴로지 스냅샷 기준으로 죽은 역할 일괄 재기동 (작업 재개는 master 판단)
fn run_restore(cwd: Option<String>, include_master: bool, no_resume: bool) -> i32 {
    let result = (|| -> Result<(usize, usize), String> {
        let topo = request("system.topology", json!({}))?;
        let live: std::collections::HashSet<String> = topo["live"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e["role"].as_str().map(String::from))
            .collect();
        let saved = topo["saved"].as_array().cloned().unwrap_or_default();
        if saved.is_empty() {
            println!("(토폴로지 스냅샷 없음 — launch-agent로 역할을 기동하면 자동 기록된다)");
            return Ok((0, 0));
        }
        let (mut ok, mut fail) = (0usize, 0usize);
        for entry in saved {
            let Some(role) = entry["role"].as_str() else {
                continue;
            };
            if role == "master" && !include_master {
                println!("· {role}: 제외 (restore 실행자가 보통 master — --include-master로 포함)");
                continue;
            }
            if live.contains(role) {
                println!("· {role}: 이미 가동 중 — 건너뜀");
                continue;
            }
            let Some(agent) = entry["agent"].as_str() else {
                println!("· {role}: agent 미상 — 건너뜀 (claim-role로 등록된 pane)");
                continue;
            };
            let target_cwd = cwd
                .clone()
                .or_else(|| entry["cwd"].as_str().map(String::from));
            println!("· {role}: {agent} 재기동…");
            if run_launch_agent_opts(role, agent, target_cwd, !no_resume) == 0 {
                ok += 1;
                if let Ok(r) = request("system.resolve_role", json!({"role": role})) {
                    if let Some(sid) = r["surface_id"].as_u64() {
                        let _ = inject_text(sid, "[RESTORE] 조직 복원 절차다. _round/SESSION_STATE.md와 자기 TODO를 읽고 상태를 복원하라. ★작업 재개는 하지 말고 master의 지시를 기다려라.");
                    }
                }
            } else {
                fail += 1;
                println!("· {role}: 기동 실패 — 나머지 역할 계속 진행");
            }
        }
        Ok((ok, fail))
    })();
    match result {
        Ok((ok, fail)) => {
            println!("restore 완료: 재기동 {ok} · 실패 {fail} · 현황 `cys status`");
            if fail > 0 {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// T2-7 디렉티브 드리프트 감지·재주입: --check면 각성 핑 먼저, 무응답 시에만 재주입
fn run_reinject(
    role: Option<String>,
    surface: Option<String>,
    check: bool,
    timeout: u64,
) -> i32 {
    let result = (|| -> Result<(), String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        let role_name = role
            .clone()
            .or_else(|| entry["role"].as_str().map(String::from))
            .ok_or("role 미상 — --role 지정 필요")?;
        if check {
            // 마커를 핑 텍스트에 통째로 넣지 않는다 — 주입 텍스트의 터미널 에코가
            // wait_for에 매칭되는 false ACK(자기-에코 오탐)를 차단 (토큰 분리 조합 지시)
            let marker = format!("DIRECTIVE-ACK-{}", std::process::id());
            let cursor = request("surface.read_text", json!({"surface_id": sid}))?
                ["latest_cursor"]
                .as_u64()
                .unwrap_or(0);
            inject_text(sid, &format!("지침 각성 확인 핑: 너의 절대지침(디렉티브)이 컨텍스트에 살아있다면, 다음 두 토큰을 공백 없이 이어붙인 한 줄을 plain으로 출력하라: 'DIRECTIVE-ACK-' 그리고 '{}'", std::process::id()))?;
            let r = request(
                "surface.wait_for",
                json!({"surface_id": sid, "pattern": marker,
                       "timeout_secs": timeout, "since_line": cursor}),
            )?;
            if r["matched"].as_bool() == Some(true) {
                println!("디렉티브 생존 확인 (ACK 수신) — 재주입 불필요");
                return Ok(());
            }
            eprintln!("[reinject] ACK 없음 ({timeout}s) — 드리프트 판정, 재주입 진행");
        }
        let directive = compose_directive(&role_name)?;
        inject_text(sid, &directive)?;
        println!(
            "reinjected {} bytes → surface:{sid} ({role_name})",
            directive.len()
        );
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// 완화책 ③: scoped 실행 — 새 프로세스 그룹에서 실행하고 원장에 등록,
/// 종료 시 그룹 전체를 강제 종료하여 서버가 절대 누적되지 않게 한다.
/// 자식의 종료 코드를 그대로 반환한다 (시그널 사망 = 128+signo).
fn run_scoped(surface: Option<String>, command: Vec<String>) -> Result<i32, String> {
    if command.is_empty() {
        return Err("no command given".into());
    }
    let sid = parse_explicit_surface(&surface)?
        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));

    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let pid = child.id();
    let pgid = pid as i64; // setsid → pgid == pid (unix); ignored on windows

    // setsid로 분리된 자식은 터미널 시그널(Ctrl-C 등)에 면역 — CLI가 죽기 전에
    // 그룹을 대신 죽여야 '종료 시 그룹 강제 종료' 보장이 유지된다.
    // (원장 deregister는 핸들러에서 생략 — dead-pid 항목은 watchdog이 자동 회수)
    #[cfg(unix)]
    {
        SCOPED_PGID.store(pgid as i32, std::sync::atomic::Ordering::SeqCst);
        let handler =
            scoped_cleanup_handler as extern "C" fn(libc::c_int) as *const () as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, handler);
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGHUP, handler);
        }
    }

    if let Err(e) = request(
        "ledger.register",
        json!({"pid": pid, "pgid": pgid, "cmd": command.join(" "), "surface_id": sid, "scoped": true}),
    ) {
        // 등록 실패 = 데몬이 생명주기를 보장할 수 없음 → 그룹 즉시 강제 종료.
        // 살려두면 어떤 거버넌스(watchdog·reap_orphan_ledger)에도 안 보이는 영구 고아가 된다.
        kill_group(pid, pgid);
        let _ = child.wait();
        return Err(format!(
            "ledger.register failed — scoped group killed (pid={pid}): {e}"
        ));
    }
    eprintln!("[scoped pid={pid} registered in ledger]");

    let wait_res = child.wait();

    // Force-kill the whole group: anything the command left behind dies with it.
    // wait가 Err여도 정리는 무조건 수행한다.
    kill_group(pid, pgid);
    let _ = request("ledger.deregister", json!({"pid": pid}));

    let status = wait_res.map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let code = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|s| 128 + s).unwrap_or(1)
    });
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(1);
    eprintln!("[scoped pid={pid} exited ({status}); process group force-killed and deregistered]");
    Ok(code)
}

fn kill_group(pid: u32, pgid: i64) {
    #[cfg(unix)]
    {
        let _ = pid;
        unsafe {
            libc::killpg(pgid as i32, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = pgid;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

#[cfg(unix)]
static SCOPED_PGID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// async-signal-safe 핸들러: killpg·_exit만 호출 (소켓 I/O·할당 금지)
#[cfg(unix)]
extern "C" fn scoped_cleanup_handler(sig: libc::c_int) {
    let pgid = SCOPED_PGID.load(std::sync::atomic::Ordering::SeqCst);
    if pgid > 0 {
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    unsafe { libc::_exit(128 + sig) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 회귀 박제: boot의 설치 판정이 경로형 cmd(틸드 절대경로 — agy)를 which로 넘기면
    /// 틸드 비확장으로 '미설치' 오판 → 4종 의무 부트가 조용히 3종이 된다.
    /// expand_tilde가 '~/'를 홈으로 확장해 파일 존재 판정이 성립해야 한다.
    #[test]
    fn expand_tilde_resolves_home_prefix() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(expand_tilde("~/.local/bin/agy"), home.join(".local/bin/agy"));
        // 비틸드 경로·단순 명령어는 그대로
        assert_eq!(
            expand_tilde("/usr/bin/env"),
            std::path::PathBuf::from("/usr/bin/env")
        );
        assert_eq!(expand_tilde("codex"), std::path::PathBuf::from("codex"));
        // '~user' 형태는 확장하지 않는다 (보수적 — 그대로 존재 판정)
        assert_eq!(expand_tilde("~root/x"), std::path::PathBuf::from("~root/x"));
    }

    /// ★불변식 박제: compose_directive는 디렉티브 → soul.md → 장기메모리 색인 → 스킬 색인
    /// 순서로 조립한다. 메모리 색인 누락은 "리뷰어·워커 장기기억 0" 결함의 재발이므로
    /// 섹션 존재와 순서를 기계 검증한다 (launch/reinject/cycle 공용 경로).
    #[test]
    fn compose_directive_includes_memory_index_after_soul() {
        let td = std::env::temp_dir().join(format!("cys-compose-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        for sub in ["directives", "memory", "skills/demo"] {
            std::fs::create_dir_all(td.join(sub)).unwrap();
        }
        std::fs::write(td.join("directives/WORKER_DIRECTIVE.md"), "# WORKER 절대지침\n").unwrap();
        std::fs::write(td.join("soul.md"), "soul-marker\n").unwrap();
        std::fs::write(td.join("memory/MEMORY.md"), "memory-index-marker\n").unwrap();
        std::fs::write(
            td.join("skills/demo/SKILL.md"),
            "name: demo\ndescription: d\n",
        )
        .unwrap();

        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);
        let out = compose_directive("worker").expect("compose 실패");
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);

        let pos = |needle: &str| out.find(needle).unwrap_or_else(|| panic!("누락: {needle}"));
        let d = pos("WORKER 절대지침");
        let s = pos("■ soul.md");
        let m = pos("■ 장기메모리 색인");
        let k = pos("■ 보유 스킬 색인");
        assert!(out.contains("memory-index-marker"), "메모리 색인 본문 미동봉");
        assert!(
            out.contains("memory/MEMORY.md") && out.contains(td.to_str().unwrap()),
            "메모리 절대경로 미표기 — 노드가 위치를 추론하게 된다"
        );
        assert!(d < s && s < m && m < k, "조립 순서 위반: 디렉티브<soul<메모리<스킬");
    }

    /// ★불변식 박제 (절대지침 앵커1-b): 탭 타이틀 = "{role}-{agent} · {워크플로우 폴더명}".
    /// 폴더를 알 수 없는 경계(루트·빈 문자열·None)는 역할-에이전트로 폴백.
    #[test]
    fn workflow_title_embeds_folder_name() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            workflow_title("worker", "claude", &some("/Users/x/Desktop/CYSjavis/cys-terminal")),
            "worker-claude · cys-terminal"
        );
        // 후행 슬래시 정규화
        assert_eq!(
            workflow_title("reviewer-gemini", "gemini", &some("/a/b/my-workflow/")),
            "reviewer-gemini-gemini · my-workflow"
        );
        // 상대 경로도 basename
        assert_eq!(workflow_title("worker", "claude", &some("proj")), "worker-claude · proj");
        // Windows 경로 + 후행 백슬래시 정규화 (file_name()이 None이 되는 케이스 방어)
        assert_eq!(
            workflow_title("worker", "claude", &some("C:\\Users\\x\\my-wf")),
            "worker-claude · my-wf"
        );
        assert_eq!(
            workflow_title("worker", "claude", &some("C:\\Users\\x\\my-wf\\")),
            "worker-claude · my-wf"
        );
        // 한글/유니코드 폴더명
        assert_eq!(
            workflow_title("worker", "claude", &some("/a/자비스-워크플로우")),
            "worker-claude · 자비스-워크플로우"
        );
        // 연속 구분자도 마지막 비공백 컴포넌트
        assert_eq!(workflow_title("worker", "claude", &some("//a//b")), "worker-claude · b");
        // 경계: 루트·빈 문자열·None·Windows 드라이브 루트·.. → 폴백
        assert_eq!(workflow_title("worker", "claude", &some("/")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &None), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("C:\\")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("D:/")), "worker-claude");
        // ".." 은 폴더명으로 부적절하지 않음 — 실제 디렉터리 참조라 그대로 표시(상위 폴더 기동 시)
        assert_eq!(workflow_title("worker", "claude", &some("/a/b/..")), "worker-claude · ..");
    }

    #[test]
    fn duration_basic_units() {
        assert_eq!(parse_duration_secs("90s"), Ok(90));
        assert_eq!(parse_duration_secs("20m"), Ok(1200));
        assert_eq!(parse_duration_secs("2h"), Ok(7200));
        assert_eq!(parse_duration_secs("1d"), Ok(86400));
    }

    #[test]
    fn duration_compound() {
        // 1h30m = 3600 + 1800
        assert_eq!(parse_duration_secs("1h30m"), Ok(5400));
        // 누적 순서 무관하게 합산
        assert_eq!(parse_duration_secs("1m30s"), Ok(90));
        assert_eq!(parse_duration_secs("1h2m3s"), Ok(3723));
    }

    #[test]
    fn duration_zero_is_ok() {
        // 0초는 형식상 유효 (값 검증은 호출부 책임)
        assert_eq!(parse_duration_secs("0s"), Ok(0));
    }

    #[test]
    fn duration_rejects_bad_input() {
        // 단위 없는 순수 숫자
        assert!(parse_duration_secs("5").is_err());
        // 빈 문자열
        assert!(parse_duration_secs("").is_err());
        // 숫자 없는 단위
        assert!(parse_duration_secs("s").is_err());
        // 알 수 없는 단위
        assert!(parse_duration_secs("5x").is_err());
        // 단위 뒤 trailing 숫자 (미완성)
        assert!(parse_duration_secs("5m3").is_err());
        assert!(parse_duration_secs("1h30").is_err());
        // 공백·기호
        assert!(parse_duration_secs("1 h").is_err());
        assert!(parse_duration_secs("-5s").is_err());
    }

    #[test]
    fn duration_overflow_is_error_not_panic() {
        // R3 버그 가드: n은 u64로 파싱되나 n*86400이 u64를 넘는 입력.
        // 과거: debug=패닉, release=silent wrap(엉뚱한 발화 epoch). 이제 Err로 거부.
        assert!(parse_duration_secs("9999999999999999d").is_err());
        // 곱셈은 안 넘쳐도 누적 합(checked_add)에서 넘치는 경로
        let near_max = format!("{}s", u64::MAX);
        assert_eq!(parse_duration_secs(&near_max), Ok(u64::MAX));
        assert!(parse_duration_secs(&format!("{}s1s", u64::MAX)).is_err());
        // u64::MAX 자체는 s 단위(×1)로 정확히 통과 — 상한 경계 보존
        assert!(parse_duration_secs(&format!("{}m", u64::MAX)).is_err()); // ×60 overflow
        // 정상 큰 값은 여전히 통과 (회귀 아님)
        assert_eq!(parse_duration_secs("100d"), Ok(100 * 86400));
    }

    #[test]
    fn cli_glob_anchored_full_match() {
        // 리터럴은 전체 일치만 (부분 일치 거부 — handlers::glob_match의 ^…$ 앵커와 동일 의미)
        assert!(cli_glob_match("reviewer", "reviewer"));
        assert!(!cli_glob_match("reviewer", "reviewer-gemini"));
        assert!(!cli_glob_match("reviewer", "xreviewer"));
        assert!(!cli_glob_match("view", "reviewer"));
    }

    #[test]
    fn cli_glob_star_semantics() {
        // '*'는 빈 문자열 포함 임의 길이 매치
        assert!(cli_glob_match("*", ""));
        assert!(cli_glob_match("*", "anything"));
        assert!(cli_glob_match("reviewer-*", "reviewer-gemini"));
        assert!(cli_glob_match("reviewer-*", "reviewer-")); // * = 빈 매치
        assert!(!cli_glob_match("reviewer-*", "reviewer")); // 하이픈 리터럴 불일치
        // 중간 '*'
        assert!(cli_glob_match("a*z", "az"));
        assert!(cli_glob_match("a*z", "abcz"));
        assert!(!cli_glob_match("a*z", "abc"));
    }

    #[test]
    fn cli_glob_backtracking_and_multistar() {
        // 백트래킹: 다중 '*'와 탐욕 매칭이 올바르게 되돌아오는지 (재귀 매처의 고전 버그 지점)
        assert!(cli_glob_match("*-*", "worker-2"));
        assert!(cli_glob_match("w*r*2", "worker-2"));
        assert!(cli_glob_match("**", "abc")); // 연속 '*'도 안전
        assert!(cli_glob_match("a**c", "abbbc"));
        // 매칭 실패 케이스 — '*'가 있어도 리터럴 제약 위반
        assert!(!cli_glob_match("a*c", "abd"));
        assert!(!cli_glob_match("*x", "abc"));
    }

    #[test]
    fn cli_glob_literal_star_in_pattern_only() {
        // value 안의 '*'는 리터럴로 취급 (패턴의 '*'만 와일드카드)
        assert!(cli_glob_match("a*", "a*literal"));
        assert!(!cli_glob_match("abc", "a*c")); // 패턴이 리터럴이면 value의 '*'와 불일치
    }

    /// handlers::glob_match(regex판, 데몬측)과 1:1 동일한 명세 (독립 오라클).
    /// '*'→".*", 나머지는 regex escape 후 ^…$ 앵커. 재귀 cli_glob_match가 이 명세에서
    /// 갈리면 CLI측 ACL(--to 글롭 브로드캐스트)이 데몬측과 비대칭 동작한다.
    fn regex_glob_oracle(pattern: &str, value: &str) -> bool {
        let mut re = String::from("^");
        for ch in pattern.chars() {
            if ch == '*' {
                re.push_str(".*");
            } else {
                re.push_str(&regex::escape(&ch.to_string()));
            }
        }
        re.push('$');
        regex::Regex::new(&re)
            .map(|r| r.is_match(value))
            .unwrap_or(false)
    }

    #[test]
    fn cli_glob_agrees_with_regex_oracle_over_corpus() {
        // 패턴·값 전수 곱집합에서 재귀 cli_glob_match와 regex 명세가 완전 일치해야 한다.
        // (handlers.rs의 대칭 테스트와 짝 — 두 바이너리 모두 같은 명세에 핀 고정.)
        // 단, regex '.'은 \n 미매치이므로 값에 개행을 넣지 않는다(역할명 무개행 전제와 일치).
        let patterns = [
            "", "*", "**", "a", "a*", "*a", "*a*", "a*b", "a**b", "a*b*c", "reviewer-*", "*-*",
            "w*r*2", "abc", "a.b", "a+b", "a?b", "[x]", "a*z", "**a**",
        ];
        let values = [
            "", "a", "ab", "abc", "a*literal", "reviewer-gemini", "reviewer-", "reviewer",
            "worker-2", "a.b", "axb", "a+b", "a?b", "[x]", "az", "abz", "abcz", "x", "-", "a-b-c",
        ];
        for p in patterns {
            for v in values {
                assert_eq!(
                    cli_glob_match(p, v),
                    regex_glob_oracle(p, v),
                    "glob 비대칭: pattern={p:?} value={v:?} (recursive={} regex={})",
                    cli_glob_match(p, v),
                    regex_glob_oracle(p, v),
                );
            }
        }
    }

    #[test]
    fn parse_explicit_surface_variants() {
        // None은 그대로 통과 (호출처가 의미 결정)
        assert_eq!(parse_explicit_surface(&None), Ok(None));
        // 유효 ref → Some
        assert_eq!(parse_explicit_surface(&Some("31".into())), Ok(Some(31)));
        assert_eq!(parse_explicit_surface(&Some("surface:7".into())), Ok(Some(7)));
        // 잘못된 형식 → Err
        assert!(parse_explicit_surface(&Some("nope".into())).is_err());
        assert!(parse_explicit_surface(&Some("-1".into())).is_err());
    }

    /// T5 Phase 2-A: claude statusline stdin JSON → usage.report 파라미터 추출 핀.
    /// 공식 stdin 스키마(used_percentage·current_usage 합·rate_limits)를 회귀 박제한다.
    #[test]
    fn statusline_params_full_schema() {
        let v = json!({
            "context_window": {
                "context_window_size": 200000,
                "used_percentage": 41.6,
                "current_usage": {
                    "input_tokens": 1000,
                    "cache_creation_input_tokens": 2000,
                    "cache_read_input_tokens": 80000,
                    "output_tokens": 5000
                }
            },
            "rate_limits": {
                "five_hour": {"used_percentage": 41.0, "resets_at": 1781314865},
                "seven_day": {"used_percentage": 12.0, "resets_at": 1781781650}
            }
        });
        let p = statusline_to_report_params(&v);
        assert_eq!(p["ctx_pct"].as_f64(), Some(41.6));
        assert_eq!(p["ctx_window"].as_u64(), Some(200000));
        // ctx_tokens = input + cache_creation + cache_read (output 제외) = 83000
        assert_eq!(p["ctx_tokens"].as_u64(), Some(83000));
        let rate = p["rate"].as_array().unwrap();
        assert_eq!(rate.len(), 2);
        assert_eq!(rate[0]["label"], json!("5h"));
        assert_eq!(rate[0]["used_pct"].as_f64(), Some(41.0));
        assert_eq!(rate[0]["resets_at"].as_f64(), Some(1781314865.0));
        assert_eq!(rate[1]["label"], json!("7d"));
    }

    /// rate_limits 부재(무료/세션 첫 응답 전): ctx만 추출, rate는 빈 벡터 — ctx 배지만 작동.
    #[test]
    fn statusline_params_no_rate_limits() {
        let v = json!({
            "context_window": {"context_window_size": 1000000, "used_percentage": 8.0}
        });
        let p = statusline_to_report_params(&v);
        assert_eq!(p["ctx_pct"].as_f64(), Some(8.0));
        assert_eq!(p["ctx_window"].as_u64(), Some(1000000));
        assert_eq!(p["rate"].as_array().unwrap().len(), 0);
        assert!(p.get("ctx_tokens").is_none(), "current_usage·total 없으면 ctx_tokens 생략");
    }

    /// 사람용 statusline 한 줄 포맷 — rate는 있을 때만, 모델명 부재 시 "claude" 폴백.
    #[test]
    fn statusline_human_line_format() {
        let v = json!({
            "model": {"display_name": "Opus 4.8"},
            "context_window": {"used_percentage": 42.0},
            "rate_limits": {
                "five_hour": {"used_percentage": 41.0},
                "seven_day": {"used_percentage": 12.0}
            }
        });
        assert_eq!(statusline_human_line(&v), "Opus 4.8 · CTX 42% · 5h 41% · 7d 12%");
        let v2 = json!({"context_window": {"used_percentage": 8.0}});
        assert_eq!(statusline_human_line(&v2), "claude · CTX 8%");
    }

    #[test]
    fn hook_command_is_os_aware_and_targets_session_start() {
        // SessionStart hook 명령은 타깃 OS에서 실행 가능한 형태여야 한다.
        // 회귀 가드: 바닐라 Windows 셸은 `.sh`를 인터프리터 없이 실행 못 하고 "open with"
        // 대화상자를 띄운다(claude-code #21847·#24097) → /clear 후 자동 재주입(autopilot 축2)
        // 무력화. Unix는 기존 `sh` 동작을 그대로 보존(제로 회귀).
        let cmd = hook_command(std::path::Path::new("/pack"));
        // 어느 OS든 항상 동봉된 session-start.sh를 가리킨다
        assert!(
            cmd.contains("hooks/session-start.sh") || cmd.contains("hooks\\session-start.sh"),
            "must target the bundled hook script: {cmd:?}"
        );
        // 인터프리터를 통해 호출한다 — 스크립트 경로를 명령 선두에 그대로 두면(=`<path>.sh`)
        // Windows 셸이 파일 연결로 가로채므로 금지
        let interp = cmd.split_whitespace().next().unwrap_or("");
        assert!(
            interp == "sh" || interp == "bash",
            "hook must be invoked via a shell interpreter, got: {interp:?}"
        );

        #[cfg(unix)]
        {
            // Unix: 기존 계약 박제 — 정확히 `sh <path>` (동작 변경 없음)
            assert_eq!(cmd, "sh /pack/hooks/session-start.sh");
        }
        #[cfg(windows)]
        {
            // Windows: `sh` 맨 이름 대신 Git Bash가 보장하는 `bash`로 호출 —
            // Claude Code가 Windows에서 `.sh` 해석에 찾는 인터프리터와 일치
            assert!(cmd.starts_with("bash "), "windows must use bash: {cmd:?}");
        }
    }

    /// 기동 화면의 평탄화(공백 제거)를 테스트에서 동일하게 재현하는 헬퍼.
    /// boot_agent_on_surface가 `text.chars().filter(|c| !c.is_whitespace())`로
    /// 만드는 입력과 1:1 동일해야 screen_shows_launch_failure 판정이 핀 고정된다.
    fn flatten_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn launch_failure_detection_is_cross_platform() {
        // 회귀 가드: launch-agent 준비 폴링의 사망 감지가 Unix 셸 오류만 잡으면
        // Windows(PowerShell/cmd)에서 기동 실패를 못 보고 죽은 셸에 지침을 주입한다.
        // hook_command OS 대칭화와 같은 결: 양 OS의 "명령 못 찾음"을 모두 잡아야 한다.

        // --- Unix: 기존 계약 박제 (제로 회귀) ---
        // zsh: "command not found: foo"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "zsh:1: command not found: claude-bogus"
        )));
        // bash: "foo: command not found"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "bash: claude-bogus: command not found"
        )));
        // 직접 바이너리 실행 실패: "No such file or directory"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "./claude-bogus: No such file or directory"
        )));
        // "not found in PATH" 표현
        assert!(screen_shows_launch_failure(&flatten_ws(
            "claude-bogus: not found in PATH"
        )));

        // --- Windows: 이번 수정으로 새로 잡혀야 하는 케이스 ---
        // PowerShell: 미존재 명령
        assert!(
            screen_shows_launch_failure(&flatten_ws(
                "claude-bogus : The term 'claude-bogus' is not recognized as the name of a cmdlet, \
                 function, script file, or operable program. Check the spelling of the name, ..."
            )),
            "PowerShell의 미존재 명령 오류를 감지하지 못함"
        );
        // cmd.exe: 미존재 명령
        assert!(
            screen_shows_launch_failure(&flatten_ws(
                "'claude-bogus' is not recognized as an internal or external command, \
                 operable program or batch file."
            )),
            "cmd.exe의 미존재 명령 오류를 감지하지 못함"
        );

        // --- 음성(negative): 정상 기동 화면은 사망으로 오판하지 않아야 함 ---
        // 정상 Claude Code 프롬프트(ready_marker ❯ 포함)
        assert!(!screen_shows_launch_failure(&flatten_ws(
            "Welcome to Claude Code\n\n❯ "
        )));
        // 폴더 신뢰 프롬프트
        assert!(!screen_shows_launch_failure(&flatten_ws(
            "Do you trust the files in this folder?"
        )));
        // 빈 화면
        assert!(!screen_shows_launch_failure(&flatten_ws("")));
    }

    #[test]
    fn fmt_secs_buckets() {
        // < 60: 초만
        assert_eq!(fmt_secs(0), "0s");
        assert_eq!(fmt_secs(59), "59s");
        // 60..3600: 분초
        assert_eq!(fmt_secs(60), "1m0s");
        assert_eq!(fmt_secs(90), "1m30s");
        assert_eq!(fmt_secs(3599), "59m59s");
        // >= 3600: 시분 (초는 표시 안 함 — 의도된 손실)
        assert_eq!(fmt_secs(3600), "1h0m");
        assert_eq!(fmt_secs(5400), "1h30m");
        assert_eq!(fmt_secs(7325), "2h2m"); // 5초 버림
    }
}
