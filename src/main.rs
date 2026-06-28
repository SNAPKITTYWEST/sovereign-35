// SOVEREIGN 3.5
// Claude 3.5 Bedrock + Tavily + DuckDuckGo + Lean4 Gate + Prolog + vLLM
// Tokio async fire — SnapKitty Collective 2026 — Apache 2.0

mod inject { pub mod trust_deed; }
mod tools  { pub mod tavily; pub mod ddg; }
mod gates  { pub mod lean4; pub mod prolog; }
mod models { pub mod bedrock; pub mod vllm; }

use anyhow::Result;
use colored::Colorize;
use sha2::{Sha256, Digest};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};

// ── WORM chain ──────────────────────────────────────────────
struct Worm { prev: String, count: u64 }

impl Worm {
    fn new() -> Self { Self { prev: "GENESIS_SOVEREIGN_35".to_string(), count: 0 } }

    fn seal(&mut self, event: &str) -> String {
        let msg = format!("{}|{}|{}", self.prev, event, chrono::Utc::now().timestamp_millis());
        let hash = hex::encode(Sha256::digest(msg.as_bytes()));
        self.prev = hash.clone();
        self.count += 1;
        hash
    }
}

// ── Agent turn ───────────────────────────────────────────────
async fn run_turn(
    query:   &str,
    worm:    &mut Worm,
    http:    &reqwest::Client,
    bedrock: &aws_sdk_bedrockruntime::Client,
) -> Result<()> {
    let t0 = Instant::now();
    println!("\n{}", "═".repeat(60).dimmed());
    println!("{} {}", "⬡ QUERY:".yellow().bold(), query);

    let is_code = query.to_lowercase().contains("code")
        || query.to_lowercase().contains("write")
        || query.to_lowercase().contains("implement")
        || query.to_lowercase().contains("function")
        || query.to_lowercase().contains("build");

    // ── Fire all tasks concurrently (Tokio) ─────────────────
    let (tavily_res, ddg_res, lean_res) = tokio::join!(
        tools::tavily::search(http, query),
        tools::ddg::search(http, query),
        gates::lean4::verify(query),
    );

    // ── Lean 4 gate report ────────────────────────────────────
    let gate_report = gates::lean4::report(&lean_res);
    println!("{}", gate_report.cyan().dimmed());
    worm.seal(&format!("LEAN4|{}", matches!(lean_res, gates::lean4::GateVerdict::Pass(_))));

    // ── Search context ────────────────────────────────────────
    let mut search_ctx = String::new();
    match tavily_res {
        Ok(t) => {
            let formatted = tools::tavily::format_results(&t);
            print!("{}", "📡 TAVILY: ".green());
            println!("{}", t.answer.as_deref().unwrap_or("searching...").dimmed());
            search_ctx.push_str(&formatted);
            worm.seal("TAVILY:OK");
        }
        Err(e) => {
            println!("{} {}", "TAVILY ERR:".red(), e);
            worm.seal("TAVILY:ERR");
        }
    }

    match ddg_res {
        Ok(d) => {
            if !d.contains("no results") {
                print!("{}", "🦆 DDG:    ".blue());
                println!("{}", d.lines().next().unwrap_or("").dimmed());
                search_ctx.push_str(&d);
                worm.seal("DDG:OK");
            }
        }
        Err(_) => { worm.seal("DDG:ERR"); }
    }

    // ── Prolog constraint ─────────────────────────────────────
    let facts: Vec<&str> = query.split_whitespace().collect();
    let prolog_out = gates::prolog::constrain(query, &facts).await;
    println!("{}", prolog_out.purple().dimmed());
    worm.seal("PROLOG");

    // ── Code path → vLLM ────────────────────────────────────
    if is_code {
        println!("{}", "⚡ CODE PATH → vLLM coder".cyan());
        let code = models::vllm::complete_code(
            http,
            &format!("// Task: {}\n// Write clean, documented code:\n\n", query),
            "Qwen/Qwen2.5-Coder-7B-Instruct",
        ).await?;
        println!("{}\n{}", "vLLM OUTPUT:".cyan().bold(), code.white());
        worm.seal("VLLM:CODE");
    }

    // ── Bedrock Claude 3.5 ────────────────────────────────────
    let system = inject::trust_deed::build_system_prompt(None);

    let user_msg = format!(
        "QUERY: {}\n\nSEARCH CONTEXT:\n{}\n\nGATE: {}\n\nAnswer using Evidence or Silence protocol.",
        query, search_ctx, gate_report
    );

    println!("\n{}", "🧠 CLAUDE 3.5 BEDROCK...".yellow());
    match models::bedrock::invoke(bedrock, &system, &[("user", &user_msg)], 1024).await {
        Ok(response) => {
            println!("\n{}", "─ SOVEREIGN RESPONSE ─".yellow().bold());
            println!("{}", response.white());
            worm.seal(&format!("BEDROCK|{}", &response[..response.len().min(32)]));
        }
        Err(e) => {
            println!("{} {}", "BEDROCK ERR:".red(), e);
            println!("{}", "Tip: check AWS region — Claude 3.5 needs us-east-1 or us-west-2".dimmed());
            worm.seal("BEDROCK:ERR");
        }
    }

    let elapsed = t0.elapsed();
    println!(
        "\n{} Ω {} | seals: {} | {}ms",
        "WORM:".dimmed(),
        &worm.prev[..16].yellow(),
        worm.count.to_string().dimmed(),
        elapsed.as_millis().to_string().dimmed()
    );

    Ok(())
}

// ── Main ─────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> Result<()> {
    // Load .env
    let _ = dotenvy::from_filename(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env")
    );

    println!("{}", r#"
  ╔══════════════════════════════════════════════════════════╗
  ║  SOVEREIGN 3.5                                           ║
  ║  Claude 3.5 Bedrock · Tavily · DDG · Lean4 · Prolog      ║
  ║  vLLM Coder · Tokio Async Fire · WORM Sealed             ║
  ║  Apache 2.0 · SnapKitty Collective 2026                  ║
  ╚══════════════════════════════════════════════════════════╝
    "#.cyan());

    // AWS Bedrock client (uses ~/.aws/credentials)
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1"))
        .load()
        .await;
    let bedrock = aws_sdk_bedrockruntime::Client::new(&aws_cfg);
    let http    = reqwest::Client::new();
    let mut worm = Worm::new();

    worm.seal("SOVEREIGN_35_BOOT");
    println!("{} Ω {}", "WORM:".dimmed(), &worm.prev[..16].yellow());

    // ── REPL ────────────────────────────────────────────────
    println!("\n{}", "Type your query (Ctrl+C to exit):".dimmed());

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    loop {
        print!("{} ", "SOVEREIGN>".yellow().bold());
        use std::io::Write;
        std::io::stdout().flush()?;

        match lines.next_line().await? {
            None => break,
            Some(line) => {
                let q = line.trim().to_string();
                if q.is_empty() { continue; }
                if q == "exit" || q == "quit" { break; }
                if let Err(e) = run_turn(&q, &mut worm, &http, &bedrock).await {
                    println!("{} {}", "ERROR:".red(), e);
                }
            }
        }
    }

    println!("\n{} Ω {} | total seals: {}", "FINAL WORM:".yellow(), &worm.prev[..16], worm.count);
    Ok(())
}
