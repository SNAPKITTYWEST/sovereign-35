# ⬡ SOVEREIGN 3.5

**Claude 3.5 Sonnet on Bedrock. Tavily. DuckDuckGo. Lean 4 gate. Prolog constraint. vLLM coder. Tokio async fire.**

Not a wrapper. Not a playground. A sovereign reasoning agent with formal verification baked in.

---

## Stack

| Layer | Technology |
|-------|-----------|
| **Reasoning** | Claude 3.5 Sonnet (AWS Bedrock `us-east-1`) |
| **Web search** | Tavily (primary) + DuckDuckGo (fallback) |
| **Formal gate** | Lean 4 — every logical claim passes through a type checker |
| **Constraints** | SWI-Prolog — constraint satisfaction before Bedrock fires |
| **Code** | vLLM (Qwen2.5-Coder-7B) — code queries route here first |
| **Async runtime** | Tokio — Tavily, DDG, Lean4 all fire concurrently |
| **Trust layer** | Trust Deed + EmojiScript injected at system prompt |
| **Audit trail** | SHA-256 WORM chain — every step sealed |

---

## Install

```bash
git clone https://github.com/SNAPKITTYWEST/sovereign-35
cd sovereign-35
cargo build --release
```

---

## Run

```bash
# AWS credentials must be in ~/.aws/credentials (us-east-1 or us-west-2)
# TAVILY_API_KEY in .env
cargo run --release
```

```
SOVEREIGN> what is the current state of AI sovereignty in 2026?
```

The agent will:
1. Fire Lean 4 gate + Tavily + DuckDuckGo **concurrently** (Tokio)
2. Run Prolog constraint on retrieved facts
3. Route to vLLM if it's a code query
4. Call Claude 3.5 Bedrock with full search context + gate report
5. WORM seal every step

---

## For code queries

Start vLLM on bbqbaddie (or any machine with enough VRAM):

```bash
python -m vllm.entrypoints.openai.api_server \
  --model Qwen/Qwen2.5-Coder-7B-Instruct \
  --port 8000
```

Then sovereign-35 auto-routes code queries there.

---

## Dependencies

- Rust + Cargo
- AWS credentials configured (`~/.aws/credentials`)
- Lean 4 (`lean` in PATH) — optional, gates fall through gracefully
- SWI-Prolog (`swipl` in PATH) — optional
- vLLM server — optional, code queries fall through if offline

---

## Architecture

```
query
  ↓
 ┌─────────────────────────────────────────┐
 │           TOKIO JOIN (concurrent)        │
 │  Tavily search ─┐                        │
 │  DDG search ────┼─→ search_ctx           │
 │  Lean4 gate ────┘                        │
 └─────────────────────────────────────────┘
  ↓
 Prolog constraint (query facts)
  ↓
 vLLM coder (if code query)
  ↓
 Claude 3.5 Bedrock (Trust Deed + search ctx + gate report)
  ↓
 WORM seal → response
```

---

## EmojiScript notation

```
⬡ = sovereign node   Ω = seal     ↺ = recursive   Ψ = agent
Δ = change           Λ = law      Σ = sum          Φ = resonance
α = genesis
```

---

## License

Apache License 2.0 · SnapKitty Collective · Bel Esprit D'Accord Trust · 2026

*Evidence or Silence.*
