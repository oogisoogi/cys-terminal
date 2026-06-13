# 경쟁 시스템 적대적 벤치마킹 — OpenClaw · Hermes vs aiterm+Jarvis

> 2026-06-11 · deep-research 하네스(검색 5각도→24출처→25클레임 3표 적대검증, 반박 탈락 0) + 로컬 증거(외부 터미널 체계 hermes hook 소스, _learn-hermes 전사 증류).
> 조사 목적: 그들이 **여전히 우수한 점**의 정확한 파악 (박사님 지시).

## 종합 요약

OpenClaw(구 Clawdbot→Moltbot, 2026-01 리브랜딩)와 Hermes Agent(Nous Research, MIT)는 모두 '단일 장수명 게이트웨이 데몬'을 허브로 삼는 자가호스팅 개인 AI 어시스턴트로, aiterm의 터미널 멀티플렉서 코어와 달리 메시징 채널(WhatsApp·Telegram·Slack·Discord·Signal 등)을 1차 표면으로 삼는다. 그들이 aiterm+Jarvis 팩 대비 여전히 우수한 지점은 우선순위 순으로 ①Hermes의 '구현 완료된' 자가개선 루프(경험→스킬 자동생성·사용 중 개선·FTS5 세션검색·Honcho 사용자모델 — 우리의 RSI '골격'과 깊이 차이) ②양사의 다채널 도달층(터미널/Tauri 밖의 메신저·이메일·음성) ③OpenClaw의 ClawHub 커뮤니티 스킬 레지스트리(SKILL.md 배포 생태계) ④OpenClaw의 모바일 포함 capability-선언 Node 모델 ⑤OpenClaw의 4모드 인플라이트 메시지 큐(steer/followup/collect/interrupt) ⑥Hermes의 6종 실행 백엔드(Modal/Daytona 서버리스 하이버네이션 + 컨테이너 하드닝)이다. 흡수 추천 설계: 스킬 자동생성+세션 FTS 검색(RSI 골격의 실체화), origin-키 세션 라우팅+cron별 fresh session, lane-aware 동시성 큐와 steer/interrupt 모드, SKILL.md 호환 스킬 포맷. 역으로 aiterm의 데몬 소유 PTY·승인 Feed(blocking)·자원 거버넌스(watchdog·프로세스 원장)·역할 디렉티브 팩은 두 시스템 문서에서 대응물이 확인되지 않은 우리 고유 강점이다.

## 검증된 사실 발견 (클레임별 출처·증거)

### 1. [high] OpenClaw 정체·아키텍처: 사용자 기기에서 직접 구동하는 self-hosted 개인 AI 어시스턴트(구 Clawdbot→Moltbot). 코어는 Node.js(24 권장/22.19+ 최소) TypeScript/pnpm 모노레포의 단일 'Gateway' 데몬으로, 'openclaw onboard --install-daemon'이 launchd(macOS)/systemd user service(Linux)로 상주 설치한다. 세션·채널·도구·이벤트의 단일 컨트롤 플레인이며, aiterm의 Rust 코어 데몬+UDS/named-pipe NDJSON RPC와 대비되는 Node 단일프로세스 설계다.

- 증거: README verbatim: "OpenClaw is a personal AI assistant you run on your own devices" · "Local-first Gateway — single control plane for sessions, channels, tools, and events" · "OpenClaw Onboard installs the Gateway daemon (launchd/systemd user service) so it stays running. Runtime: Node 24 (recommended) or Node 22.19+." CNBC 및 다수 2차 출처가 리브랜딩 연혁(1/27, 1/30) 교차확인. 단, LLM 추론 자체는 클라우드 API 의존 가능(어시스턴트가 로컬이지 모델이 로컬은 아님).
- 출처: https://github.com/openclaw/openclaw · https://docs.openclaw.ai/install · https://www.cnbc.com/2026/02/02/openclaw-open-source-ai-agent-rise-controversy-clawdbot-moltbot-moltbook.html
- 검증: 3-0 (claims 0, 1 병합)

### 2. [high] OpenClaw 허브는 메시징-채널 게이트웨이다: 단일 장수명 Gateway가 WhatsApp(Baileys)·Telegram(grammY)·Slack·Discord·Signal·iMessage·WebChat 등 모든 메시징 표면을 소유하고, 127.0.0.1:18789에서 typed WebSocket API(JSON Schema 프레임 검증)를 노출하며 agent/chat/presence/health/heartbeat/cron 이벤트를 프로토콜 레벨로 발행한다. aiterm의 NDJSON RPC+heartbeat 스케줄러+출력 헬스 룰과 기능적으로 대응하나, OpenClaw의 health 이벤트는 transport/gateway 헬스이지 aiterm식 '출력 파싱 기반 로그인 만료 감지'는 아니다.

- 증거: 아키텍처 문서 verbatim: "A single long-lived Gateway owns all messaging surfaces (WhatsApp via Baileys, Telegram via grammY, Slack, Discord, Signal, iMessage, WebChat)" · "Exposes a typed WS API... Validates inbound frames against JSON Schema... Emits events like agent, chat, presence, health, heartbeat, cron". heartbeat 문서: "Heartbeat runs periodic agent turns in the main session" — aiterm heartbeat(정시 과업을 AI stdin push)의 직접 대응물.
- 출처: https://docs.openclaw.ai/concepts/architecture · https://docs.openclaw.ai/gateway/protocol · https://docs.openclaw.ai/gateway/heartbeat
- 검증: 3-0 (claims 4, 5 병합)

### 3. [high] OpenClaw 멀티에이전트 모델은 '격리'다: 인바운드 채널/계정/피어를 결정론적 우선순위 매칭(peer>guild>account>channel>default)으로 격리된 에이전트(각자 workspace ~/.openclaw/workspace-<agentId> + 전용 세션 스토어)에 라우팅한다. '하나의 어시스턴트가 다수의 격리 에이전트를 전면'하는 구조로, aiterm의 pane=동등 노드 양방향 push(--to master) 모델과 구조적으로 다르다 — 에이전트 간 수평 통신이 아니라 게이트웨이 중개 디스패치.

- 증거: README: "Multi-agent routing — route inbound channels/accounts/peers to isolated agents (workspaces + per-agent sessions)". 문서: "An agent is a fully scoped brain with its own: Workspace... State directory (agentDir)... Session store" + "Never reuse agentDir across agents (it causes auth/session collisions)". aiterm의 peer-to-peer push는 이 모델에 없음 — 우리 고유 강점으로 역확인됨.
- 출처: https://github.com/openclaw/openclaw · https://docs.openclaw.ai/concepts/multi-agent
- 검증: 3-0 (claim 2)

### 4. [high] OpenClaw는 공개 스킬 레지스트리 ClawHub(2026 초 13,000+ 스킬, GitHub 계정 1주+ 조건의 open-publish)를 보유하며, 스킬은 SKILL.md(YAML frontmatter+markdown) 디렉터리로 ~/.openclaw/workspace/skills/<skill>/ 등 다층 경로에서 로드된다. 커뮤니티 배포형 능력 확장 메커니즘으로, aiterm의 역할 디렉티브 팩(내장 4종)에는 없는 생태계 우위다.

- 증거: clawhub 리포는 "Skill + Plugin Registry for OpenClaw"로 명명, `openclaw skills install`로 설치. 스킬 로드 경로는 workspace skills 폴더가 최우선이나 <workspace>/.agents/skills, ~/.agents/skills 등 복수. 13k 스킬 수치는 2차 출처 보고로 정확 시점 변동 가능.
- 출처: https://github.com/openclaw/clawhub · https://docs.openclaw.ai/tools/skills
- 검증: 3-0 (claim 3)

### 5. [high] OpenClaw는 컨트롤플레인 클라이언트(macOS 앱·CLI·웹 UI·자동화)와 'Nodes'(macOS/iOS/Android/headless, role: node로 접속하며 caps/commands를 명시 선언 — camera, location.get, sms.send, canvas 등 capability-gated)를 구분한다. 모바일까지 도달하는 capability-선언 노드 모델로, aiterm의 로컬 pane-as-node보다 디바이스 도달 범위가 넓다.

- 증거: 문서 verbatim: "Nodes (macOS/iOS/Android/headless) also connect over WebSocket, but declare role: node with explicit caps/commands" · "Nodes must include role: \"node\" plus caps/commands/permissions in connect." iOS/Android 페어링은 독립 3자 가이드(janaksenevirathne.medium.com 등)로 교차확인.
- 출처: https://docs.openclaw.ai/concepts/architecture · https://docs.openclaw.ai/nodes
- 검증: 3-0 (claim 6)

### 6. [high] OpenClaw 세션 영속·라우팅: 모든 세션 상태는 게이트웨이가 소유하고 UI는 질의만 한다(파일 기반 — ~/.openclaw/agents/<agentId>/sessions/sessions.json 메타데이터 + <sessionId>.jsonl append-only 트랜스크립트, UI 재시작 생존). 메시지는 origin별로 세션에 키잉(DM=공유 메인, 그룹/룸/웹훅=격리, cron=매 실행 fresh session — 권한 상속 차단 sanitization 포함). aiterm의 데몬 소유 PTY와 '장수명 로컬 프로세스가 세션을 소유해 UI와 무관하게 영속'한다는 점은 동형이나, 소유 대상이 채팅 트랜스크립트 vs 터미널 PTY로 다르다.

- 증거: 문서 verbatim: "All session state is owned by the gateway. UI clients query the gateway for session data." + 두 파일 경로 명시 + "Isolated cron jobs always mint a fresh sessionId per run (no idle reuse)". 단 GitHub issue #66522: 게이트웨이 재시작 시 sessions.json 인덱스 재구축으로 UI 가시성 소실 사례(jsonl 원본은 보존) — 인덱스/원본 분리의 약점.
- 출처: https://docs.openclaw.ai/concepts/session · https://docs.openclaw.ai/automation/cron-jobs
- 검증: 3-0 (claims 7, 8, 9 병합)

### 7. [high] OpenClaw 동시성 제어: 순수 TypeScript+promise 인프로세스 lane-aware FIFO 큐(외부 의존성·워커스레드 없음)로 인바운드 auto-reply 실행을 직렬화한다. 세션키 lane(session:<key>)당 활성 실행 1개 보장, lane별 동시성 캡(기본 1, main=4, subagent=8), 전역은 agents.defaults.maxConcurrent. 실행 중 도착 메시지는 4모드 처리 — steer(기본: 도구 실행 후 활성 런타임에 주입), followup(후속 턴 큐잉), collect(quiet window 후 병합), interrupt(중단 후 최신 메시지 실행), 기본값 debounceMs=500/cap=20/drop=summarize. aiterm에는 이 '실행 중 메시지 핸들링 정책' 계층이 없다.

- 증거: 문서 verbatim: "a tiny in-process queue to prevent multiple agent runs from colliding, while still allowing safe parallelism across sessions" · "No external dependencies or background worker threads; pure TypeScript + promises" · 4모드와 기본값 전부 문서 일치. 약점도 확인: issue #48488(hung promise가 세션 lane 영구 블록 — 순수 promise 큐의 특성적 실패 모드), issue #50880(Discord에서 steer가 followup으로 강등 보고, 미확인 단건). 흡수 시 steer/interrupt 모드 + lane 캡 설계가 핵심 차용 포인트.
- 출처: https://docs.openclaw.ai/concepts/queue
- 검증: 3-0 (claims 10, 11, 12, 13 병합)

### 8. [high] Hermes Agent 정체·핵심 차별점: Nous Research의 MIT 오픈소스(github.com/NousResearch/hermes-agent, v0.16.0 2026-06-06, ~46k stars) 서버 상주 자율 에이전트('IDE 코파일럿이 아니다' 포지셔닝). 핵심은 '구현 완료된' 폐쇄 학습 루프 — 복잡한 작업 후 자율 스킬 생성(v0.12.0 Curator), 사용 중 스킬 자가개선, 지식 영속 self-nudge, FTS5+LLM 요약 기반 과거 세션 검색, Honcho dialectic 사용자 모델, MEMORY.md/USER.md 영속 메모리(시스템 프롬프트 주입) + DSPy/GEPA 자가진화 컴패니언 리포. aiterm의 '장기메모리/RSI 골격'과 구현 깊이에서 가장 큰 격차 — 적대적 벤치마킹의 1순위 우위 지점.

- 증거: README verbatim: "It's the only agent with a built-in learning loop — it creates skills from experience, improves them during use, nudges itself to persist knowledge, searches its own past conversations, and builds a deepening model of who you are across sessions." 랜딩: "An autonomous agent that lives on your server, remembers what it learns, and gets more capable the longer it runs." 단 'only agent' 'never forgets'는 마케팅 과장 — 큐레이트 메모리 파일은 2,200자/1,375자 한도의 소형이고, 무한 보존은 SQLite 세션 히스토리(session_search)와 스킬 파일이 담당. '학습 루프가 실제 성능을 올린다'는 독립 벤치마크는 미확인.
- 출처: https://github.com/NousResearch/hermes-agent · https://hermes-agent.nousresearch.com/ · https://hermes-agent.nousresearch.com/docs/user-guide/features/memory
- 검증: 3-0 (claims 14, 18, 21, 22 병합)

### 9. [high] Hermes 도달층·자동화: 단일 게이트웨이 프로세스가 3개 배치 진입점(CLI cli.py / Gateway gateway/run.py / ACP 어댑터 — VS Code·Zed·JetBrains stdio JSON-RPC)을 갖고, Gateway는 20개 플랫폼 어댑터(telegram·discord·slack·whatsapp·signal·email·sms·matrix·homeassistant·webhook·api_server 등 — 사용자 문서상 메시징은 16~24개)와 사용자 인가(allowlist+DM 페어링)·슬래시 명령·hook 시스템을 제공. 음성 메모 전사·플랫폼 간 대화 연속성·자연어 cron 스케줄러(임의 플랫폼 배달, 무인 실행)·격리 서브에이전트 스폰·Python-over-RPC 도구 스크립팅('zero-context-cost' 파이프라인) 포함. aiterm heartbeat(stdin push 한정)보다 트리거·배달 표면이 넓다.

- 증거: 아키텍처 문서 verbatim: "CLI (cli.py), Gateway (gateway/run.py), ACP (acp_adapter/)" + "20 platform adapters" 명단. README: "Built-in cron scheduler with delivery to any platform... all in natural language, running unattended" · "Write Python scripts that call tools via RPC, collapsing multi-step pipelines into zero-context-cost turns"(자기 서술 용어, 독립 벤치마크 아님). cron/scheduler.py 실재 — README 카피가 아닌 구현 확인.
- 출처: https://github.com/NousResearch/hermes-agent · https://hermes-agent.nousresearch.com/docs/developer-guide/architecture · https://hermes-agent.nousresearch.com/docs/user-guide/messaging/
- 검증: 3-0 (claims 15, 17, 19, 24 병합)

### 10. [high] Hermes 실행 격리·도구 표면: 70+ 도구/~28 toolsets 중앙 레지스트리(tools/registry.py), 터미널 도구는 6개 백엔드(local·Docker·SSH·Singularity·Modal·Daytona — 랜딩페이지의 '5개'는 구버전, Daytona 추가됨). Docker는 --cap-drop ALL·no-new-privileges·--pids-limit 256, Singularity는 --containall --no-home 네임스페이스 격리, Modal/Daytona는 서버리스 영속(유휴 시 환경 하이버네이션·온디맨드 wake — 노트북 꺼져도 원격 파일시스템 생존). aiterm 데몬 소유 PTY(로컬 머신 결박)보다 실행-격리·클라우드 영속 스토리가 넓다. 단 local 백엔드는 무샌드박스, SSH는 네트워크 경계뿐 — '전부 샌드박스'는 아님.

- 증거: 아키텍처 문서 verbatim: "70+ registered tools across ~28 toolsets" · "Terminal tools support 6 backends (local, Docker, SSH, Daytona, Modal, Singularity)". README: "Daytona and Modal offer serverless persistence — your agent's environment hibernates when idle and wakes on demand, costing nearly nothing between sessions." 하드닝 플래그는 공식 설정 문서에 구체 명시.
- 출처: https://github.com/NousResearch/hermes-agent · https://hermes-agent.nousresearch.com/docs/developer-guide/architecture · https://hermes-agent.nousresearch.com/docs/user-guide/configuration
- 검증: 3-0 / 2-1 (claims 16, 20, 23 병합 — 23은 2-1이나 '5개' 수치 노후화 외 실질 반박 없음)

### 11. [medium] [적대적 벤치마킹 종합] 그들이 여전히 우수한 점 우선순위: P1=Hermes 자가개선 루프의 구현 깊이(스킬 자동생성·FTS5 세션검색·Honcho 사용자모델 — 우리 RSI는 프로토콜/골격 단계). P2=다채널 도달(양사 메신저·이메일·모바일 vs 우리 터미널/Tauri 한정). P3=ClawHub 커뮤니티 스킬 생태계(13k+ 스킬 네트워크 효과). P4=OpenClaw 모바일 Node capability 모델. P5=OpenClaw 4모드 인플라이트 큐. P6=Hermes 클라우드 실행 백엔드·샌드박스 하드닝. 흡수 제안: ①세션 트랜스크립트 SQLite FTS 검색+작업 후 스킬 자동생성 훅(P1 직격) ②SKILL.md 호환 포맷 채택으로 ClawHub/Hermes 스킬 재사용 ③lane-aware 동시성 캡+steer/interrupt 메시지 정책 ④cron별 fresh-session+권한 sanitization ⑤장기적으로 메신저 1개(예: Telegram) 채널 어댑터. 역으로 우리 고유 강점(양사 문서에 대응물 부재): pane=동등노드 양방향 push, 승인 Feed blocking wait/reply, 자원 거버넌스(watchdog·프로세스 원장·scoped run 전멸), 출력 헬스 룰(로그인 만료 감지), 역할 디렉티브 자동 주입, Rust 코어(양사 모두 Node/Python 단일프로세스 — hung promise lane 블록 등 특성적 실패 보고됨).

- 증거: 위 10개 사실 발견의 해석적 종합. '우리 고유 강점' 판정은 양사 공식 문서에서 대응 기능이 발견되지 않았다는 부재 증거(absence of evidence) 기반이므로 high가 아닌 medium — 미문서화 기능 존재 가능성 잔존. aiterm 측 기능 명세는 연구 브리프 기술을 사실로 전제.
- 출처: https://github.com/openclaw/openclaw · https://github.com/NousResearch/hermes-agent · https://docs.openclaw.ai/concepts/queue · https://docs.openclaw.ai/concepts/architecture
- 검증: synthesis (25개 확정 클레임 기반)

## 한계·주의 (caveats)

(1) 비교 기준인 aiterm+Jarvis 팩의 기능 명세는 연구 브리프에 주어진 대로 사실로 전제했으며 독립 검증하지 않았다 — 비교 절(節)의 aiterm 측 서술은 그 전제에 의존한다. (2) '그들에게 없다=우리 고유 강점' 판정은 공식 문서 부재 기반 추론이라 미문서화 기능이 있으면 뒤집힐 수 있다. (3) 두 프로젝트 모두 변화 속도가 매우 빠르다(OpenClaw 1주 2회 리브랜딩, Hermes 백엔드 5→6개로 랜딩페이지가 이미 노후) — 수치(13k 스킬, 70+ 도구, 동시성 기본값)는 2026-06-11 시점 스냅샷. (4) Hermes의 'only agent with learning loop' 'never forgets' 'zero-context-cost'는 벤더 자기서술/과장이며 학습 루프의 실제 성능 향상을 보여주는 독립 벤치마크는 확인되지 않았다. (5) 연구 질문이 요구한 경쟁 시스템(Warp, conductor류, Claude Code 오케스트레이터)은 3표 검증을 통과한 클레임에 포함되지 않아 본 종합에서 다루지 못했다. (6) OpenClaw 관련 CNBC 기사 제목이 'controversy'를 언급하나 보안 논란의 실체는 검증된 클레임에 없다. (7) 외부 터미널 체계의 hermes-agent hook 지원 자체(질문 전제)는 verifier가 GitHub issue·hooks 문서로 존재만 확인했고 통합 깊이는 미조사.

## 열린 질문 (후속 조사 후보)

- 경쟁 시스템 공백: Warp, Conductor류, Claude Code 기반 오케스트레이터(Claude Squad 등)와 외부 터미널 체계 업스트림 자체의 2026 현황은 별도 조사가 필요하다 — 본 라운드 생존 클레임에 부재.
- Hermes의 학습 루프(스킬 자동생성·GEPA 자가진화)가 실제로 측정 가능한 성능 향상을 내는가? 독립 eval/벤치마크 데이터가 존재하는가, 아니면 메커니즘만 구현된 상태인가?
- OpenClaw 'controversy'(CNBC 2026-02-02 제목 언급)의 실체 — 게이트웨이 노출·스킬 레지스트리 open-publish(1주 계정 조건)에 따른 보안 사고·공급망 위험 사례가 있는가? 우리가 SKILL.md 호환을 채택할 경우의 공급망 리스크 평가 필요.
- 외부 터미널 체계의 hermes-agent hook 통합의 구체 깊이(이벤트 종류·양방향성) — aiterm이 동일 수준의 Hermes 연동을 제공해야 할 전략적 이유가 있는가?

## 출처 목록

- [primary] https://github.com/openclaw/openclaw
- [primary] https://docs.openclaw.ai/concepts/architecture
- [primary] https://docs.openclaw.ai/concepts/session
- [blog] https://milvus.io/blog/openclaw-formerly-clawdbot-moltbot-explained-a-complete-guide-to-the-autonomous-ai-agent.md
- [primary] https://docs.openclaw.ai/concepts/queue
- [blog] https://nebius.com/blog/posts/openclaw-security
- [primary] https://github.com/NousResearch/hermes-agent
- [primary] https://hermes-agent.nousresearch.com/docs/developer-guide/architecture
- [primary] https://hermes-agent.nousresearch.com/
- [blog] https://www.mindstudio.ai/blog/hermes-agent-five-pillars-memory-skills-soul-crons
- [blog] https://securityboulevard.com/2026/06/8-self-evolving-skills-hermes-agent-writes-on-its-own/
- [secondary] https://github.com/SamurAIGPT/awesome-hermes-agent
- [primary] https://docs.openclaw.ai/concepts/multi-agent
- [blog] https://ppaolo.substack.com/p/openclaw-system-architecture-overview
- [blog] https://dev.to/entelligenceai/inside-openclaw-how-a-persistent-ai-agent-actually-works-1mnk
- [primary] https://docs.openclaw.ai/automation/cron-jobs
- [blog] https://amux.io/blog/best-multi-agent-orchestrators-2026/
- [blog] https://nimbalyst.com/blog/best-multi-agent-desktop-apps-claude-code-codex-2026/
- [primary] https://code.claude.com/docs/en/agent-teams
- [secondary] https://www.marktechpost.com/2026/05/10/openclaw-vs-hermes-agent-why-nous-researchs-self-improving-agent-now-leads-openrouters-global-rankings/
- [primary] https://snyk.io/articles/clawdbot-ai-assistant/
- [secondary] https://thehackernews.com/2026/03/openclaw-ai-agent-flaws-could-enable.html
- [forum] https://news.ycombinator.com/item?id=48056003
- [forum] https://news.ycombinator.com/item?id=47064470