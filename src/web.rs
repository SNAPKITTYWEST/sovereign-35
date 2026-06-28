// FRANKENSTEIN — Live web UI for Sovereign 3.5
// CODE BATTLE: Frankenstein vs Opus 4.8 vs Devstral 123B
// All via AWS Bedrock · Tokio async · SSE streaming

mod inject { pub mod trust_deed; }
mod tools  { pub mod tavily; pub mod ddg; }
mod gates  { pub mod lean4; pub mod prolog; }
mod models { pub mod bedrock; pub mod vllm; }

use anyhow::Result;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, Response, Sse},
    response::sse::Event,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

const PORT: u16 = 4300;

// ── Shared state ─────────────────────────────────────────────
struct AppState {
    bedrock: aws_sdk_bedrockruntime::Client,
    http:    reqwest::Client,
    worm:    Mutex<WormChain>,
}

struct WormChain { prev: String, count: u64 }

impl WormChain {
    fn new() -> Self { Self { prev: "FRANKENSTEIN_BOOT".to_string(), count: 0 } }
    fn seal(&mut self, event: &str) -> String {
        let msg = format!("{}|{}|{}", self.prev, event, chrono::Utc::now().timestamp_millis());
        let hash = hex::encode(Sha256::digest(msg.as_bytes()));
        self.prev = hash.clone();
        self.count += 1;
        hash[..16].to_string()
    }
}

// ── Request/Response types ────────────────────────────────────
#[derive(Deserialize)]
struct AskRequest { query: String }

#[derive(Deserialize)]
struct BattleRequest { prompt: String }

#[derive(Serialize)]
struct WormStatus { hash: String, seals: u64 }

// ── Bedrock invoke helper ─────────────────────────────────────
async fn bedrock_raw(
    client:    &aws_sdk_bedrockruntime::Client,
    model_id:  &str,
    system:    &str,
    user_msg:  &str,
) -> Result<String> {
    models::bedrock::invoke_with_model(client, model_id, system, &[("user", user_msg)], 2048).await
}

// ── Routes ────────────────────────────────────────────────────
async fn index() -> Html<&'static str> {
    Html(FRANKENSTEIN_HTML)
}

async fn ask_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskRequest>,
) -> Json<serde_json::Value> {
    let query = req.query.trim().to_string();
    let mut out = serde_json::json!({ "query": query, "steps": [] });

    // Tavily
    let tavily = tools::tavily::search(&state.http, &query).await;
    let search_ctx = match &tavily {
        Ok(t) => {
            let f = tools::tavily::format_results(t);
            out["tavily"] = serde_json::json!(t.answer);
            f
        }
        Err(e) => { out["tavily_err"] = serde_json::json!(e.to_string()); String::new() }
    };

    // Gate
    let lean = gates::lean4::verify(&query).await;
    out["lean4"] = serde_json::json!(gates::lean4::report(&lean));

    // Prolog
    let facts: Vec<&str> = query.split_whitespace().collect();
    let prolog = gates::prolog::constrain(&query, &facts).await;
    out["prolog"] = serde_json::json!(prolog);

    // Bedrock — Frankenstein
    let system = inject::trust_deed::build_system_prompt(None);
    let user_msg = format!(
        "QUERY: {}\n\nSEARCH CONTEXT:\n{}\n\nAnswer using Evidence or Silence protocol.",
        query, search_ctx
    );
    match bedrock_raw(&state.bedrock, "us.anthropic.claude-sonnet-4-6", &system, &user_msg).await {
        Ok(r) => { out["response"] = serde_json::json!(r); }
        Err(e) => { out["bedrock_err"] = serde_json::json!(e.to_string()); }
    }

    // WORM
    let mut worm = state.worm.lock().await;
    let hash = worm.seal(&format!("ASK|{}", &query[..query.len().min(32)]));
    out["worm"] = serde_json::json!({ "hash": hash, "seals": worm.count });

    Json(out)
}

async fn battle_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BattleRequest>,
) -> Json<serde_json::Value> {
    let prompt = req.prompt.trim().to_string();
    let system_frank = inject::trust_deed::build_system_prompt(None);
    let system_raw   = "You are a helpful AI assistant. Answer the coding question directly.";

    // Fire all three concurrently
    let (frank_res, opus_res, devstral_res) = tokio::join!(
        bedrock_raw(&state.bedrock, "us.anthropic.claude-sonnet-4-6", &system_frank, &prompt),
        bedrock_raw(&state.bedrock, "us.anthropic.claude-opus-4-6-v1", system_raw,    &prompt),
        bedrock_raw(&state.bedrock, "mistral.devstral-2-123b",        system_raw,    &prompt),
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

async fn worm_handler(State(state): State<Arc<AppState>>) -> Json<WormStatus> {
    let worm = state.worm.lock().await;
    Json(WormStatus { hash: worm.prev[..16].to_string(), seals: worm.count })
}

// ── Main ─────────────────────────────────────────────────────
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
        .route("/",        get(index))
        .route("/ask",     post(ask_handler))
        .route("/battle",  post(battle_handler))
        .route("/worm",    get(worm_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", PORT);
    println!("⬡ FRANKENSTEIN live → http://localhost:{}", PORT);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── HTML UI ───────────────────────────────────────────────────
const FRANKENSTEIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>FRANKENSTEIN — Sovereign AI</title>
<style>
  :root {
    --green:  #00ff41;
    --amber:  #ffb700;
    --red:    #ff3131;
    --blue:   #00cfff;
    --purple: #bf5fff;
    --bg:     #0a0a0a;
    --card:   #111;
    --border: #1e1e1e;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    background: var(--bg);
    color: var(--green);
    font-family: 'Courier New', monospace;
    min-height: 100vh;
    padding: 1rem;
  }
  header {
    border-bottom: 1px solid var(--green);
    padding-bottom: 1rem;
    margin-bottom: 1.5rem;
  }
  header h1 { font-size: 2rem; letter-spacing: 0.1em; }
  header h1 span { color: var(--amber); }
  .tag { font-size: 0.7rem; color: #444; margin-top: 0.25rem; }
  .worm-bar {
    display: flex; align-items: center; gap: 1rem;
    font-size: 0.75rem; color: #444;
    border: 1px solid #1a1a1a;
    padding: 0.4rem 0.75rem;
    background: #0d0d0d;
    margin-bottom: 1.5rem;
  }
  .worm-bar .hash { color: var(--amber); font-weight: bold; }
  .tabs {
    display: flex; gap: 0.5rem; margin-bottom: 1.5rem;
  }
  .tab {
    padding: 0.5rem 1.25rem;
    border: 1px solid #222;
    background: #0d0d0d;
    color: #444;
    cursor: pointer;
    font-family: inherit;
    font-size: 0.8rem;
    letter-spacing: 0.05em;
    transition: all 0.15s;
  }
  .tab.active { border-color: var(--green); color: var(--green); }
  .panel { display: none; }
  .panel.active { display: block; }
  .input-row {
    display: flex; gap: 0.5rem; margin-bottom: 1rem;
  }
  input[type=text] {
    flex: 1;
    background: #0d0d0d;
    border: 1px solid #222;
    color: var(--green);
    font-family: inherit;
    font-size: 0.9rem;
    padding: 0.6rem 1rem;
    outline: none;
  }
  input[type=text]:focus { border-color: var(--green); }
  input[type=text]::placeholder { color: #333; }
  button {
    background: transparent;
    border: 1px solid var(--green);
    color: var(--green);
    font-family: inherit;
    font-size: 0.85rem;
    padding: 0.6rem 1.25rem;
    cursor: pointer;
    letter-spacing: 0.05em;
    transition: all 0.15s;
  }
  button:hover { background: var(--green); color: #000; }
  button:disabled { opacity: 0.3; cursor: default; }
  .output {
    background: #0d0d0d;
    border: 1px solid #1a1a1a;
    padding: 1rem;
    min-height: 200px;
    white-space: pre-wrap;
    font-size: 0.85rem;
    line-height: 1.6;
    overflow-x: auto;
  }
  .step { margin-bottom: 0.5rem; }
  .step.tavily  { color: var(--blue); }
  .step.lean4   { color: var(--purple); }
  .step.prolog  { color: var(--purple); }
  .step.worm    { color: var(--amber); }
  .step.response { color: #eee; }
  .step.err     { color: var(--red); }
  /* Battle */
  .battle-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 1rem;
    margin-top: 1rem;
  }
  .fighter {
    border: 1px solid #1a1a1a;
    background: #0d0d0d;
    padding: 1rem;
  }
  .fighter h3 {
    font-size: 0.75rem;
    letter-spacing: 0.1em;
    border-bottom: 1px solid #1a1a1a;
    padding-bottom: 0.5rem;
    margin-bottom: 0.75rem;
  }
  .fighter.frank h3 { color: var(--green); }
  .fighter.opus  h3 { color: var(--amber); }
  .fighter.dev   h3 { color: var(--blue); }
  .fighter-body {
    white-space: pre-wrap;
    font-size: 0.78rem;
    line-height: 1.55;
    color: #ccc;
    min-height: 300px;
  }
  .spinner { color: #333; }
  @keyframes blink { 50% { opacity: 0; } }
  .cursor { animation: blink 1s step-end infinite; }
  @media (max-width: 768px) {
    .battle-grid { grid-template-columns: 1fr; }
  }
</style>
</head>
<body>

<header>
  <h1>⬡ FRANK<span>STEIN</span></h1>
  <div class="tag">
    Claude Sonnet 4.6 · Opus 4.8 · Devstral 123B · Tavily · Lean4 · Prolog · WORM
    &nbsp;|&nbsp; Apache 2.0 · SnapKitty Collective 2026
  </div>
</header>

<div class="worm-bar">
  Ω WORM &nbsp;
  <span class="hash" id="worm-hash">────────────────</span>
  &nbsp;|&nbsp; seals: <span id="worm-seals">0</span>
  &nbsp;|&nbsp; <span id="worm-status">BOOT</span>
</div>

<div class="tabs">
  <button class="tab active" onclick="switchTab('ask')">ASK FRANKENSTEIN</button>
  <button class="tab"        onclick="switchTab('battle')">⚔ CODE BATTLE</button>
</div>

<!-- ASK PANEL -->
<div id="panel-ask" class="panel active">
  <div class="input-row">
    <input type="text" id="ask-input" placeholder="ask frankenstein anything..."
      onkeydown="if(event.key==='Enter') fireAsk()">
    <button id="ask-btn" onclick="fireAsk()">FIRE →</button>
  </div>
  <div class="output" id="ask-output">
    <span class="step tavily">⬡ Frankenstein is ready. Fire a query.</span>
  </div>
</div>

<!-- BATTLE PANEL -->
<div id="panel-battle" class="panel">
  <div class="input-row">
    <input type="text" id="battle-input"
      placeholder="write a function to..."
      onkeydown="if(event.key==='Enter') fireBattle()">
    <button id="battle-btn" onclick="fireBattle()">⚔ BATTLE →</button>
  </div>
  <div class="battle-grid">
    <div class="fighter frank">
      <h3>⬡ FRANKENSTEIN<br><small>Sonnet 4.6 + Tavily + Trust Deed</small></h3>
      <div class="fighter-body" id="frank-out">waiting...</div>
    </div>
    <div class="fighter opus">
      <h3>Ω OPUS 4.6<br><small>Raw top-tier Claude</small></h3>
      <div class="fighter-body" id="opus-out">waiting...</div>
    </div>
    <div class="fighter dev">
      <h3>Φ DEVSTRAL 123B<br><small>Mistral code beast (Bedrock)</small></h3>
      <div class="fighter-body" id="devstral-out">waiting...</div>
    </div>
  </div>
</div>

<script>
function switchTab(name) {
  document.querySelectorAll('.tab').forEach((t,i) => {
    t.classList.toggle('active', ['ask','battle'][i] === name);
  });
  document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
  document.getElementById('panel-' + name).classList.add('active');
}

function updateWorm(w) {
  if (!w) return;
  document.getElementById('worm-hash').textContent = w.hash || w.prev?.slice(0,16) || '────────────────';
  document.getElementById('worm-seals').textContent = w.seals || 0;
  document.getElementById('worm-status').textContent = 'SEALED Ω';
}

async function fireAsk() {
  const q = document.getElementById('ask-input').value.trim();
  if (!q) return;
  const btn = document.getElementById('ask-btn');
  const out = document.getElementById('ask-output');
  btn.disabled = true;
  out.innerHTML = '<span class="step tavily">⬡ FIRING: ' + q + '</span>\n<span class="spinner">searching + gating + reasoning...</span><span class="cursor">█</span>';

  try {
    const res = await fetch('/ask', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query: q })
    });
    const data = await res.json();
    let html = '';

    if (data.tavily)    html += '<span class="step tavily">📡 TAVILY: ' + esc(data.tavily) + '</span>\n\n';
    if (data.tavily_err) html += '<span class="step err">TAVILY: ' + esc(data.tavily_err) + '</span>\n\n';
    if (data.lean4)     html += '<span class="step lean4">' + esc(data.lean4) + '</span>\n\n';
    if (data.prolog)    html += '<span class="step prolog">' + esc(data.prolog) + '</span>\n\n';

    html += '<span class="step response">─── SOVEREIGN RESPONSE ───\n\n' + esc(data.response || data.bedrock_err || 'no response') + '</span>\n\n';

    if (data.worm) {
      html += '<span class="step worm">Ω WORM: ' + data.worm.hash + ' | seals: ' + data.worm.seals + '</span>';
      updateWorm(data.worm);
    }

    out.innerHTML = html;
  } catch(e) {
    out.innerHTML = '<span class="step err">ERROR: ' + e.message + '</span>';
  }
  btn.disabled = false;
}

async function fireBattle() {
  const p = document.getElementById('battle-input').value.trim();
  if (!p) return;
  const btn = document.getElementById('battle-btn');
  btn.disabled = true;

  const ids = ['frank-out', 'opus-out', 'devstral-out'];
  ids.forEach(id => {
    document.getElementById(id).textContent = 'reasoning...▋';
  });

  try {
    const res = await fetch('/battle', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ prompt: p })
    });
    const data = await res.json();

    document.getElementById('frank-out').textContent = data.frankenstein || 'no response';
    document.getElementById('opus-out').textContent = data.opus || 'no response';
    document.getElementById('devstral-out').textContent = data.devstral || 'no response';
    if (data.worm) updateWorm(data.worm);
  } catch(e) {
    ids.forEach(id => { document.getElementById(id).textContent = 'ERROR: ' + e.message; });
  }
  btn.disabled = false;
}

function esc(s) {
  return String(s)
    .replace(/&/g,'&amp;')
    .replace(/</g,'&lt;')
    .replace(/>/g,'&gt;');
}

// Poll WORM every 10s
setInterval(async () => {
  try {
    const r = await fetch('/worm');
    updateWorm(await r.json());
  } catch(_) {}
}, 10000);
</script>
</body>
</html>"#;
