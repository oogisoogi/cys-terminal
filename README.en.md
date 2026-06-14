# cys-terminal : A Dedicated Terminal for the CYSJavis System (core daemon + CLI + CYSJavis Pack)

> Independently written **from scratch**, referencing only the protocol design ideas of 외부 터미널 체계 (외부 프로젝트) — no GPL code used. Cross-platform: macOS & Windows.

## Design Principles (ABSOLUTE)

1. **Bidirectional socket communication** — no one-way send + capture polling.
   Every pane on the same socket is an **equal node** that can actively push to any other pane by surface ID.
   `cys send --surface surface:31 "..."` + `send-key Return` → injected directly into the target pane's **PTY stdin** → arrives as a new user turn.
   Server→client direction is the `cys events` push stream (sequence numbers, resume on reconnect).
2. **Resource governance as a first-class feature** — built-in mitigation for 외부 터미널 체계's fatal flaw (orphan server accumulation → load explosion → 401/hang).
3. **Core/UI separation** — the daemon (cysd) runs independently of any UI. Even if the UI hangs, the socket control channel stays alive (out-of-band recovery).

## Architecture

```
cysd  headless core daemon: NDJSON socket server (UDS / Windows named pipe), PTY (portable-pty:
         macOS openpty, Windows ConPTY), vt100 screen reconstruction, event bus, watchdog, process ledger
cys   CLI: the equal-node client used by the AI inside each pane
```

Every pane process gets `CYS_SURFACE_ID`, `CYS_SURFACE_REF`, `CYS_SOCKET` injected automatically — the AI inside a pane learns its own address instantly via `cys identify`.

## Quick Start

```bash
cargo build --release
./target/release/cysd &                      # daemon (duplicate launch auto-rejected)

cys new-surface --title worker1              # → surface:1
cys send --surface surface:1 "echo hello"
cys send-key --surface surface:1 Return
cys read-screen --surface surface:1          # screen reconstructed exactly via vt100 (--lines 200 for scrollback tail)
cys events --reconnect                       # push event stream (replaces polling)
cys attach surface:1                         # output mirror (read-only)
```

## Resource Governance (3 mitigations)

| Mitigation | Feature | Command / Event |
|---|---|---|
| ① Auth/health failure detection | Health rules matched against every output line (default: Not logged in, 401, token expired, rate limit) → push with 30s debounce | `health.alert` event · `cys add-health-rule <name> <regex>` |
| ② Short work units | Idle detection (default: 300s of no output) → push so the master can decide to split or inspect | `pane.idle` event |
| ③ Forced server lifecycle | **scoped run**: new process group + ledger registration, SIGKILL of the whole group on exit · **close-surface**: kills the pane's entire child tree · **watchdog**: detects loadavg / child count / duplicate commands (default 3+), auto-cleanup with `CYS_AUTOKILL_DUP=1` | `cys run -- <cmd>` · `cys ps` · `cys kill <pid>` · `watchdog.*` events |

## Protocol (NDJSON, one JSON per line)

Request `{"id":1,"method":"surface.send_text","params":{...}}` → Response `{"id":1,"ok":true,"result":{...}}` / `{"id":1,"ok":false,"error":{"code","message"}}`

Methods: `system.ping` `system.identify` `surface.create/list/send_text/send_key/read_text/resize/close/attach` `events.stream` `ledger.register/deregister/list/kill` `health.add_rule/list_rules`

Events: `surface.created/closed/exited/input_injected (sender tagged)` `health.alert` `watchdog.load_high/proc_count_high/duplicate_procs/duplicates_killed` `pane.idle` `ledger.registered/killed` `daemon.started`

## Environment Variables

`CYS_SOCKET` socket path (default `~/.local/state/cys/cys.sock`, Windows `\\.\pipe\cys`) · `CYS_SHELL` ·
`CYS_LOAD_THRESHOLD` (default cores×2) · `CYS_PROC_THRESHOLD` (50) · `CYS_DUP_THRESHOLD` (3) · `CYS_AUTOKILL_DUP` (0/1) · `CYS_IDLE_SECONDS` (300)

## UI (Tauri 2 + xterm.js)

```bash
cd ui && sh build.sh          # frontend bundle (bun)
cargo build -p cys-app     # dev run: ./target/debug/cys-app
bun x @tauri-apps/cli build   # release: target/release/bundle/macos/cys.app
# bundle daemon & CLI into the app: cp target/release/{cysd,cys} <app>/Contents/MacOS/
```

- **Core/UI separation**: the UI is just a socket client. Sessions (PTYs) are owned by the daemon — they survive UI restarts and app reinstalls (re-attach).
- Workspace tabs (add / rename / close) · split panes (⌘T, ⌘D, ⌘⇧D, ⌘W) · draggable divider resize.
- health/watchdog/feed push events → toasts. Note: rebuild the app after editing ui/ (frontend is embedded in the binary).

## Approval Feed (centralized worker approval requests)

```bash
cys feed push --wait --title "approve git push" --body "..."   # blocks until decided (exit 0=allow, 2=deny, 3=timeout)
cys feed list --status pending                                 # list pending requests
cys feed reply <request_id> allow                              # master, or UI Allow/Deny buttons
```

`feed.push wait=true` → the daemon blocks the connection via a oneshot channel (default 120s) until `feed.reply` releases it. Agent hook example (Claude Code PreToolUse): the hook script calls `cys feed push --wait ...` and maps the exit code to the decision.
