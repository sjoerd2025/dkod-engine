# dk-spike — backend crate evaluation

> **Not a product crate.** `dk-spike` exists only to force candidate
> dependencies into the workspace compile graph so we can confirm they
> co-habitate cleanly before wiring them into real crates.
> `publish = false`, `version = "0.0.0"`.

## Status of each candidate

All of the crates below pass `cargo check -p dk-spike` against the
existing dkod-engine workspace (tonic 0.14, prost 0.14, tokio 1.x,
sqlx 0.8, rustls/ring). No conflicts, no duplicate resolver picks that
bricked production crates.

| Crate | Version | Status | Planned home in dkod |
|---|---|---|---|
| `octocrab` | `0.49` | ✅ | `dk-github` — PR/issue ops for the review loop, wraps GitHub REST. |
| `genai` | `0.5.3` | ✅ | `dk-agent-sdk` / `dk-runner` — multi-provider LLM client (OpenAI, Anthropic, xAI, Ollama, Groq, DeepSeek). Sits underneath the BAML-generated clients for ad-hoc calls that don't fit a BAML function. |
| `enum_dispatch` | `0.3` | ✅ | `dk-runner` — zero-cost `dyn Trait` replacement for workflow step enums. |
| `typetag` | `0.2` | ✅ | `dk-protocol` / `dk-engine` — serde-friendly trait objects for persisted step payloads + analytics events. |
| `tracing-core` | `0.1` | ✅ | `dk-observability` — direct dep once we write a custom Langfuse subscriber. |
| `gh-workflow` | `0.8` | ✅ | `dk-runner` — agent-authored GitHub Actions YAML builder (matches the `gh aw` flow the user flagged in knowledge). |
| `cargo-issue-lib` | `0.1` | ✅ | Tooling — flag open GitHub issues at compile time via proc-macro. |
| `langfuse-sdk` | `0.1.1` | ✅ | `dk-observability` — typed Langfuse traces/generations/scores client. Complements the OTLP exporter already used for cloud.langfuse.com. |
| `rmcp-macros` | `1.5` | ✅ | `dk-mcp` — proc-macros from the official rmcp SDK. Only promotes if we refactor our hand-rolled MCP server; otherwise we consume via `rmcp` directly. |
| `connectrpc` | `0.3.2` (feat: `client`) | ✅ | `dk-server` / clients — Connect-RPC (HTTP+protobuf, browser-friendly) alongside the existing tonic gRPC surface. |
| `buffa` | `0.3.0` (feat: `json`) | ✅ | `dk-protocol` — protobuf ↔ JSON for Connect clients without a separate code path. |
| `buffa-types` | `0.3.0` (feat: `json`) | ✅ | `dk-protocol` — type helpers for Buffa. |
| `swiftide` | `0.32.1` | ✅ | `dk-agent-sdk` — streaming RAG / ingestion pipelines. Uses rig under the hood; heavy but keeps retrieval code declarative. |
| `dspy-rs` | `0.7.3` | ✅ | `dk-agent-sdk` — prompt optimization. Does **not** conflict with `genai`: DSPy wraps its own LLM traits and BAML handles the typed-call layer on top. |
| `baml` | `0.221.0` | ✅ | `dk-agent-sdk` — runtime for the BAML-generated Rust client (see `baml_src/`). Agent + workflow graphs are authored in `.baml`, compiled to typed Rust via `baml-cli generate`. |

## Binary-only peers (installed to the dev environment, not a Cargo dep)

| Tool | How to get it | Role in dkod |
|---|---|---|
| `rtk` | `cargo install --git https://github.com/rtk-ai/rtk` | Repo toolkit; used by dev workflows, not linked into the engine. |
| `grit` | `cargo install --git https://github.com/rtk-ai/grit grit` | Code-search / structural rewrites CLI (the package has multiple bins, must specify `grit`). |
| `baml-cli` | `cargo install baml-cli` | Compiles `baml_src/*.baml` into `baml_client/` (Rust module). |
| `icm` | `cargo install --git https://github.com/rtk-ai/icm --bin icm` | "Infinite Context Memory" — single-binary memory/knowledge graph server. Exposes itself as an MCP server; dkod registers it as a managed server, does **not** vendor the code. |

## BAML workflow (source of truth for agents + graphs)

See `baml_src/` at the repo root. The project already has:

- `generators.baml` — Rust generator pointing at `baml_client/` (gitignored; regenerate with `baml-cli generate`).
- `clients.baml` — `DkodDefault`, `DkodJudge`, `DkodAnthropic` LLM clients. All secrets read from env vars (`DK_LLM_*`, `CLAUDE_API_KEY`) — none embedded.
- `agents.baml` — the actual graph-building functions:
  - `PlanWorkflow(intent, repo_context) -> WorkflowPlan` — orchestrator that emits a `{steps, edges, entry}` DAG. dk-runner walks the graph at runtime.
  - `JudgeChangeset(summary, review_notes) -> ReviewVerdict` — LLM-as-judge approval gate.
  - `DeepReview(diff, context) -> ReviewVerdict` — reviewer pass used before judge.

Regenerate the Rust client after any `.baml` edit:

```bash
baml-cli generate  # from repo root; writes baml_client/
cargo check        # confirms the generated module type-checks
```

## What's explicitly out of scope in this PR

- No production crate is touched. `dk-core`, `dk-engine`, `dk-server`, `dk-runner`,
  `dk-mcp`, `dk-cli`, `dk-agent-sdk`, `dk-protocol`, `dk-analytics`: untouched.
- No runtime integration of BAML — only the `.baml` source files + the
  `baml` crate in `dk-spike`. The generated `baml_client/` is gitignored.
- No dk-server Langfuse exporter yet — that lands in the follow-up PR
  together with `dk-observability`.
- No icm server wiring. This PR only documents how to install the
  `icm` binary; actual registration in `dk-mcp` lands in the follow-up.

## Follow-up PR plan

1. Promote passing crates out of `dk-spike` into real crates:
   - `dk-github` (octocrab, gh-workflow)
   - `dk-observability` (langfuse-sdk, tracing-core, opentelemetry-otlp)
   - `dk-agents` (baml, genai, dspy-rs, swiftide)
2. Wire `baml-cli generate` into a `cargo xtask` or `justfile` target so CI
   regenerates the client from `baml_src/`.
3. Register `icm` in `dk-mcp`'s managed-server registry.
4. Seed the clickhouse-monitoring "managed MCP servers" page from the
   platform.csv the user attached (Linear, Notion, Stripe, Figma, etc.).
5. Delete `dk-spike` once all its deps have found a real home.
