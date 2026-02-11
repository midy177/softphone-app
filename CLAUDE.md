# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Tauri 2.0 desktop softphone application: Vue 3 + TypeScript frontend with a Rust backend implementing SIP VoIP functionality.

## Build & Development Commands

```bash
# Development (starts both Vite dev server and Tauri backend)
bun run tauri dev

# Production build (bundles frontend + Rust binary)
bun run tauri build

# Frontend only
bun run dev          # Vite dev server on port 1420
bun run build        # TypeScript check + Vite production build

# Rust backend only (from src-tauri/)
cargo build
cargo check
cargo test
```

Package manager is **Bun** (not npm/yarn). Use `bun install` for dependencies.

## Architecture

### Frontend (`src/`)
- Vue 3 with `<script setup>` + TypeScript strict mode
- Tailwind CSS v4 with oklch color theme variables
- shadcn-vue component library (configured in `components.json`)
- Path alias: `@/` → `./src/`
- Communicates with Rust via Tauri commands (`@tauri-apps/api`)

### Rust Backend (`src-tauri/src/`)

**Entry points:**
- `main.rs` — binary entry, calls `lib.rs`
- `lib.rs` — Tauri app initialization, registers commands

**SIP module (`sip/`)** — Core VoIP protocol stack:
- `mod.rs` — `SipClient` orchestrator: registration, call routing, dialog management, media playback modes (File, TTS, Echo, Silent)
- `registration.rs` — Periodic REGISTER requests
- `make_call.rs` — Outbound INVITE handling, media preparation, RTP streaming
- `coming_request.rs` — Incoming INVITE processing
- `dialog.rs` — Call state machine (calling → ringing → active → terminating)
- `helpers.rs` — Transport creation, SDP offer generation, local IP detection, protocol enum (UDP/TCP/TLS/WS/WSS)
- `stun.rs` — NAT traversal via STUN for external IP detection

**Audio module (`audio/`)** — Media pipeline:
- `player.rs` — RTP packet encoding/streaming (G.711 PCMU/PCMA, Opus)
- `converter.rs` — Audio format/sample-rate conversion
- `tts.rs` — Text-to-speech via `msedge_tts`, feeds into audio converter pipeline

**Infrastructure:**
- `logging.rs` — `tracing` crate with custom compact color formatter
- `signal.rs` — Graceful shutdown (Unix signals + Windows equivalents)

### Key Rust Patterns
- Async/await with `tokio` runtime throughout
- `mpsc` channels for call request communication
- `CancellationToken` for task cancellation and graceful shutdown
- `rsipstack::Result<T>` for error propagation
- `rust_embed!` macro for embedding default audio assets
- Media is pre-generated before call setup to minimize latency

### Key Rust Dependencies
- `rsipstack` / `rsip` — SIP protocol stack
- `rtp_rs` / `sdp_rs` — RTP and SDP handling
- `stun_rs` — STUN protocol
- `cpal` / `rodio` / `rubato` — Audio capture, playback, resampling
- `msedge_tts` — Text-to-speech engine
- `tracing` — Structured logging

### Data Flow
1. Vue UI → Tauri command → Rust backend
2. SipClient handles SIP signaling (REGISTER, INVITE, BYE)
3. Audio pipeline: source file/TTS → format conversion → codec encoding → RTP packets
4. Dialog state changes propagate back to frontend
