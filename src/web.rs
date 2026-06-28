// FRANKENSTEIN — BOB-powered 2-step sovereign workflow
//
// BRAIN  = Claude Sonnet 4.6 (Bedrock) — architect, Trust Deed, reasoning
// HANDS  = pgvector                    — corpus retrieval (semantic context)
// LEGS   = Granite (vLLM, local)       — code executor, Tokio-spawned baby
// WORM   = SHA-256 chain               — every step sealed
//
// Powered by BOB. SnapKitty Collective 2026. Apache 2.0.

mod inject { pub mod trust_deed; }
mod tools  { pub mod tavily; pub mod ddg; }
mod gates  { pub mod lean4; pub mod prolog; }
mod models { pub mod bedrock; pub mod vllm; }

use anyhow::Result;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
    response::Html,
};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

const PORT: u16 = 4300;
const BRAIN_MODEL:  &str = "us.anthropic.claude-sonnet-4-6";
const BATTLE_OPUS:  &str = "us.anthropic.claude-opus-4-6-v1";
const BATTLE_CODE:  &str = "mistral.devstral-2-123b";
const LEGS_MODEL:   &str = "ibm-granite/granite-3.3-8b-instruct"; // vLLM local
const VLLM_URL:     &str = "http://localhost:8000/v1/chat/completions";
const PGVECTOR_URL: &str = "http://localhost:5433/retrieve"; // pgvector sidecar

// ── Shared state ─────────────────────────────────────────────────────────────
struct AppState {
    bedrock: aws_sdk_bedrockruntime::Client,
    http:    reqwest::Client,
    worm:    Mutex<WormChain>,
}

struct WormChain { prev: String, count: u64 }

impl WormChain {
    fn new() -> Self { Self { prev: "FRANKENSTEIN_GENESIS".to_string(), count: 0 } }
    fn seal(&mut self, event: &str) -> String {
        let msg = format!("{}|{}|{}", self.prev, event, chrono::Utc::now().timestamp_millis());
        let hash = hex::encode(Sha256::digest(msg.as_bytes()));
        self.prev = hash.clone();
        self.count += 1;
        hash[..16].to_string()
    }
}

// ── Request / Response ────────────────────────────────────────────────────────
#[derive(Deserialize)]
struct WorkflowRequest { prompt: String }

#[derive(Deserialize)]
struct BattleRequest { prompt: String }

#[derive(Deserialize)]
struct AskRequest { query: String }

#[derive(Serialize)]
struct WorkflowStep {
    step:   String,
    model:  String,
    output: String,
    worm:   String,
    ms:     u128,
}

#[derive(Serialize)]
struct WorkflowResponse {
    prompt:    String,
    is_code:   bool,
    steps:     Vec<WorkflowStep>,
    final_out: String,
    worm:      WormStatus,
}

#[derive(Serialize)]
struct WormStatus { hash: String, seals: u64 }

// ── Bedrock helper ────────────────────────────────────────────────────────────
async fn brain(
    client:  &aws_sdk_bedrockruntime::Client,
    system:  &str,
    msg:     &str,
    tokens:  u32,
) -> Result<String> {
    models::bedrock::invoke_with_model(client, BRAIN_MODEL, system, &[("user", msg)], tokens).await
}

async fn brain_model(
    client:  &aws_sdk_bedrockruntime::Client,
    model:   &str,
    system:  &str,
    msg:     &str,
    tokens:  u32,
) -> Result<String> {
    models::bedrock::invoke_with_model(client, model, system, &[("user", msg)], tokens).await
}

// ── Legs: Granite via vLLM ────────────────────────────────────────────────────
async fn legs(http: &reqwest::Client, spec: &str) -> Result<String> {
    let body = serde_json::json!({
        "model": LEGS_MODEL,
        "messages": [
            {"role": "system", "content": "You are Granite — a sovereign code executor. You receive architecture specs and produce complete, working code. No explanations. Pure code output."},
            {"role": "user", "content": spec}
        ],
        "max_tokens": 2048,
        "temperature": 0.1
    });

    match http.post(VLLM_URL).json(&body).send().await {
        Ok(resp) => {
            let data: serde_json::Value = resp.json().await?;
            Ok(data["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string())
        }
        Err(_) => Ok("LEGS_OFFLINE — start Granite: python -m vllm.entrypoints.openai.api_server --model ibm-granite/granite-3.3-8b-instruct --port 8000".to_string())
    }
}

// ── Hands: pgvector context retrieval ────────────────────────────────────────
async fn hands(http: &reqwest::Client, query: &str) -> String {
    let body = serde_json::json!({ "query": query, "k": 3 });
    match http.post(PGVECTOR_URL).json(&body).send().await {
        Ok(resp) => {
            let data: serde_json::Value = resp.json().await.unwrap_or_default();
            data["results"].as_array()
                .map(|v| v.iter()
                    .map(|r| r["text"].as_str().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join("\n---\n"))
                .unwrap_or_else(|| "HANDS_OFFLINE — pgvector not running (stub: no context retrieved)".to_string())
        }
        Err(_) => "HANDS_OFFLINE — pgvector not running (corpus retrieval skipped)".to_string()
    }
}

// ── /workflow — the real Frankenstein ────────────────────────────────────────
async fn workflow_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WorkflowRequest>,
) -> Json<WorkflowResponse> {
    let prompt = req.prompt.trim().to_string();
    let mut steps: Vec<WorkflowStep> = vec![];

    let is_code = ["code","write","implement","function","build","class","asm","assembly",
                   "script","program","algorithm","struct","fn ","def ","pub "]
        .iter().any(|kw| prompt.to_lowercase().contains(kw));

    // ── STEP 1: BRAIN — architecture plan ────────────────────────────────────
    let t = std::time::Instant::now();
    let brain_system = inject::trust_deed::build_system_prompt(None);
    let brain_prompt = if is_code {
        format!("You are the ARCHITECT. Produce a structured technical spec for this task. \
            Output: (1) GOAL, (2) COMPONENTS list, (3) DATA STRUCTURES, (4) PSEUDOCODE outline. \
            Be precise. The LEGS (Granite vLLM) will implement from your spec.\n\nTASK: {}", prompt)
    } else {
        format!("Answer using Evidence or Silence protocol. Use Tavily search context if available.\n\nQUERY: {}", prompt)
    };

    let brain_out = brain(&state.bedrock, &brain_system, &brain_prompt, 1024)
        .await
        .unwrap_or_else(|e| format!("BRAIN_ERR: {}", e));

    let mut worm = state.worm.lock().await;
    let h1 = worm.seal("BRAIN");
    drop(worm);

    steps.push(WorkflowStep {
        step:   "BRAIN".to_string(),
        model:  format!("Claude Sonnet 4.6 (Bedrock · {})", BRAIN_MODEL),
        output: brain_out.clone(),
        worm:   h1,
        ms:     t.elapsed().as_millis(),
    });

    // ── STEP 2: HANDS — pgvector retrieval (parallel with brain output ready) ──
    let t = std::time::Instant::now();
    let context = hands(&state.http, &prompt).await;

    let mut worm = state.worm.lock().await;
    let h2 = worm.seal("HANDS");
    drop(worm);

    steps.push(WorkflowStep {
        step:   "HANDS".to_string(),
        model:  "pgvector (sovereign corpus)".to_string(),
        output: context.clone(),
        worm:   h2,
        ms:     t.elapsed().as_millis(),
    });

    // ── STEP 3: LEGS — Granite vLLM (only on code prompts) ───────────────────
    let legs_out = if is_code {
        let t = std::time::Instant::now();
        let spec = format!(
            "ARCHITECT SPEC:\n{}\n\nCORPUS CONTEXT:\n{}\n\nORIGINAL TASK:\n{}\n\nWrite complete working code. No prose. Pure implementation.",
            brain_out, context, prompt
        );

        // Tokio spawn — legs run independently, brain doesn't block
        let http_clone = state.http.clone();
        let spec_clone = spec.clone();
        let legs_handle = tokio::spawn(async move {
            legs(&http_clone, &spec_clone).await
        });

        let result = legs_handle.await
            .unwrap_or_else(|e| Ok(format!("SPAWN_ERR: {}", e)))
            .unwrap_or_else(|e| format!("LEGS_ERR: {}", e));

        let mut worm = state.worm.lock().await;
        let h3 = worm.seal("LEGS");
        drop(worm);

        steps.push(WorkflowStep {
            step:   "LEGS".to_string(),
            model:  format!("Granite vLLM (local · {})", LEGS_MODEL),
            output: result.clone(),
            worm:   h3,
            ms:     t.elapsed().as_millis(),
        });

        result
    } else {
        String::new()
    };

    // ── STEP 4: BRAIN REVIEW — merge + seal ───────────────────────────────────
    let t = std::time::Instant::now();
    let final_prompt = if is_code && !legs_out.starts_with("LEGS") {
        format!(
            "You are the SOVEREIGN REVIEWER. The LEGS (Granite) produced this implementation:\n\n{}\n\nYour job:\n1. Verify it matches the spec\n2. Note any gaps\n3. Add Trust Deed seal\n4. Output: VERDICT (PASS/PATCH/REJECT) + brief notes\n\nOriginal task: {}",
            legs_out, prompt
        )
    } else {
        format!("Summarize your answer with the Omega seal. Evidence or Silence.\n\n{}", brain_out)
    };

    let review = brain(&state.bedrock, &brain_system, &final_prompt, 512)
        .await
        .unwrap_or_else(|e| format!("REVIEW_ERR: {}", e));

    let mut worm = state.worm.lock().await;
    let h4 = worm.seal("REVIEW");
    let final_hash = worm.prev[..16].to_string();
    let final_seals = worm.count;
    drop(worm);

    steps.push(WorkflowStep {
        step:   "REVIEW".to_string(),
        model:  format!("Claude Sonnet 4.6 (Bedrock · {})", BRAIN_MODEL),
        output: review.clone(),
        worm:   h4,
        ms:     t.elapsed().as_millis(),
    });

    // Final output = legs code (if code) + brain review
    let final_out = if is_code && !legs_out.is_empty() {
        format!("{}\n\n---\n🧠 BRAIN REVIEW:\n{}", legs_out, review)
    } else {
        brain_out
    };

    Json(WorkflowResponse {
        prompt,
        is_code,
        steps,
        final_out,
        worm: WormStatus { hash: final_hash, seals: final_seals },
    })
}

// ── /battle — 3-way compare ───────────────────────────────────────────────────
async fn battle_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BattleRequest>,
) -> Json<serde_json::Value> {
    let prompt = req.prompt.trim().to_string();
    let system_frank = inject::trust_deed::build_system_prompt(None);
    let system_raw   = "You are a helpful AI assistant. Answer directly.";

    let (frank_res, opus_res, devstral_res) = tokio::join!(
        brain_model(&state.bedrock, BRAIN_MODEL, &system_frank, &prompt, 1024),
        brain_model(&state.bedrock, BATTLE_OPUS,  system_raw,   &prompt, 1024),
        brain_model(&state.bedrock, BATTLE_CODE,  system_raw,   &prompt, 1024),
    );

    let mut worm = state.worm.lock().await;
    let hash = worm.seal(&format!("BATTLE|{}", &prompt[..prompt.len().min(32)]));

    Json(serde_json::json!({
        "prompt": prompt,
        "frankenstein": frank_res.unwrap_or_else(|e| format!("ERROR: {}", e)),
        "opus":         opus_res.unwrap_or_else(|e| format!("ERROR: {}", e)),
        "devstral":     devstral_res.unwrap_or_else(|e| format!("ERROR: {}", e)),
        "worm": { "hash": hash, "seals": worm.count },
    }))
}

// ── /ask — sovereign query (no code path) ────────────────────────────────────
async fn ask_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskRequest>,
) -> Json<serde_json::Value> {
    let query = req.query.trim().to_string();
    let system = inject::trust_deed::build_system_prompt(None);

    let tavily = tools::tavily::search(&state.http, &query).await;
    let search_ctx = match &tavily {
        Ok(t) => tools::tavily::format_results(t),
        Err(_) => String::new(),
    };

    let msg = format!("QUERY: {}\n\nSEARCH CONTEXT:\n{}", query, search_ctx);
    let resp = brain(&state.bedrock, &system, &msg, 1024).await;

    let mut worm = state.worm.lock().await;
    let hash = worm.seal("ASK");

    Json(serde_json::json!({
        "query": query,
        "tavily": tavily.ok().and_then(|t| t.answer),
        "response": resp.unwrap_or_else(|e| format!("ERROR: {}", e)),
        "worm": { "hash": hash, "seals": worm.count },
    }))
}

async fn worm_handler(State(state): State<Arc<AppState>>) -> Json<WormStatus> {
    let worm = state.worm.lock().await;
    Json(WormStatus { hash: worm.prev[..16].to_string(), seals: worm.count })
}

async fn index() -> Html<&'static str> {
    Html(FRANKENSTEIN_HTML)
}

// ── Main ──────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::from_filename(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env")
    );

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1"))
        .load()
        .await;

    let state = Arc::new(AppState {
        bedrock: aws_sdk_bedrockruntime::Client::new(&aws_cfg),
        http:    reqwest::Client::new(),
        worm:    Mutex::new(WormChain::new()),
    });

    let app = Router::new()
        .route("/",          get(index))
        .route("/workflow",  post(workflow_handler))
        .route("/battle",    post(battle_handler))
        .route("/ask",       post(ask_handler))
        .route("/worm",      get(worm_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    println!("⬡ FRANKENSTEIN → http://localhost:{}", PORT);
    println!("  BRAIN:  {}", BRAIN_MODEL);
    println!("  LEGS:   {} ({})", LEGS_MODEL, VLLM_URL);
    println!("  HANDS:  pgvector ({})", PGVECTOR_URL);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", PORT)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── HTML UI ───────────────────────────────────────────────────────────────────
const FRANKENSTEIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>FRANKENSTEIN — BOB Workflow</title>
<style>
:root {
  --green:  #00ff41;
  --amber:  #ffb700;
  --blue:   #00cfff;
  --purple: #bf5fff;
  --red:    #ff3131;
  --bg:     #080808;
  --card:   #0e0e0e;
  --border: #1c1c1c;
  --dim:    #333;
}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--green);font-family:'Courier New',monospace;min-height:100vh;padding:1.2rem}
header{border-bottom:1px solid var(--green);padding-bottom:1rem;margin-bottom:1.5rem}
h1{font-size:2rem;letter-spacing:.15em}
h1 span{color:var(--amber)}
.sub{font-size:.7rem;color:var(--dim);margin-top:.3rem;letter-spacing:.05em}

/* WORM bar */
.wormbar{display:flex;align-items:center;gap:1rem;font-size:.72rem;color:var(--dim);
  border:1px solid #111;padding:.4rem .8rem;background:#0a0a0a;margin-bottom:1.5rem}
.wormbar .h{color:var(--amber);font-weight:700}

/* Tabs */
.tabs{display:flex;gap:.5rem;margin-bottom:1.5rem}
.tab{padding:.45rem 1.1rem;border:1px solid #1a1a1a;background:#0a0a0a;color:var(--dim);
  cursor:pointer;font-family:inherit;font-size:.78rem;letter-spacing:.05em;transition:all .15s}
.tab.active{border-color:var(--green);color:var(--green)}
.panel{display:none}.panel.active{display:block}

/* Input */
.row{display:flex;gap:.5rem;margin-bottom:1rem}
input{flex:1;background:#0a0a0a;border:1px solid #1a1a1a;color:var(--green);
  font-family:inherit;font-size:.9rem;padding:.6rem 1rem;outline:none}
input:focus{border-color:var(--green)}
input::placeholder{color:#222}
button{background:transparent;border:1px solid var(--green);color:var(--green);
  font-family:inherit;font-size:.82rem;padding:.6rem 1.2rem;cursor:pointer;
  letter-spacing:.05em;transition:all .15s;white-space:nowrap}
button:hover{background:var(--green);color:#000}
button:disabled{opacity:.3;cursor:default}

/* Workflow pipeline */
.pipeline{display:grid;grid-template-columns:repeat(4,1fr);gap:.75rem;margin-bottom:1rem}
.pipe-step{border:1px solid var(--border);background:var(--card);padding:.8rem}
.pipe-step h3{font-size:.68rem;letter-spacing:.1em;margin-bottom:.4rem;padding-bottom:.3rem;border-bottom:1px solid var(--border)}
.pipe-step.brain h3{color:var(--green)}
.pipe-step.hands h3{color:var(--blue)}
.pipe-step.legs  h3{color:var(--amber)}
.pipe-step.review h3{color:var(--purple)}
.pipe-body{font-size:.72rem;line-height:1.5;color:#bbb;white-space:pre-wrap;min-height:120px;max-height:320px;overflow-y:auto}
.pipe-meta{font-size:.62rem;color:var(--dim);margin-top:.4rem;border-top:1px solid var(--border);padding-top:.3rem}
.pipe-worm{color:var(--amber)}
.pipe-step.active h3{animation:pulse 1s infinite}
@keyframes pulse{50%{opacity:.4}}

/* Final output */
.final-out{background:#0a0a0a;border:1px solid var(--border);padding:1rem;
  font-size:.8rem;line-height:1.6;color:#ddd;white-space:pre-wrap;
  min-height:80px;margin-top:1rem;max-height:400px;overflow-y:auto}

/* Battle grid */
.battle-grid{display:grid;grid-template-columns:1fr 1fr 1fr;gap:.75rem;margin-top:1rem}
.fighter{border:1px solid var(--border);background:var(--card);padding:.9rem}
.fighter h3{font-size:.68rem;letter-spacing:.1em;border-bottom:1px solid var(--border);padding-bottom:.4rem;margin-bottom:.6rem}
.fighter.frank h3{color:var(--green)}
.fighter.opus  h3{color:var(--amber)}
.fighter.dev   h3{color:var(--blue)}
.fighter-body{font-size:.72rem;line-height:1.5;color:#ccc;white-space:pre-wrap;min-height:200px;max-height:500px;overflow-y:auto}

/* Ask output */
.ask-out{background:#0a0a0a;border:1px solid var(--border);padding:1rem;
  font-size:.8rem;line-height:1.6;white-space:pre-wrap;min-height:160px;max-height:500px;overflow-y:auto}

/* Badges */
.badge{display:inline-block;padding:.1rem .4rem;font-size:.6rem;border:1px solid;margin-right:.3rem}
.badge.code{border-color:var(--amber);color:var(--amber)}
.badge.text{border-color:var(--dim);color:var(--dim)}

@media(max-width:900px){
  .pipeline{grid-template-columns:1fr 1fr}
  .battle-grid{grid-template-columns:1fr}
}
</style>
</head>
<body>

<header>
  <h1>⬡ FRANK<span>STEIN</span></h1>
  <div class="sub">
    🧠 BRAIN: Sonnet 4.6 (Bedrock) &nbsp;·&nbsp;
    🤲 HANDS: pgvector corpus &nbsp;·&nbsp;
    🦵 LEGS: Granite vLLM (local, free) &nbsp;·&nbsp;
    Ω WORM sealed &nbsp;·&nbsp;
    Powered by BOB &nbsp;·&nbsp; SnapKitty Collective 2026
  </div>
</header>

<div class="wormbar">
  Ω WORM &nbsp;<span class="h" id="worm-hash">────────────────</span>
  &nbsp;|&nbsp; seals: <span id="worm-seals">0</span>
  &nbsp;|&nbsp; <span id="worm-status">BOOT</span>
</div>

<div class="tabs">
  <button class="tab active" onclick="sw('workflow')">⬡ WORKFLOW</button>
  <button class="tab"        onclick="sw('battle')">⚔ CODE BATTLE</button>
  <button class="tab"        onclick="sw('ask')">ASK</button>
</div>

<!-- ── WORKFLOW PANEL ────────────────────────────────────────── -->
<div id="panel-workflow" class="panel active">
  <div class="row">
    <input id="wf-input" placeholder="prompt frankenstein with anything — code path auto-detected..."
      onkeydown="if(event.key==='Enter') fireWorkflow()">
    <button id="wf-btn" onclick="fireWorkflow()">⬡ RUN WORKFLOW →</button>
  </div>

  <!-- Pipeline steps — always visible -->
  <div class="pipeline">
    <div class="pipe-step brain" id="step-brain">
      <h3>🧠 BRAIN — Claude Sonnet 4.6</h3>
      <div class="pipe-body" id="brain-body">waiting...</div>
      <div class="pipe-meta">
        Bedrock &nbsp;|&nbsp; Trust Deed injected &nbsp;|&nbsp; Ω <span class="pipe-worm" id="brain-worm">─</span> &nbsp;|&nbsp; <span id="brain-ms">─</span>ms
      </div>
    </div>
    <div class="pipe-step hands" id="step-hands">
      <h3>🤲 HANDS — pgvector corpus</h3>
      <div class="pipe-body" id="hands-body">waiting...</div>
      <div class="pipe-meta">
        Sovereign corpus &nbsp;|&nbsp; Ω <span class="pipe-worm" id="hands-worm">─</span> &nbsp;|&nbsp; <span id="hands-ms">─</span>ms
      </div>
    </div>
    <div class="pipe-step legs" id="step-legs">
      <h3>🦵 LEGS — Granite (vLLM local)</h3>
      <div class="pipe-body" id="legs-body">waiting... (only fires on code prompts)</div>
      <div class="pipe-meta">
        Tokio-spawned baby &nbsp;|&nbsp; Ω <span class="pipe-worm" id="legs-worm">─</span> &nbsp;|&nbsp; <span id="legs-ms">─</span>ms
      </div>
    </div>
    <div class="pipe-step review" id="step-review">
      <h3>🧠 BRAIN REVIEW — seal</h3>
      <div class="pipe-body" id="review-body">waiting...</div>
      <div class="pipe-meta">
        Merge + verdict &nbsp;|&nbsp; Ω <span class="pipe-worm" id="review-worm">─</span> &nbsp;|&nbsp; <span id="review-ms">─</span>ms
      </div>
    </div>
  </div>

  <div style="font-size:.72rem;color:var(--dim);margin:.5rem 0">
    FINAL OUTPUT <span id="code-badge"></span>
  </div>
  <div class="final-out" id="final-out">─ fire a prompt to see the full workflow ─</div>
</div>

<!-- ── BATTLE PANEL ───────────────────────────────────────────── -->
<div id="panel-battle" class="panel">
  <div class="row">
    <input id="battle-input" placeholder="write a function that..."
      onkeydown="if(event.key==='Enter') fireBattle()">
    <button id="battle-btn" onclick="fireBattle()">⚔ BATTLE →</button>
  </div>
  <div class="battle-grid">
    <div class="fighter frank">
      <h3>⬡ FRANKENSTEIN<br><small>Sonnet 4.6 + Trust Deed + Tavily</small></h3>
      <div class="fighter-body" id="frank-out">─</div>
    </div>
    <div class="fighter opus">
      <h3>Ω OPUS 4.6<br><small>Raw Claude — no stack</small></h3>
      <div class="fighter-body" id="opus-out">─</div>
    </div>
    <div class="fighter dev">
      <h3>Φ DEVSTRAL 123B<br><small>Mistral code beast (Bedrock)</small></h3>
      <div class="fighter-body" id="devstral-out">─</div>
    </div>
  </div>
</div>

<!-- ── ASK PANEL ─────────────────────────────────────────────── -->
<div id="panel-ask" class="panel">
  <div class="row">
    <input id="ask-input" placeholder="ask frankenstein anything..."
      onkeydown="if(event.key==='Enter') fireAsk()">
    <button id="ask-btn" onclick="fireAsk()">FIRE →</button>
  </div>
  <div class="ask-out" id="ask-out">─ sovereign query ready ─</div>
</div>

<script>
function sw(name){
  ['workflow','battle','ask'].forEach((n,i)=>{
    document.querySelectorAll('.tab')[i].classList.toggle('active',n===name)
    document.getElementById('panel-'+n).classList.toggle('active',n===name)
  })
}

function esc(s){ return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;') }

function updateWorm(w){
  if(!w) return
  document.getElementById('worm-hash').textContent = w.hash||'─'
  document.getElementById('worm-seals').textContent = w.seals||0
  document.getElementById('worm-status').textContent = 'SEALED Ω'
}

async function fireWorkflow(){
  const p = document.getElementById('wf-input').value.trim()
  if(!p) return
  const btn = document.getElementById('wf-btn')
  btn.disabled = true

  // Reset all steps
  ;['brain','hands','legs','review'].forEach(k=>{
    document.getElementById(k+'-body').textContent = 'running...'
    document.getElementById(k+'-worm').textContent = '─'
    document.getElementById(k+'-ms').textContent = '─'
    document.getElementById('step-'+k).classList.add('active')
  })
  document.getElementById('final-out').textContent = '─ processing ─'
  document.getElementById('code-badge').innerHTML = ''

  try {
    const res = await fetch('/workflow',{
      method:'POST',
      headers:{'Content-Type':'application/json'},
      body: JSON.stringify({prompt:p})
    })
    const d = await res.json()

    // Fill each step
    d.steps.forEach(s=>{
      const key = s.step.toLowerCase()
      const body = document.getElementById(key+'-body')
      const wormEl = document.getElementById(key+'-worm')
      const msEl   = document.getElementById(key+'-ms')
      if(body) body.textContent = s.output
      if(wormEl) wormEl.textContent = s.worm
      if(msEl)   msEl.textContent   = s.ms
      document.getElementById('step-'+key)?.classList.remove('active')
    })

    document.getElementById('final-out').textContent = d.final_out || '─'
    document.getElementById('code-badge').innerHTML =
      d.is_code
        ? '<span class="badge code">CODE PATH — Granite legs fired</span>'
        : '<span class="badge text">TEXT PATH — Brain only</span>'

    updateWorm(d.worm)
  } catch(e){
    document.getElementById('final-out').textContent = 'ERROR: '+e.message
  }
  btn.disabled = false
}

async function fireBattle(){
  const p = document.getElementById('battle-input').value.trim()
  if(!p) return
  const btn = document.getElementById('battle-btn')
  btn.disabled = true
  ;['frank-out','opus-out','devstral-out'].forEach(id=>{
    document.getElementById(id).textContent = 'reasoning...▋'
  })
  try {
    const res = await fetch('/battle',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({prompt:p})})
    const d = await res.json()
    document.getElementById('frank-out').textContent    = d.frankenstein||'─'
    document.getElementById('opus-out').textContent     = d.opus||'─'
    document.getElementById('devstral-out').textContent = d.devstral||'─'
    updateWorm(d.worm)
  } catch(e){ ['frank-out','opus-out','devstral-out'].forEach(id=>{ document.getElementById(id).textContent='ERROR: '+e.message }) }
  btn.disabled = false
}

async function fireAsk(){
  const q = document.getElementById('ask-input').value.trim()
  if(!q) return
  const btn = document.getElementById('ask-btn')
  btn.disabled = true
  document.getElementById('ask-out').textContent = 'searching + reasoning...'
  try {
    const res = await fetch('/ask',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({query:q})})
    const d = await res.json()
    let out = ''
    if(d.tavily) out += '📡 TAVILY: '+d.tavily+'\n\n'
    out += d.response||'─'
    out += '\n\nΩ WORM: '+(d.worm?.hash||'─')
    document.getElementById('ask-out').textContent = out
    updateWorm(d.worm)
  } catch(e){ document.getElementById('ask-out').textContent='ERROR: '+e.message }
  btn.disabled = false
}

setInterval(async()=>{ try{ const r=await fetch('/worm'); updateWorm(await r.json()) }catch(_){} },8000)
</script>
</body>
</html>"#;
