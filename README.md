# WhatsGoingOn (`wgo`)

[![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%2F11-0078D6?logo=windows)](https://www.microsoft.com/windows)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**See every running terminal, `cmd.exe`, PowerShell, WSL, and background dev server on Windows — who started it, what's inside it, and what port it's holding.**

When AI coding agents, IDEs, scripts, and dev tools spawn terminals in the background, you end up with mystery black windows, hidden shells, and occupied ports. **`wgo`** gives you one clean, fast native dashboard to see what's actually going on and act on it immediately.

---

## Key Features

| Feature | What it solves |
| :--- | :--- |
| **Owner Attribution & Ancestry** | Walks the parent process chain to show *who* spawned each terminal (VS Code, Cursor, Claude, Codex, Windows Terminal, JetBrains IDEs, Docker, Explorer, etc.). |
| **Visible / Minimized / Hidden** | Uses Win32 `EnumWindows` to distinguish visible, minimized, and headless/hidden shells — including **per-window cards** for multi-window Windows Terminal (`Window N/M`) and Windows 11 Default Terminal Delegation. |
| **Listening Ports & Orphaned Servers** | Detects TCP listening ports (`:3000`, `:3030`, `:5173`, etc.) and surfaces detached dev servers (`bun`, `node`, `python`, `uvicorn`, `cargo`, `deno`, `go`, `dotnet`, `java`) even after their parent shell has exited. |
| **Live Console Output Peek** | Reads the live Win32 console screen buffer (`AttachConsole` + `ReadConsoleOutputCharacterW`) inside the inspector — works even on hidden windows, and tells you when a window's buffer is completely blank. |
| **Smart Purpose Tags** | Automatically categorizes **AI Agent / MCP**, **Dev Server**, **Build / Test**, **Git / VCS**, **Script / Tool**, and **Idle / Blank** shells. |
| **One-Click Actions** | **Focus**, **Minimize**, **Hide / Show**, **Close Window** (`WM_CLOSE`), **Kill Process Tree**, **Open Working Folder**, **Locate `.exe`**, **Copy Command**, and **Copy Summary**. |

---

## Quick Start

### Option 1: Download Pre-built Binary
Download `wgo.exe` from the [Latest Release](https://github.com/jhalaga/whatsgoingon/releases/latest) and run it directly — no runtime dependencies required.

### Option 2: Build from Source
Requires [Rust](https://rustup.rs/) (MSVC toolchain on Windows):

```powershell
git clone https://github.com/jhalaga/whatsgoingon.git
cd whatsgoingon
cargo run --release
```

Or install `wgo` globally into Cargo's bin folder:

```powershell
cargo install --path .
wgo
```

---

## Common Workflows

- **Find what's holding a local port (e.g. `:3030`)**: Click the **`⇄ Listening Ports`** pill in the top bar or type `3030` in the search box.
- **Identify a mystery black `cmd.exe` window**: Click **`● Visible`** (or **`○ Idle / Blank`**), select the card, and check **Who Started This**, **What's Running Inside**, and **Console Output Peek**.
- **Clean up stuck agent or dev-server shells**: Select the card and click **Kill Tree** to terminate the shell and all of its child processes cleanly.

---

## Architecture

- **`eframe` / `egui`** — Native GPU-accelerated dark UI with background worker threads for non-blocking process scans and console peeks.
- **`windows-sys`** — Direct Win32 APIs (`EnumWindows`, `GetExtendedTcpTable`, `AttachConsole`, `ReadConsoleOutputCharacterW`, `PostMessageW`, `TerminateProcess`).
- **`sysinfo`** — Cross-process CPU, memory, command line, executable path, and working directory collection.

---

## License

MIT — see [LICENSE](LICENSE).
