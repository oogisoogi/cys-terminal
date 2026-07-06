//! cysd — CYSJavis 터미널 헤드리스 코어 데몬.
//! UI와 완전 분리: UI가 hang이어도 이 데몬과 소켓 제어 채널은 항상 살아있다 (OOB 회생).
// Windows: 데몬은 콘솔이 없어야 한다. 콘솔 서브시스템으로 두면 GUI(windows_subsystem)가
// cysd.exe 를 띄울 때 Windows가 실제 콘솔을 할당(Win11=Windows Terminal 검은 빈 창)하고,
// 그 상속 콘솔이 ConPTY 유사콘솔 핸드오프를 오염시켜 셸 surface가 즉시 종료된다([surface exited]).
// GUI 앱과 동일하게 릴리스에서 windows subsystem 으로 빌드해 콘솔을 원천 제거한다(디버그는 콘솔 유지).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod analytics;
mod approval;
mod caps;
mod channels;
mod classifier;
mod cost;
mod deadman;
mod events;
mod governance;
mod handlers;
mod hwmon;
mod recall;
mod schedule;
mod severity;
mod state;
mod undo;
mod usage;

use cys::Request;
use handlers::Reply;
use serde_json::json;
use state::Daemon;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

type Stream = Box<dyn AsyncReadWrite>;

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// Claude Code 세션 안에서 spawn된 데몬이 그 세션의 정체성 env를 PTY 자식들에게
/// 물려주면, pane의 claude가 **child-session 모드**(부모 세션 종속)로 동작해 트랜스크립트
/// .jsonl을 영속하지 않는다 — 복원(restore)·recall·사용량 관측(T5)이 전부 깨진다
/// (2026-06-13 실측: 데몬을 `cys ping`으로 claude Bash에서 재기동하자 신규 노드 4종
/// 전부 트랜스크립트 미생성, env에 CLAUDE_CODE_SESSION_ID=부모세션 확인).
/// 데몬은 어떤 환경에서 spawn되든 자식에게 세션 정체성을 누설하면 안 된다 — 기동 즉시 제거.
fn scrub_claude_session_env() {
    const LEAKY: [&str; 5] = [
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SSE_PORT",
    ];
    for k in LEAKY {
        if std::env::var_os(k).is_some() {
            std::env::remove_var(k);
            eprintln!("[cysd] scrubbed leaky claude session env: {k}");
        }
    }
}

#[tokio::main]
async fn main() {
    scrub_claude_session_env();
    // ★무중단 rename-swap 잔해 청소(nsis-hooks.nsh의 짝): 업데이트가 잠긴 파일을 죽이는 대신
    // <이름>.prev*(cysd/cys 고정 체인 + unlock-sweep의 <이름>.prev<rand> — msys-2.0.dll 등 세션이
    // 로드한 runtime 이미지)로 밀어두므로, 새 cysd 기동 시 설치 트리를 재귀 순회하며 이름에
    // ".prev"가 든 파일을 best-effort 삭제한다. lame-duck이 아직 점유 중이면 실패가 정상 —
    // 조용히 스킵하고 다음 기동이 마저 청소한다(fail-open · 세션 보존 우선). 깊이 상한 12.
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            fn sweep_prev(dir: &std::path::Path, depth: u8) {
                if depth == 0 {
                    return;
                }
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        sweep_prev(&p, depth - 1);
                    } else if p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains(".prev"))
                        && std::fs::remove_file(&p).is_ok()
                    {
                        eprintln!("[cysd] stale update leftover removed: {}", p.display());
                    }
                }
            }
            sweep_prev(dir, 12);
        }
    }
    // crash recovery(§7-⑤): 직전 pack-update가 apply 도중 죽어 남긴 orphan 저널을 install(false)
    // **이전에** 자가치유한다(미커밋=rollback / 커밋완료=정리). 순서가 중요 — install(false)가
    // 부분반영 트리 위에서 돌면 안 되므로 반드시 선행한다.
    match cys::pack::recover_pack_journal() {
        Ok(true) => eprintln!("[cysd] pack-update orphan journal recovered (self-heal)"),
        Ok(false) => {}
        Err(e) => eprintln!("[cysd] pack journal recovery skipped: {e}"),
    }
    // 온보딩②: 신규 머신 첫 기동 시 pack 자동 설치 (보존 모드 — 기존 사용자 파일 불가침).
    // launch-agent·디렉티브·acl이 "init-pack을 아는 사람"에게만 동작하는 것을 없앤다.
    match cys::pack::install(false) {
        Ok((written, _)) if written > 0 => eprintln!(
            "[cysd] CYSJavis Pack: {written} file(s) installed at {}",
            cys::pack::pack_dir().display()
        ),
        Ok(_) => {}
        Err(e) => eprintln!("[cysd] pack auto-install skipped: {e}"),
    }
    let socket_path = cys::socket_path();
    let daemon = Daemon::new(socket_path.clone());

    governance::spawn_watchdog(Arc::clone(&daemon));
    schedule::spawn_scheduler(Arc::clone(&daemon));
    usage::spawn_usage_collector(Arc::clone(&daemon));
    usage::spawn_agy_collector(Arc::clone(&daemon));
    // C0: 채널 부팅 재조정(고아 선-kill→새 토큰 재스폰) — 이벤트버스·state 준비 후(§2.1-2).
    // 불사조 복원 프로토콜의 "채널 재조정" 단계. 그 다음 주기 sweep(재배달·타임아웃·재스폰) 등록.
    channels::reconcile(&daemon);
    channels::spawn_channel_sweep(Arc::clone(&daemon));
    // 셧다운 경로: 원장은 메모리 전용이라 데몬이 죽으면 scoped 프로세스를 아무도 회수하지
    // 못한다 — SIGTERM/SIGINT 때 scoped 그룹을 전부 정리한 뒤 종료한다.
    #[cfg(unix)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            shutdown_cleanup(&d, "signal");
            std::process::exit(0);
        });
    }
    // Windows: SIGTERM/SIGINT가 없으므로 콘솔 제어 이벤트로 같은 회수를 건다.
    // Ctrl-C·콘솔 닫힘·로그오프/셧다운(=catchable) 시 scoped 그룹을 정리한다.
    // (taskkill /F는 TerminateProcess라 어떤 핸들러도 못 받음 — 그 경로는 호출측
    //  taskkill /T·원장 정리의 몫. 여기선 unix가 잡던 모든 catchable 종료를 대칭화.)
    #[cfg(windows)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown};
            let (Ok(mut cc), Ok(mut close), Ok(mut shutdown)) =
                (ctrl_c(), ctrl_close(), ctrl_shutdown())
            else {
                return;
            };
            tokio::select! {
                _ = cc.recv() => {},
                _ = close.recv() => {},
                _ = shutdown.recv() => {},
            }
            shutdown_cleanup(&d, "console_event");
            std::process::exit(0);
        });
    }
    daemon.bus.publish(
        "daemon.started",
        "system",
        None,
        json!({"pid": std::process::id(), "socket": socket_path.to_string_lossy()}),
    );

    eprintln!(
        "cysd (CYSJavis terminal daemon) listening on {}",
        socket_path.display()
    );
    accept_loop(daemon, &socket_path).await;
}

/// 종료 직전 회수: 원장의 scoped 그룹을 전부 죽이고, stopping 이벤트 발행 후
/// 소켓 파일을 제거한다. unix·windows 양쪽 종료 핸들러가 공유한다 (크로스플랫폼 대칭).
/// (windows named pipe엔 제거할 파일이 없어 remove_file은 무해한 no-op이 된다.)
fn shutdown_cleanup(daemon: &Arc<Daemon>, reason: &str) {
    let scoped = governance::collect_scoped_for_shutdown(&daemon.ledger.lock().unwrap());
    for (pid, pgid) in scoped {
        governance::kill_group_or_pid(pid, pgid);
    }
    daemon
        .bus
        .publish("daemon.stopping", "system", None, json!({"reason": reason}));
    let _ = std::fs::remove_file(&daemon.socket_path);
}

#[cfg(unix)]
async fn accept_loop(daemon: Arc<Daemon>, socket_path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = socket_path.parent() {
        let _ = std::fs::create_dir_all(dir);
        // 상태 디렉터리는 소유자 전용 — transcripts.db·feed.jsonl·소켓을 같은 UID로 봉인
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    // 동시 기동 TOCTOU 차단: 점검-삭제-바인드를 flock으로 직렬화 — 늦게 뜬 데몬이
    // 살아있는 데몬의 소켓 파일을 unlink해 도달 불가 좀비로 만드는 경로를 막는다.
    // 락 파일은 데몬 수명 동안 보유한다.
    // ★W3: 경합 시 단순 exit(1)이 아니라, 홀더가 hung(무응답 + heartbeat stale)이면 데드맨이
    // 회수·인수한다. 건강한 홀더/구 락파일(pid 미상)은 fail-closed로 exit(무손실·오살상 차단).
    let lock_path = socket_path.with_extension("lock");
    let state_dir = socket_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let _lock_file = acquire_startup_lock(&lock_path, socket_path, &state_dir);

    // ★W3 heartbeat: 승자만 주기적으로 mtime을 갱신한다 → 런타임이 wedge되면 interval 태스크가
    // 진행하지 못해 자연히 stale이 되고, 다음 경합자의 데드맨이 dead로 판정할 수 있다.
    // 기동 창(락 획득~첫 주기 touch)은 claim_lock의 동기 초기 touch가 방어한다.
    {
        let hb = deadman::heartbeat_path(&state_dir);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(deadman::HEARTBEAT_INTERVAL);
            loop {
                tick.tick().await;
                deadman::touch_heartbeat(&hb);
            }
        });
    }
    // Refuse to start if a live daemon already owns the socket (중복 기동 방지 — 거버넌스 철학).
    if socket_path.exists() {
        if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
            eprintln!(
                "error: another cysd is already listening on {}",
                socket_path.display()
            );
            std::process::exit(1);
        }
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = tokio::net::UnixListener::bind(socket_path)
        .unwrap_or_else(|e| panic!("bind {} failed: {e}", socket_path.display()));
    // 소켓 파일은 소유자만 read/write — 인증 없는 제어 채널을 같은 UID로 한정한다.
    // (master·worker·gemini·codex 노드는 모두 오너 UID로 도는 단일 사용자 구조)
    let _ = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600));

    // ★W2 콜드부트 자동 복원: 소켓 바인드·수신 준비가 끝난 '이후'에만 1회 발화한다(자식
    // phoenix가 이 데몬 소켓으로 즉시 RPC할 수 있어야 하므로 바인드 성공이 선행 조건).
    // raw `cys restore`가 아니라 phoenix를 태워 desired_roster·묘비·회로차단기·저널을 경유한다.
    spawn_auto_restore(&state_dir);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // T1-3 발신자 신원: 커널이 보증하는 peer pid (자기신고 from의 검증 토대)
                let caller_pid = peer_pid(&stream);
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(daemon, Box::new(stream) as Stream, caller_pid).await;
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// ★W3 startup lock 획득 — 경합 시 데드맨 에스컬레이션(hung 홀더 회수·인수)까지 수행한다.
/// 성공 시 락파일에 자기 pid 기록 + heartbeat 초기 touch 후 락 핸들 반환(데몬 수명 동안 보유).
/// 락 파일 자체를 못 열면 None(기존 동작 — connect 점검만으로 진행).
/// 회수 실패·건강한 홀더·구 락파일(pid 미상)은 fail-closed로 exit(1)(dedupe 로그).
#[cfg(unix)]
fn acquire_startup_lock(
    lock_path: &std::path::Path,
    socket_path: &std::path::Path,
    state_dir: &std::path::Path,
) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
    {
        Ok(f) => f,
        Err(_) => return None, // 락 파일 생성 실패 — 기존 connect 점검만으로 진행
    };
    let try_flock =
        |f: &std::fs::File| unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 };

    if try_flock(&file) {
        deadman::claim_lock(&mut file, state_dir);
        return Some(file);
    }

    // 경합: 현재 홀더 상태 진단(pid·소켓 응답·heartbeat 신선도) → 판정.
    let holder_pid = deadman::read_holder_pid(lock_path);
    let responded = deadman::probe_holder(socket_path, deadman::PROBE_TIMEOUT);
    let hb_stale = deadman::heartbeat_stale(
        &deadman::heartbeat_path(state_dir),
        deadman::HEARTBEAT_STALE_THRESHOLD,
    );
    match deadman::judge_holder(holder_pid, responded, hb_stale) {
        deadman::HolderVerdict::Dead => {
            // 홀더 hung 확정 → 회수(SIGTERM→SIGKILL, cysd 검증 후) → 락 1회 재획득 시도.
            let pid = holder_pid.expect("Dead 판정은 pid 존재를 함의");
            if deadman::reclaim_from_dead_holder(pid, deadman::RECLAIM_GRACE, deadman::pid_is_cysd)
                && try_flock(&file)
            {
                deadman::claim_lock(&mut file, state_dir);
                eprintln!("[cysd] deadman: reclaimed startup lock from dead holder (pid {pid})");
                return Some(file);
            }
            log_lock_loss(state_dir, lock_path, "dead-holder-reclaim-failed");
            std::process::exit(1);
        }
        deadman::HolderVerdict::Healthy => {
            log_lock_loss(state_dir, lock_path, "healthy-holder");
            std::process::exit(1);
        }
        deadman::HolderVerdict::FailClosed => {
            // 구 락파일(pid 미상) — 오살상 방지 위해 개입하지 않고 exit.
            log_lock_loss(state_dir, lock_path, "unknown-holder-pid");
            std::process::exit(1);
        }
    }
}

/// ★W3 crashloop 로그 dedupe — 동일 사유 연속 패배는 N회당 1줄만(누적 카운트 병기).
/// 상태는 state_dir 파일 기반(프로세스가 매번 새로 뜨므로 in-memory 불가).
#[cfg(unix)]
fn log_lock_loss(state_dir: &std::path::Path, lock_path: &std::path::Path, reason: &str) {
    let state_path = state_dir.join("lockloss.state");
    let prev = std::fs::read_to_string(&state_path).ok();
    let (should_log, count, new_state) =
        deadman::dedupe_loss_log(prev.as_deref(), reason, deadman::LOCK_LOSS_LOG_EVERY_N);
    let _ = std::fs::write(&state_path, new_state);
    if should_log {
        eprintln!(
            "error: another cysd holds the startup lock ({}) — reason={reason}, occurrence #{count}",
            lock_path.display()
        );
    }
}

/// ★W2 콜드부트 자동 복원 판정(순수 함수 — 부수효과 없음, 단위 테스트 가능).
/// opt-out(CYS_NO_AUTORESTORE) 또는 phoenix 미설치면 스폰하지 않는다.
#[derive(Debug, PartialEq)]
enum AutoRestore {
    /// CYS_NO_AUTORESTORE=1 — 사용자가 콜드부트 복원을 껐다.
    OptedOut,
    /// phoenix 스크립트 부재(구 배포·미설치) — 조용히 skip(로그 1줄).
    PhoenixMissing(std::path::PathBuf),
    /// 스폰 대상: `python3 <phoenix> restore --auto`.
    /// ★W1/B3(§5-1): env 에 PHOENIX_CYS(exe 옆 cys 절대경로)·PATH(runtime 선두주입)를 주입한다 —
    /// GUI/데몬 최소 PATH(/usr/bin:/bin:…)에서 phoenix 가 `cys` 를 못 찾아 FileNotFoundError→exit 1
    /// 침묵사하던 라이브 결함(2026-07-06 실증)의 근원 수리. 순수 판정이라 단위 테스트로 env 를 검증한다.
    Ready {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
}

/// ★W1/B3: exe_dir·current_path 를 인자로 받는 순수 함수(부수효과 없음·env 주입까지 단위 테스트 가능).
fn decide_auto_restore(
    pack_dir: &std::path::Path,
    opted_out: bool,
    exe_dir: &std::path::Path,
    current_path: &str,
) -> AutoRestore {
    if opted_out {
        return AutoRestore::OptedOut;
    }
    let phoenix = pack_dir.join("bin").join("javis_phoenix.py");
    if !phoenix.exists() {
        return AutoRestore::PhoenixMissing(phoenix);
    }
    let mut env: Vec<(String, String)> = Vec::new();
    // PHOENIX_CYS: 데몬 exe 옆 동봉 cys 절대경로. 실존할 때만 주입한다(없으면 phoenix 의 which→표준경로
    // 폴백에 맡긴다 — 존재하지 않는 경로를 강제 주입해 재차 FileNotFoundError 를 만들지 않는다).
    let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
    let cys_path = exe_dir.join(cys_name);
    if cys_path.is_file() {
        env.push((
            "PHOENIX_CYS".to_string(),
            cys_path.to_string_lossy().into_owned(),
        ));
    }
    // PATH 선두주입 — pane 자식(state.rs)과 동일 유틸 재사용(중복 구현 금지). 이미 포함이면 None(무주입).
    if let Some(newp) = cys::runtime_prefixed_path(exe_dir, current_path) {
        env.push(("PATH".to_string(), newp));
    }
    AutoRestore::Ready {
        program: "python3".to_string(),
        args: vec![
            phoenix.to_string_lossy().into_owned(),
            "restore".to_string(),
            "--auto".to_string(),
        ],
        env,
    }
}

/// 콜드부트 auto-restore를 detached 스폰한다(env에 CYS_NO_AUTOSTART=1 — 자식 CLI가 라이벌
/// 데몬을 autostart하는 재귀를 차단). 대기 스레드가 자식을 reap해 좀비 잔존을 막는다.
/// ★W1: PHOENIX_CYS·PATH 주입(§5-1 침묵사 근원 수리) · stdout/stderr 를 null 대신 phoenix-restore.log 로
/// 캡처(P0-5 사후 진단 불가 수리) · exit 계약 처리(5·6=재시도 금지, 그 외 비0=60s 후 1회 재시도).
fn spawn_auto_restore(state_dir: &std::path::Path) {
    let opted_out = cys::env_compat("CYS_NO_AUTORESTORE")
        .map(|v| v == "1")
        .unwrap_or(false);
    // exe_dir(데몬 바이너리 디렉터리) — PHOENIX_CYS·PATH 계산 기준.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let current_path = std::env::var("PATH").unwrap_or_default();
    let exe_dir_ref = exe_dir.as_deref().unwrap_or_else(|| std::path::Path::new("."));
    match decide_auto_restore(&cys::pack::pack_dir(), opted_out, exe_dir_ref, &current_path) {
        AutoRestore::OptedOut => {
            eprintln!("[cysd] auto-restore skipped (CYS_NO_AUTORESTORE=1)");
        }
        AutoRestore::PhoenixMissing(p) => {
            eprintln!(
                "[cysd] auto-restore skipped (phoenix not installed: {})",
                p.display()
            );
        }
        AutoRestore::Ready { program, args, env } => {
            let log_path = state_dir.join("phoenix-restore.log");
            std::thread::spawn(move || {
                loop_auto_restore(&program, &args, &env, &log_path);
            });
            eprintln!("[cysd] auto-restore triggered (phoenix restore --auto)");
        }
    }
}

/// ★W1 재시도 지연(codex major test seam): 기본 60000ms. CYS_AUTORESTORE_RETRY_DELAY_MS 로 override —
/// 테스트가 sleep 0 으로 결정론 검증(1차 비0→2차 NOOP·중복 스폰 0, 5/6 무재시도)을 돌리게 한다.
fn autorestore_retry_delay() -> std::time::Duration {
    let ms = std::env::var("CYS_AUTORESTORE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60_000);
    std::time::Duration::from_millis(ms)
}

/// auto-restore 자식을 실행하고 exit 계약에 따라 처리한다. 비0(단 5·6 제외)은 delay 후 정확히 1회 재시도한다
/// — 재시도의 멱등성은 phoenix 의 lease·liveness 재산정에 맡긴다(수동 복원이 이미 끝났으면 재시도는 NOOP·중복 스폰 0).
fn loop_auto_restore(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) {
    let program = program.to_string();
    let args = args.to_vec();
    let env = env.to_vec();
    let log_path = log_path.to_path_buf();
    loop_auto_restore_with(
        |_attempt| run_auto_restore_once(&program, &args, &env, &log_path),
        autorestore_retry_delay(),
    );
}

/// ★재시도 결정 루프(test seam · 순수 로직 — 러너·지연 주입). 반환 = 실행 횟수(테스트 단언용).
/// exit 계약: 0=성공 종료 · 5(BREAKER)/6(CORRUPT·identity)=재시도 금지 · 그 외 비0/None=delay 후 1회 재시도.
fn loop_auto_restore_with<F>(mut run: F, retry_delay: std::time::Duration) -> u32
where
    F: FnMut(u32) -> Option<i32>,
{
    let mut attempt = 0u32;
    loop {
        let code = run(attempt);
        attempt += 1;
        match code {
            Some(0) => {
                eprintln!("[cysd] auto-restore finished (exit=0)");
                return attempt;
            }
            Some(5) => {
                eprintln!("[cysd] auto-restore BREAKER_OPEN (exit=5) — 재시도 금지(크래시루프 정지·사람 승인 필요)");
                return attempt;
            }
            Some(6) => {
                eprintln!("[cysd] auto-restore CORRUPT/identity (exit=6) — 재시도 금지(사람 개입 필요)");
                return attempt;
            }
            other => {
                if attempt >= 2 {
                    eprintln!(
                        "[cysd] auto-restore finished (exit={other:?}) — 재시도 소진(1회). phoenix-restore.log 참조"
                    );
                    return attempt;
                }
                eprintln!(
                    "[cysd] auto-restore non-zero (exit={other:?}) — {}ms 후 1회 재시도 (lease/liveness 재산정에 위임)",
                    retry_delay.as_millis()
                );
                std::thread::sleep(retry_delay);
            }
        }
    }
}

/// ★W1 로그 대상 결정(codex major): phoenix-restore.log(primary) → temp_dir 폴백 → 둘 다 실패면 inherit.
/// null 로 떨어뜨리지 않는다 — 파일시스템/경로 실패가 진단 대상인데 그 순간 증거를 소실시키는 게 정확히 W1 관측성
/// 위반이므로, 최악이라도 자식 stdio 를 데몬 stderr 로 inherit 해 증거를 보존한다.
fn open_restore_log(log_path: &std::path::Path) -> Option<std::fs::File> {
    use std::io::Write;
    let open = |p: &std::path::Path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    };
    let f = open(log_path).or_else(|| {
        let tmp = std::env::temp_dir().join("cys-phoenix-restore.log");
        let alt = open(&tmp);
        if alt.is_some() {
            eprintln!(
                "[cysd] auto-restore primary log 실패({}) — temp 폴백 {}",
                log_path.display(),
                tmp.display()
            );
        }
        alt
    });
    if let Some(mut f) = f {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(
            f,
            "\n===== phoenix auto-restore @ epoch={epoch} (pid cysd={}) =====",
            std::process::id()
        );
        Some(f)
    } else {
        eprintln!(
            "[cysd] auto-restore log(primary+temp) 모두 open 실패 — 자식 stdio 를 데몬 stderr 로 inherit(증거 소실 방지)"
        );
        None
    }
}

/// 자식 1회 실행 — stdout/stderr 를 phoenix-restore.log(폴백 포함)에 append. exit code 반환(None=스폰 실패).
fn run_auto_restore_once(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) -> Option<i32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).env("CYS_NO_AUTOSTART", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::null());
    match open_restore_log(log_path) {
        Some(f) => {
            // stderr 는 clone 으로 같은 파일에 합류. clone 실패 시 null 이 아니라 inherit(증거 보존).
            match f.try_clone() {
                Ok(errf) => {
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::from(errf));
                }
                Err(e) => {
                    eprintln!("[cysd] auto-restore log stderr clone 실패({e}) — stderr inherit 폴백(null 금지)");
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::inherit());
                }
            }
        }
        None => {
            cmd.stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
        }
    }
    match cmd.status() {
        Ok(s) => Some(s.code().unwrap_or(-1)),
        Err(e) => {
            eprintln!("[cysd] auto-restore spawn failed: {e}");
            None
        }
    }
}

/// T1-3: UDS peer pid 조회 — macOS LOCAL_PEERPID, Linux SO_PEERCRED.
#[cfg(unix)]
fn peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        const SOL_LOCAL: libc::c_int = 0;
        const LOCAL_PEERPID: libc::c_int = 0x002;
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && pid > 0 {
            return Some(pid as u32);
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && cred.pid > 0 {
            return Some(cred.pid as u32);
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = fd;
        None
    }
}

/// Windows accept_loop가 `connect()` 오류 후 같은 broken 인스턴스에 곧장 재시도하다
/// 100% CPU로 spin하지 않도록 두는 backoff. mio `ConnectNamedPipe`는 정상 대기는
/// WouldBlock(→tokio가 await)으로, 진짜 OS 오류는 즉시 Err로 반환하므로(connecting 플래그도
/// 즉시 해제 → self-throttle 없음), 오류 분기는 ①로그 ②인스턴스 재생성 ③이 짧은 sleep로
/// 회생해야 Unix arm(accept err→다음 await)·tokio 표준 루프(?로 전파)와 대칭이 된다.
/// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, 정책 값을 모듈 최상위로 빼
///  비-Windows 테스트가 'spin 방지=non-zero backoff' 불변을 박제하게 한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_ACCEPT_ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// owner-only DACL의 SDDL: D:P=보호된(상속차단) DACL, FA=full access를
/// OW(OWNER_RIGHTS=creator)·SY(SYSTEM)·BA(BUILTIN\Administrators)에게만 부여.
/// WD(Everyone)·AU(Authenticated Users) 같은 광역 SID가 없어 같은 머신의 임의 사용자를 배제한다.
/// (cfg(windows) 밖에서도 회귀 테스트가 참조할 수 있게 모듈 최상위 const로 둔다.
///  비-Windows 비-test 빌드에서는 실사용처가 없으므로 dead_code를 명시 허용한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_SDDL_OWNER_ONLY: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)";

/// Windows named pipe 보안 디스크립터: 소유자(creator)·SYSTEM·Administrators에게만
/// full access를 허용하는 owner-only DACL(PIPE_SDDL_OWNER_ONLY)을 SECURITY_ATTRIBUTES에 싣는다.
/// UDS 0o700 dir + 0o600 소켓의 단일-UID 봉인과 대칭 — 같은 머신의 임의 로컬 사용자가
/// 인증 없는 제어 채널(send_text·send_key·ledger.kill)에 접근하는 권한 우회를 차단한다.
/// 반환된 PSECURITY_DESCRIPTOR는 LocalFree로 해제해야 하므로, RAII 가드로 SA와 함께 수명을 묶는다.
#[cfg(windows)]
struct OwnerOnlySecurity {
    sa: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    psd: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
}

#[cfg(windows)]
impl OwnerOnlySecurity {
    fn new() -> Option<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        // 와이드 널종단 SDDL 문자열
        let sddl: Vec<u16> = PIPE_SDDL_OWNER_ONLY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut psd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || psd.is_null() {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd,
            bInheritHandle: 0,
        };
        Some(Self { sa, psd })
    }

    /// create_with_security_attributes_raw에 넘길 *mut SECURITY_ATTRIBUTES (가드 수명 동안 유효).
    fn as_ptr(&self) -> *mut std::ffi::c_void {
        &self.sa as *const _ as *mut std::ffi::c_void
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurity {
    fn drop(&mut self) {
        // ConvertString…가 LocalAlloc로 잡은 SD를 해제 (가드가 데몬 수명 동안 살아있으므로
        // 실무상 프로세스 종료 시점에만 호출되나, 누수 방지를 위해 명시 해제).
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.psd as *mut _);
        }
    }
}

#[cfg(windows)]
async fn accept_loop(daemon: Arc<Daemon>, socket_path: &std::path::Path) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let pipe_name = socket_path.to_string_lossy().into_owned();
    // owner-only DACL 보안 디스크립터 — 데몬 수명 동안 보유해 모든 파이프 인스턴스에 적용.
    // SDDL 변환이 실패하면(이론상 거의 없음) None → null 포인터로 폴백하되 경고를 남긴다.
    let security = OwnerOnlySecurity::new();
    if security.is_none() {
        eprintln!(
            "warning: failed to build owner-only pipe security descriptor; \
             falling back to default DACL (any local user may connect)"
        );
    }
    let sa_ptr = security
        .as_ref()
        .map(|s| s.as_ptr())
        .unwrap_or(std::ptr::null_mut());
    // Safety: sa_ptr는 null이거나 `security` 가드가 소유한 유효한 SECURITY_ATTRIBUTES를 가리키며,
    // 그 가드는 이 함수(=데몬 수명) 끝까지 살아있어 모든 파이프 생성보다 오래 산다.
    let mut server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .create_with_security_attributes_raw(&pipe_name, sa_ptr)
    }
    .unwrap_or_else(|e| panic!("create pipe {pipe_name} failed: {e}"));
    loop {
        match server.connect().await {
            Ok(()) => {
                let connected = server;
                server = unsafe {
                    ServerOptions::new().create_with_security_attributes_raw(&pipe_name, sa_ptr)
                }
                .expect("recreate pipe failed");
                // 발신자 신원: 커널이 보증하는 named pipe 클라이언트 pid (UDS peer_pid와 대칭).
                // 박는 이유: claim_role·surface.close·status.set 등은 발신 신원이 None이면 무조건
                // 거부하므로, 미구현(None)이면 Windows에서 자기 surface 자가-claim('cys claim-role
                // master' 등 launch-agent 밖 직접 기동 노드)이 영영 막힌다. boxing 전에 조회한다.
                let caller_pid = peer_pid(&connected);
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(daemon, Box::new(connected) as Stream, caller_pid).await;
                });
            }
            Err(e) => {
                // connect()가 즉시 Err를 반환하면(broken 핸들 등) 같은 인스턴스에 곧장
                // 재시도해도 같은 Err가 무한 반복돼 100% CPU spin이 된다(mio가 connecting
                // 플래그를 즉시 해제해 self-throttle도 없음). Unix arm(accept err→다음 await)·
                // tokio 표준 루프(?로 전파)와 대칭이 되도록: ①로그 ②인스턴스 재생성 ③짧은 backoff.
                eprintln!("accept error: {e}");
                server = unsafe {
                    ServerOptions::new().create_with_security_attributes_raw(&pipe_name, sa_ptr)
                }
                .expect("recreate pipe failed");
                tokio::time::sleep(PIPE_ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

/// Windows named pipe 클라이언트 pid 조회 — UDS peer_pid(macOS LOCAL_PEERPID/Linux SO_PEERCRED)와
/// 대칭. GetNamedPipeClientProcessId는 서버 측 핸들에서 연결된 클라이언트 프로세스 id를 돌려준다.
/// 실패(0 반환 또는 pid 0)면 None — 호출부는 UDS와 동일하게 익명 발신으로 처리한다.
#[cfg(windows)]
fn peer_pid(pipe: &tokio::net::windows::named_pipe::NamedPipeServer) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    let mut pid: u32 = 0;
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
            pipe.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
            &mut pid,
        )
    };
    if ok != 0 && pid != 0 {
        Some(pid)
    } else {
        None
    }
}

/// 개행 없는 무한 스트림이 데몬 메모리를 잠식하지 못하게 줄 길이 상한을 둔 line reader.
async fn next_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    cap: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&buf).into_owned())
            });
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            r.consume(pos + 1);
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let n = available.len();
        buf.extend_from_slice(available);
        r.consume(n);
        if buf.len() > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request line too long",
            ));
        }
    }
}

const MAX_REQUEST_LINE: usize = 10 * 1024 * 1024; // 지침 주입(수백 KB)에 충분한 10MB

async fn handle_connection(daemon: Arc<Daemon>, stream: Stream, caller_pid: Option<u32>) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    while let Ok(Some(line)) = next_line_capped(&mut reader, MAX_REQUEST_LINE).await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp =
                    cys::err_response(&serde_json::Value::Null, "parse_error", &e.to_string());
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
                continue;
            }
        };

        match handlers::dispatch(&daemon, req, caller_pid) {
            Reply::Single(resp) => {
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::EventStream {
                ack,
                after_seq,
                names,
                categories,
            } => {
                run_event_stream(&daemon, &mut write_half, ack, after_seq, names, categories).await;
                return;
            }
            Reply::Attach { ack, surface_id } => {
                run_attach(&daemon, &mut write_half, ack, surface_id).await;
                return;
            }
            Reply::FeedWait {
                id,
                request_id,
                rx,
                timeout_secs,
            } => {
                // T4-15: pause 중에는 카운트다운 동결 — kill-switch가 대기 중인 워커들을
                // timeout-deny로 우수수 떨어뜨리지 않는다 (resume 후 잔여 시간부터 재개).
                let mut rx = rx;
                let mut remaining = timeout_secs;
                let outcome: Option<String> = loop {
                    tokio::select! {
                        r = &mut rx => break r.ok(),
                        // 클라이언트 연결 끊김 감지: 대기 중에는 응답을 아직 쓰기 전이라
                        // events.stream·attach의 write 실패 안전망이 닿지 않는다. read half를
                        // 함께 감시해, 워커가 응답 전에 끊으면(EOF/에러) 즉시 정리하고 빠져나간다.
                        // 없으면 끊긴 워커의 waiter·연결 태스크가 timeout(최대 3600초)까지,
                        // pause 중에는 remaining이 동결돼 resume까지 무기한 잔존한다.
                        read = reader.fill_buf() => match read {
                            // EOF(빈 슬라이스) = 끊김. 비어있지 않은 바이트는 대기 중 추가 전송으로
                            // 프로토콜 위반이라 연결을 신뢰할 수 없다 — 셋 다 끊김으로 정리.
                            Ok([]) | Ok([_, ..]) | Err(_) => break None,
                        },
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                            if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
                                if remaining <= 1 { break None; }
                                remaining -= 1;
                            }
                        }
                    }
                };
                let resp = match outcome {
                    Some(decision) => cys::ok_response(
                        &id,
                        json!({"request_id": request_id, "status": "resolved", "decision": decision}),
                    ),
                    None => {
                        // Timeout or dropped: mark the item and tell the caller.
                        daemon.feed_waiters.lock().unwrap().remove(&request_id);
                        let snapshot = {
                            let mut items = daemon.feed_items.lock().unwrap();
                            items
                                .iter_mut()
                                .find(|i| i.request_id == request_id)
                                .filter(|i| i.status == "pending")
                                .map(|item| {
                                    item.status = "timeout".into();
                                    item.resolved_at = Some(crate::state::now_epoch());
                                    item.clone()
                                })
                        };
                        if let Some(s) = &snapshot {
                            daemon.persist_feed_item(s);
                            daemon.bus.publish(
                                "feed.item.timeout",
                                "feed",
                                None,
                                json!({"request_id": request_id}),
                            );
                            cys::ok_response(
                                &id,
                                json!({"request_id": request_id, "status": "timeout", "decision": null}),
                            )
                        } else {
                            // 동시 feed.reply가 이미 종결 — 승인 결정을 삼키고 timeout으로
                            // 오보하는 대신 실제 결정을 돌려준다 (모순 이벤트도 미발행)
                            let decision = daemon
                                .feed_items
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|i| i.request_id == request_id)
                                .and_then(|i| i.decision.clone());
                            match decision {
                                Some(d) => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "resolved", "decision": d}),
                                ),
                                None => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "timeout", "decision": null}),
                                ),
                            }
                        }
                    }
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::WaitFor {
                id,
                surface_id,
                pattern,
                timeout_secs,
                since_line,
            } => {
                // T3-14 완료 대기: 데몬 내부 폴링(토큰 비용 0) — plain-line 마커 규약 전제.
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(timeout_secs);
                let mut cursor = since_line;
                let resp = loop {
                    let Some(surface) = daemon.get_surface(surface_id) else {
                        break cys::err_response(
                            &id,
                            "not_found",
                            &format!("surface {surface_id} closed"),
                        );
                    };
                    let (lines, start) = {
                        // ★레이스 차단: scrollback 락을 먼저 잡고 그 안에서 line_count를 읽는다
                        // (writer가 push·fetch_add를 같은 락 아래 수행 — total/sb.len 일관 관측).
                        let sb = surface.scrollback.lock().unwrap_or_else(|e| e.into_inner());
                        let total = surface
                            .line_count
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let oldest = total.saturating_sub(sb.len() as u64);
                        let start = cursor.max(oldest);
                        let skip = (start - oldest) as usize;
                        let lines: Vec<String> = sb.iter().skip(skip).cloned().collect();
                        (lines, start)
                    };
                    let mut matched = None;
                    for (i, line) in lines.iter().enumerate() {
                        if pattern.is_match(line) {
                            matched = Some((start + i as u64, line.clone()));
                            break;
                        }
                    }
                    cursor = start + lines.len() as u64;
                    if let Some((line_no, line)) = matched {
                        break cys::ok_response(
                            &id,
                            json!({"matched": true, "line": line, "line_no": line_no,
                                   "next_cursor": line_no + 1}),
                        );
                    }
                    if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "surface_exited",
                                   "next_cursor": cursor}),
                        );
                    }
                    if std::time::Instant::now() >= deadline {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "timeout",
                                   "next_cursor": cursor}),
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// T1-6: cys↔cysd ABI producer 자기검증 경계. 응답 `Value`를 `cys::wire::frame_response`로
/// 통과시켜 round-trip 동일성(선언==실제 직렬화)을 검증하고 `_flen`/`_pv`를 additive하게
/// 부착한다(top-level `ok`/`result`는 보존 → 구 디코더 호환). 위반은 T1-3 `Severity`로
/// 사상해 fail-loud 기록한다(Drift/LenMismatch=Critical 격리, VersionSkew=Recoverable).
/// 검증 실패가 응답 자체를 삼켜 클라이언트를 무기한 대기시키지 않도록, 기록 후 legacy 직렬화로
/// 폴백해 한 줄은 항상 내보낸다(가용성 보존 — 격리 판정은 Severity 로그가 담당).
fn abi_severity(e: &cys::wire::AbiError) -> severity::Severity {
    match e {
        cys::wire::AbiError::Drift | cys::wire::AbiError::LenMismatch => severity::Severity::Critical,
        cys::wire::AbiError::VersionSkew { .. } => severity::Severity::Recoverable,
    }
}

async fn write_line<W: AsyncWrite + Unpin>(
    w: &mut W,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    // T4-5A(==T5-6 strand-3, ONE guard): 단일 RPC 응답 바이트 상한. cap 초과 시 fail-loud
    // 트렁케이트 sentinel로 치환(컨텍스트/메모리 폭주 차단). 직교 가드 — watchdog와 별개 책임.
    let capped = cys::wire::cap_response(value);
    let value: &serde_json::Value = capped.as_ref().unwrap_or(value);
    let line = match cys::wire::frame_response(value) {
        Ok(framed) => framed,
        Err(e) => {
            let sev = abi_severity(&e);
            eprintln!(
                "[cysd] ABI producer self-verify {} ({:?}) — falling back to legacy serialization",
                sev.as_str(),
                e
            );
            let mut body = serde_json::to_string(value).unwrap_or_default();
            body.push('\n');
            body
        }
    };
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

/// Push channel: replay missed events, then forward live events until the client disconnects.
async fn run_event_stream<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    after_seq: Option<u64>,
    names: Vec<String>,
    categories: Vec<String>,
) {
    // Subscribe BEFORE replay so no events fall into the gap.
    let mut rx = daemon.bus.subscribe();
    // dispatch 시점이 아닌 구독 직후의 최신 seq로 갱신 — 클라이언트 커서 시드 정확화
    let mut ack = ack;
    let live_latest = daemon.bus.latest_seq();
    ack["latest_seq"] = json!(live_latest);
    // (1)-sync: resume 블록도 구독 직후 최신값으로 동기 — dispatch 시점 값과 어긋나지 않게
    if ack.get("resume").is_some() {
        ack["resume"]["latest_seq"] = json!(live_latest);
        ack["resume"]["next_seq"] = json!(live_latest + 1);
    }
    if write_line(w, &ack).await.is_err() {
        return;
    }
    let mut last_seq = after_seq.unwrap_or(0);
    if let Some(after) = after_seq {
        // 갭 신호: 커서 이후 일부 이벤트가 ring에서 밀려나 재생 불가하면 무음 유실 대신 알린다
        let (oldest, latest) = daemon.bus.replay_bounds();
        let gap_until = oldest.map(|o| o.saturating_sub(1)).unwrap_or(latest);
        if gap_until > after {
            let warn = json!({"type": "error", "ok": false,
                "error": {"code": "replay_gap",
                    "message": format!("events {}..={} no longer available (ring evicted or daemon restarted)", after + 1, gap_until)}});
            if write_line(w, &warn).await.is_err() {
                return;
            }
        }
        for event in daemon.bus.replay_after(after) {
            last_seq = event["seq"].as_u64().unwrap_or(last_seq);
            if events::event_matches(&event, &names, &categories)
                && write_line(w, &event).await.is_err()
            {
                return;
            }
        }
    }
    // (2b) live 루프: 15s heartbeat 타이머와 함께 select! — 이벤트 무발생 구간에서도
    // half-open 소켓을 조기 감지·재연결 유도. 패턴은 run_attach(아래)의 select! 동일.
    let mut hb = tokio::time::interval(std::time::Duration::from_secs(15));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    hb.tick().await; // 첫 tick은 즉시 발화 — 소비해 15s 후부터 heartbeat
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(event) => {
                    let seq = event["seq"].as_u64().unwrap_or(0);
                    if seq <= last_seq {
                        continue; // already replayed
                    }
                    last_seq = seq; // 중복 차단 커서 전진(원본 누락 — 의도 명확화, 동작 동일)
                    if events::event_matches(&event, &names, &categories)
                        && write_line(w, &event).await.is_err()
                    {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let warn = json!({"type": "error", "ok": false,
                        "error": {"code": "slow_consumer", "message": format!("dropped {n} events")}});
                    let _ = write_line(w, &warn).await;
                    return; // (2a) 종료해 클라이언트가 last_seq부터 재replay로 갭을 메우게 강제
                }
                Err(_) => return,
            },
            _ = hb.tick() => {
                let beat = json!({"type": "heartbeat", "latest_seq": daemon.bus.latest_seq()});
                if write_line(w, &beat).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Raw PTY output mirror: ack line (JSON), then raw bytes as they arrive.
async fn run_attach<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    surface_id: u64,
) {
    let Some(surface) = daemon.get_surface(surface_id) else {
        // dispatch 검사와 재조회 사이에 surface가 닫힌 경우 — 무응답 종료 대신 에러를 알린다
        let err = json!({"type": "ack", "ok": false,
            "error": {"code": "not_found", "message": format!("surface {surface_id} closed")}});
        let _ = write_line(w, &err).await;
        return;
    };
    // parser 락 아래에서 구독+스냅샷 — 그 사이 도착한 청크가 스냅샷과 live 양쪽에
    // 중복 배달되는 창을 닫는다 (reader 스레드는 parser 락에서 직렬화됨)
    let (mut rx, snapshot) = {
        let parser = surface.parser.lock().unwrap_or_else(|e| e.into_inner());
        let rx = surface.out_tx.subscribe();
        (rx, parser.screen().contents_formatted())
    };
    if write_line(w, &ack).await.is_err() {
        return;
    }
    // Send a formatted (color/cursor-accurate) redraw of the current screen first.
    if !snapshot.is_empty() && w.write_all(&snapshot).await.is_err() {
        return;
    }
    loop {
        // out_tx Sender는 Surface 구조체가 소유라 자력 종료(셸 exit) 후에도 채널이 닫히지
        // 않는다 — exited 플래그를 주기 점검해 스트림을 끝내야 클라이언트가 EOF를 받는다.
        tokio::select! {
            r = rx.recv() => match r {
                Ok(chunk) => {
                    if w.write_all(&chunk).await.is_err() || w.flush().await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod env_scrub_tests {
    /// 회귀 박제: claude 세션 안에서 spawn된 데몬이 세션 정체성 env를 보존하면 PTY 자식
    /// claude가 child-session으로 강등돼 트랜스크립트 미영속(복원·recall·T5 전부 파괴).
    /// scrub은 누설 변수만 제거하고 무관 변수는 보존해야 한다.
    #[test]
    fn scrub_removes_leaky_session_vars_only() {
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent-session");
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CYS_SCRUB_TEST_KEEP", "yes"); // 무관 변수 — 보존 확인용
        super::scrub_claude_session_env();
        assert!(std::env::var_os("CLAUDE_CODE_SESSION_ID").is_none());
        assert!(std::env::var_os("CLAUDE_CODE_CHILD_SESSION").is_none());
        assert!(std::env::var_os("CLAUDECODE").is_none());
        assert_eq!(
            std::env::var("CYS_SCRUB_TEST_KEEP").as_deref(),
            Ok("yes"),
            "무관 env까지 지우면 안 된다"
        );
        std::env::remove_var("CYS_SCRUB_TEST_KEEP");
    }
}

#[cfg(test)]
mod abi_severity_tests {
    use crate::severity::Severity;

    /// T1-6: AbiError → T1-3 Severity 사상이 §4.2 계약과 일치하는지 박제.
    /// Drift/LenMismatch=Critical(격리), VersionSkew=Recoverable(graceful).
    #[test]
    fn abi_error_to_severity() {
        assert_eq!(super::abi_severity(&cys::wire::AbiError::Drift), Severity::Critical);
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::LenMismatch),
            Severity::Critical
        );
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::VersionSkew {
                peer_pv: 2,
                local_pv: cys::wire::PROTO_PV
            }),
            Severity::Recoverable
        );
        // 격리 술어와의 정합: Critical만 격리, Recoverable은 재시도.
        assert!(super::abi_severity(&cys::wire::AbiError::Drift).is_critical());
        assert!(!super::abi_severity(&cys::wire::AbiError::VersionSkew {
            peer_pv: 2,
            local_pv: cys::wire::PROTO_PV
        })
        .is_critical());
    }
}

#[cfg(test)]
mod attach_race_tests {
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// ★회귀 박제 (state.rs reader thread ↔ main.rs run_attach 불변식):
    /// run_attach는 parser 락 아래에서 `out_tx.subscribe()`+화면 스냅샷을 원자적으로 뜬다
    /// (main.rs:538-542). 그 불변식이 성립하려면 reader 스레드도 `parser.process(chunk)`와
    /// `out_tx.send(chunk)`를 같은 parser 락 임계영역에 묶어야 한다. 둘이 분리되면
    /// (과거 버그) 다음 인터리빙이 같은 청크를 스냅샷·live 양쪽에 중복 배달한다:
    ///   ① reader: process(C) 후 락 해제
    ///   ② attach: 락 획득→subscribe(rx)→스냅샷(C 반영됨)→락 해제
    ///   ③ reader: out_tx.send(C) → ②의 rx가 C를 live로 수신  ⇒ C가 스냅샷+live 중복
    ///
    /// 이 테스트는 run_attach가 하는 일(락 아래 subscribe+스냅샷)을 그대로 모사하는 관측자를
    /// 실제 Surface reader 스레드와 동시에 돌려, "스냅샷 시점에 파서에 이미 반영된 마지막
    /// 청크가 그 직후 새 rx로 live 도착하는" 중복 창이 닫혔는지 다회 검증한다. 버그(분리)면
    /// 충분한 반복에서 중복이 잡히고, 수정(결합)이면 불변식이 무조건 성립해 0건이다.
    ///
    /// 핵심 신호: parser 락을 쥔 채 화면에 반영된 출력 바이트 수(=process가 본 누적 바이트)와
    /// 같은 락 구간에서 subscribe한 rx로 이후 도착하는 바이트가 겹치면(겹친 청크 존재) 중복.
    /// 마커를 청크 단위로 유일하게 만들어 "스냅샷에 보였는데 live로도 온" 마커를 직접 센다.
    #[test]
    fn process_and_send_are_atomic_under_parser_lock_no_dup_delivery() {
        // 멀티스레드 런타임 불필요 — 동기 스레드만 사용. PTY reader는 create_surface가
        // 내부에서 std::thread로 띄운다.
        let tmp = std::env::temp_dir().join(format!(
            "cys-attach-race-{}-{}.sock",
            std::process::id(),
            now_nanos()
        ));
        let daemon = Daemon::new(tmp.clone());

        // 출력 스트림: 각 라인은 유일 토큰 "MK<seq>E". reader 스레드가 끊임없이 청크
        // 경계를 만들도록 긴 루프로 연속 출력하며, 32라인마다 짧은 양보(usleep 미사용 —
        // 셸 내장만)로 reader/observer가 process↔send 경계를 다수 통과하게 한다.
        const N: usize = 6000;
        let script = format!(
            "i=0; while [ $i -lt {N} ]; do printf 'MK%dE\\n' $i; i=$((i+1)); done; sleep 3"
        );
        let surface = daemon
            .create_surface(None, Some(script), None, None, 35, 120)
            .expect("create_surface");

        // 다수 관측자 스레드: run_attach의 '락-아래 subscribe+스냅샷'을 그대로 모사하며
        // process↔send 분리 시 열리는 중복 창(스냅샷에 이미 보인 마커가 새 rx로 live 도착)을
        // 동시 다발로 두드린다. 여러 스레드가 경합해야 좁은 창에 안정적으로 착지한다.
        const OBSERVERS: usize = 6;
        let mut handles = Vec::new();
        for _ in 0..OBSERVERS {
            let surf = Arc::clone(&surface);
            handles.push(std::thread::spawn(move || {
                let mut dup_incidents: Vec<usize> = Vec::new();
                loop {
                    if surf.exited.load(Ordering::Relaxed) {
                        break;
                    }
                    // ── run_attach와 동일: parser 락 아래 subscribe + 스냅샷 ──
                    let (mut rx, snapshot_markers) = {
                        let parser = surf.parser.lock().unwrap_or_else(|e| e.into_inner());
                        let rx = surf.out_tx.subscribe();
                        let snap = parser.screen().contents();
                        (rx, parse_markers(snap.as_bytes()))
                    };
                    // 스냅샷에 마지막으로 보인(=파서에 이미 반영된) 마커. 이 마커는
                    // 결합(수정) 시 '이미 send 완료'라 새 rx로는 절대 오면 안 된다.
                    let Some(&last_in_snapshot) = snapshot_markers.iter().max() else {
                        continue;
                    };
                    // 새 rx를 잠깐 비워 live 마커를 수집 (non-blocking try_recv 폴링).
                    let mut live: Vec<usize> = Vec::new();
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_micros(500);
                    while std::time::Instant::now() < deadline {
                        match rx.try_recv() {
                            Ok(bytes) => live.extend(parse_markers(&bytes)),
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                                std::thread::yield_now()
                            }
                            Err(_) => break,
                        }
                    }
                    // 중복 판정: 스냅샷에 보였던(≤last_in_snapshot) 마커가 live로도 도착하면
                    // 그 청크가 스냅샷·live 양쪽에 배달된 것 — run_attach 주석이 막겠다던 케이스.
                    // (수정본은 process↔send가 원자적이라 새 rx에는 항상 >last_in_snapshot만 온다.)
                    for m in &live {
                        if *m <= last_in_snapshot {
                            dup_incidents.push(*m);
                        }
                    }
                }
                dup_incidents
            }));
        }

        let mut dup_incidents: Vec<usize> = Vec::new();
        for h in handles {
            dup_incidents.extend(h.join().expect("observer thread"));
        }

        // 정리: surface 종료 유도 (자력 종료 전에 kill — 좀비 방지)
        if let Ok(mut child) = surface.child.lock() {
            let _ = child.kill();
        }
        let _ = std::fs::remove_file(&tmp);

        assert!(
            dup_incidents.is_empty(),
            "process↔send가 parser 락에서 분리되어 청크 중복 배달 발생: {} 건 (예: {:?}). \
             reader 스레드는 process(chunk)와 out_tx.send(chunk)를 같은 parser 락 \
             임계영역에 묶어야 한다.",
            dup_incidents.len(),
            &dup_incidents[..dup_incidents.len().min(8)]
        );
    }

    /// "MK<n>E" 토큰을 바이트 스트림에서 추출 (청크/스냅샷 공통 파서).
    fn parse_markers(bytes: &[u8]) -> Vec<usize> {
        let s = String::from_utf8_lossy(bytes);
        let mut out = Vec::new();
        let mut rest = s.as_ref();
        while let Some(p) = rest.find("MK") {
            rest = &rest[p + 2..];
            if let Some(e) = rest.find('E') {
                if let Ok(n) = rest[..e].parse::<usize>() {
                    out.push(n);
                }
                rest = &rest[e + 1..];
            } else {
                break;
            }
        }
        out
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod feed_wait_disconnect_tests {
    use super::{handle_connection, Stream};
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// ★회귀 박제 (FeedWait 대기 중 클라이언트 끊김 + pause 동결):
    /// feed.push --wait의 대기 루프(main.rs)는 ① oneshot rx(=feed.reply) ② 1초 sleep ③ read
    /// half(끊김 감지) 세 가지를 select! 한다. ③이 없으면 워커가 응답 전에 연결을 끊어도
    /// 연결 태스크와 feed_waiters 엔트리가 timeout(최대 3600초)까지 살아남고, 데몬이 pause되면
    /// remaining이 영영 감소하지 않아(if !paused) timeout 분기에 절대 도달하지 못해 resume까지
    /// 무기한 잔존한다. 끊긴 워커가 pause 전후로 반복되면 연결 태스크·oneshot 채널이 단조 누적.
    ///
    /// 이 테스트는 ① feed.push --wait를 보내 waiter를 등록시키고 ② 데몬을 pause한 뒤
    /// ③ 클라이언트를 끊어, 연결 태스크가 (a) 유한 시간 내 종료하고 (b) feed_waiters 엔트리를
    /// 정리하는지 검증한다. 버그(③ 부재)면 pause 동결로 태스크가 영영 살아 timeout이 터지고
    /// waiter도 남는다. 수정(③ 존재)이면 끊김을 감지해 즉시 정리·종료한다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feed_wait_releases_waiter_when_client_disconnects_during_pause() {
        // ★상태 격리: state_dir = socket의 부모 디렉터리이고 거기에 feed.jsonl이 영속된다.
        // 소켓을 고유 하위 디렉터리에 두지 않으면 temp_dir/feed.jsonl을 다른 실행과 공유해
        // 직전 실행이 남긴 같은 request_id가 replay되어 'duplicate request_id'로 오염된다.
        let dir = std::env::temp_dir().join(format!(
            "cys-feedwait-disc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("cysd.sock");
        let daemon = Daemon::new(tmp.clone());

        // 인메모리 양방향 스트림: server는 handle_connection이, client는 테스트가 보유.
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection(Arc::clone(&daemon), server, None));

        // feed.push --wait — timeout_secs는 길게 줘서 끊김이 아닌 timeout으로 빠지는 오판을 배제.
        let mut client = client;
        let req = serde_json::json!({
            "id": "1",
            "method": "feed.push",
            "params": {
                "request_id": "disc-test-1",
                "kind": "approval",
                "title": "t",
                "body": "b",
                "wait": true,
                "timeout_secs": 3600
            }
        });
        let mut line = serde_json::to_vec(&req).unwrap();
        line.push(b'\n');
        client.write_all(&line).await.unwrap();
        client.flush().await.unwrap();

        // waiter 등록 대기 (FeedWait 진입 확인).
        let registered = wait_until(Duration::from_secs(5), || {
            daemon.feed_waiters.lock().unwrap().contains_key("disc-test-1")
        })
        .await;
        assert!(registered, "feed.push --wait가 waiter를 등록하지 못함");

        // 데몬 pause — 이 상태에서 timeout 카운트다운은 동결된다.
        daemon.paused.store(true, Ordering::Relaxed);

        // 클라이언트 끊김 (워커 프로세스 kill 모사).
        drop(client);

        // 수정본: 끊김을 감지해 유한 시간 내 연결 태스크 종료 + waiter 정리.
        // 버그: pause 동결로 영영 살아 timeout이 터진다.
        let finished = tokio::time::timeout(Duration::from_secs(10), conn).await;
        assert!(
            finished.is_ok(),
            "FeedWait 대기 태스크가 클라이언트 끊김을 감지하지 못해 종료하지 않음 \
             (pause 중 remaining 동결 → timeout 분기 영구 미도달)"
        );

        let waiter_cleared = daemon
            .feed_waiters
            .lock()
            .unwrap()
            .get("disc-test-1")
            .is_none();
        assert!(
            waiter_cleared,
            "끊김 후 feed_waiters['disc-test-1'] 엔트리가 정리되지 않고 잔존"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn wait_until<F: FnMut() -> bool>(limit: Duration, mut cond: F) -> bool {
        let deadline = std::time::Instant::now() + limit;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }
}

#[cfg(test)]
mod pipe_security_tests {
    use super::PIPE_SDDL_OWNER_ONLY;

    /// ★회귀 박제 (Windows named pipe = UDS 0o600 대칭 봉인):
    /// 기본 ServerOptions::create()는 lpSecurityAttributes=NULL로 파이프를 만들어
    /// 기본 DACL(같은 머신 임의 로컬 사용자에게 read/write 허용)을 받는다 — 인증 없는
    /// 제어 채널(send_text·send_key·ledger.kill)이 권한 우회로 노출되는 비대칭.
    /// 수정본은 owner-only SDDL을 SECURITY_ATTRIBUTES로 실어 creator·SYSTEM·Administrators만
    /// 접근하게 봉인한다. 이 테스트는 그 SDDL이 (a)광역 SID를 포함하지 않고 (b)보호된 DACL이며
    /// (c)owner를 명시 허용함을 단정해, 누군가 광역 권한을 다시 끼워넣거나 D:P를 떼어내면 깨진다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, SDDL 문자열 정합성으로 의도를 박제한다.)
    #[test]
    fn pipe_sddl_excludes_world_and_is_protected_owner_only() {
        let sddl = PIPE_SDDL_OWNER_ONLY;
        // (b) 보호된 DACL — 부모 ACL 상속을 차단해 광역 ACE가 흘러들지 않게 한다.
        assert!(
            sddl.starts_with("D:P"),
            "DACL must be protected (D:P) to block inherited world ACEs: {sddl}"
        );
        // (c) owner(creator)·SYSTEM·Administrators full-access ACE 존재.
        assert!(
            sddl.contains("(A;;FA;;;OW)"),
            "owner (OW) must have full access: {sddl}"
        );
        assert!(
            sddl.contains("(A;;FA;;;SY)") && sddl.contains("(A;;FA;;;BA)"),
            "SYSTEM (SY) and Administrators (BA) must be present: {sddl}"
        );
        // (a) 광역 SID 금지: Everyone(WD)·Authenticated Users(AU)·Anonymous(AN)·
        //     Network(NU)가 ACE로 들어오면 같은 머신/네트워크의 타 사용자가 접근 가능 → 회귀.
        for world in [";;;WD)", ";;;AU)", ";;;AN)", ";;;NU)"] {
            assert!(
                !sddl.contains(world),
                "broad SID {world} would re-open the pipe to other users: {sddl}"
            );
        }
        // deny ACE("D;")가 아닌 allow ACE("A;")만으로 구성 — 의도된 화이트리스트.
        assert!(
            !sddl.contains("(D;"),
            "owner-only seal should be an allow-list, not contain deny ACEs: {sddl}"
        );
    }

    /// ★회귀 박제 (Windows accept_loop의 connect() 오류 후 100% CPU spin 방지):
    /// 과거 Windows arm은 `loop { if server.connect().await.is_ok() { ... } }` 형태로
    /// 오류 분기가 전무했다. mio `ConnectNamedPipe`는 진짜 OS 오류를 즉시 Err로 돌려주고
    /// (정상 대기만 WouldBlock→tokio await) connecting 플래그도 즉시 해제하므로, 같은 broken
    /// 인스턴스에 sleep 없이 곧장 재시도하면 같은 Err가 무한 반복돼 tokio 워커 스레드가 영구
    /// 100% CPU를 태운다(자원 거버넌스를 표방하는 24/365 데몬에 치명적). 수정본은 오류 분기에서
    /// ①로그 ②인스턴스 재생성 ③backoff sleep로 회생한다. 그 backoff가 0이면 spin이 되살아나므로,
    /// 정책 상수가 non-zero임을 단정해 누가 다시 0/제거하면 깨지게 박제한다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로 정책 상수 정합성으로 의도를 박제한다 —
    ///  PIPE_SDDL_OWNER_ONLY 박제와 같은 방식.)
    #[test]
    fn pipe_accept_error_backoff_is_nonzero_to_prevent_cpu_spin() {
        let backoff = super::PIPE_ACCEPT_ERROR_BACKOFF;
        assert!(
            !backoff.is_zero(),
            "accept-error backoff must be non-zero, else connect() Err re-tries on the same \
             broken pipe instance with no yield → 100% CPU spin: {backoff:?}"
        );
    }
}

#[cfg(test)]
mod auto_restore_tests {
    use super::{autorestore_retry_delay, decide_auto_restore, loop_auto_restore_with, AutoRestore};

    /// opt-out(CYS_NO_AUTORESTORE=1)이면 phoenix가 있어도 스폰하지 않는다.
    #[test]
    fn opted_out_never_spawns() {
        let dir = std::env::temp_dir().join(format!("cys-ar-optout-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(
            decide_auto_restore(&dir, true, &bin, "/usr/bin:/bin"),
            AutoRestore::OptedOut
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// phoenix 미설치(구 배포)면 조용히 skip — 데몬은 정상 기동한다.
    #[test]
    fn missing_phoenix_skips() {
        let dir = std::env::temp_dir().join(format!("cys-ar-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        match decide_auto_restore(&dir, false, &dir, "/usr/bin:/bin") {
            AutoRestore::PhoenixMissing(p) => {
                assert!(p.ends_with("bin/javis_phoenix.py"), "부재 경로: {}", p.display())
            }
            other => panic!("expected PhoenixMissing, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// phoenix 설치 시 `python3 <phoenix> restore --auto` 스폰 스펙을 낸다(--auto 필수).
    #[test]
    fn present_phoenix_builds_auto_restore_command() {
        let dir = std::env::temp_dir().join(format!("cys-ar-ready-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let ph = bin.join("javis_phoenix.py");
        std::fs::write(&ph, "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin") {
            AutoRestore::Ready { program, args, .. } => {
                assert_eq!(program, "python3");
                assert_eq!(args[0], ph.to_string_lossy());
                assert_eq!(&args[1..], &["restore".to_string(), "--auto".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 실존하면 PHOENIX_CYS 를 그 절대경로로 주입하고, exe_dir 가 PATH 에 없으면
    /// PATH 를 선두주입한다(GUI/데몬 최소 PATH 침묵사 근원 수리). "python3" 문자열 단언만으로는 불충분(D4).
    #[test]
    fn ready_injects_phoenix_cys_and_path_env() {
        let dir = std::env::temp_dir().join(format!("cys-ar-env-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir 에 실행가능 cys 스텁을 둔다(PHOENIX_CYS 주입 조건 = 파일 실존).
        let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
        let cys_path = bin.join(cys_name);
        std::fs::write(&cys_path, "#!/bin/sh\n").unwrap();
        // GUI/데몬 최소 PATH 모사 — exe_dir 미포함이라 선두주입이 일어나야 한다.
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin:/usr/sbin:/sbin") {
            AutoRestore::Ready { env, .. } => {
                let cys_env = env
                    .iter()
                    .find(|(k, _)| k == "PHOENIX_CYS")
                    .map(|(_, v)| v.clone());
                assert_eq!(
                    cys_env.as_deref(),
                    Some(cys_path.to_string_lossy().as_ref()),
                    "PHOENIX_CYS 는 exe 옆 cys 절대경로여야 한다"
                );
                let path_env = env
                    .iter()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| v.clone())
                    .expect("PATH 선두주입이 있어야 한다(exe_dir 미포함 PATH)");
                assert!(
                    path_env.starts_with(bin.to_string_lossy().as_ref()),
                    "PATH 는 exe_dir 선두여야 한다: {path_env}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 없으면 PHOENIX_CYS 를 주입하지 않는다(존재하지 않는 경로 강제 주입으로
    /// 재차 FileNotFoundError 를 만들지 않는다 — phoenix 의 which→표준경로 폴백에 위임).
    #[test]
    fn ready_omits_phoenix_cys_when_absent() {
        let dir = std::env::temp_dir().join(format!("cys-ar-nocys-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin") {
            AutoRestore::Ready { env, .. } => {
                assert!(
                    !env.iter().any(|(k, _)| k == "PHOENIX_CYS"),
                    "cys 부재 시 PHOENIX_CYS 무주입이어야 한다: {env:?}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    use std::cell::RefCell;
    use std::time::Duration;

    /// ★W1(codex major): 1차 비0 → (수동 복원으로 대상 라이브) → 2차 NOOP(=0)·정확히 2회 실행·중복 스폰 0.
    /// run_auto_restore_once 대신 스크립트 러너를 주입해 sleep 0 으로 결정론 검증(60s 실 sleep 회귀 회피).
    #[test]
    fn retry_first_nonzero_then_noop_runs_twice() {
        let calls = RefCell::new(Vec::<u32>::new());
        // 1차=exit 1(비0) → 2차=exit 0(수동 복원 후 재산정 NOOP). 스폰은 각 attempt 1회씩만(중복 0).
        let scripted = |attempt: u32| -> Option<i32> {
            calls.borrow_mut().push(attempt);
            if attempt == 0 { Some(1) } else { Some(0) }
        };
        let runs = loop_auto_restore_with(scripted, Duration::from_millis(0));
        assert_eq!(runs, 2, "1차 비0→2차 실행이어야 한다(정확히 2회)");
        assert_eq!(*calls.borrow(), vec![0, 1], "attempt 0,1 각 1회 — 중복 스폰 0");
    }

    /// ★재시도 소진: 2차도 비0이면 무한 재시도 금지(정확히 2회에서 종료).
    #[test]
    fn retry_exhausts_after_one_retry() {
        let n = RefCell::new(0u32);
        let runs = loop_auto_restore_with(
            |_a| {
                *n.borrow_mut() += 1;
                Some(1)
            },
            Duration::from_millis(0),
        );
        assert_eq!(runs, 2, "비0 후 1회만 재시도 — 무한 루프 금지");
        assert_eq!(*n.borrow(), 2);
    }

    /// ★exit 5(BREAKER)·6(CORRUPT/identity)=재시도 금지 — 정확히 1회 실행.
    #[test]
    fn breaker_and_corrupt_never_retry() {
        for code in [5, 6] {
            let n = RefCell::new(0u32);
            let runs = loop_auto_restore_with(
                |_a| {
                    *n.borrow_mut() += 1;
                    Some(code)
                },
                Duration::from_millis(0),
            );
            assert_eq!(runs, 1, "exit {code} 는 재시도 금지(1회 실행)");
        }
    }

    /// ★성공(0)은 재시도 없음 — 1회 실행.
    #[test]
    fn success_runs_once() {
        let runs = loop_auto_restore_with(|_a| Some(0), Duration::from_millis(0));
        assert_eq!(runs, 1);
    }

    /// ★스폰 실패(None)도 비0 클래스 — 1회 재시도 후 소진(2회).
    #[test]
    fn spawn_failure_retries_once() {
        let runs = loop_auto_restore_with(|_a| None, Duration::from_millis(0));
        assert_eq!(runs, 2, "None(스폰 실패)도 1회 재시도 후 종료");
    }

    /// ★CYS_AUTORESTORE_RETRY_DELAY_MS override 파싱(기본 60000·override 반영).
    #[test]
    fn retry_delay_env_override() {
        // 기본값
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(60_000));
        // override — 이 테스트만 단일 스레드 실행 계약(--test-threads=1)이라 env 격리 안전.
        std::env::set_var("CYS_AUTORESTORE_RETRY_DELAY_MS", "0");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(0));
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
    }
}
