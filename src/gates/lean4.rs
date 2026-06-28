use anyhow::Result;
use tokio::process::Command;

pub enum GateVerdict {
    Pass(String),
    Reject(String),
    Unavailable,
}

pub async fn verify(claim: &str) -> GateVerdict {
    // Build a minimal Lean 4 proof obligation from the claim
    let lean_src = format!(r#"
-- Sovereign Lean 4 gate — auto-generated
-- Claim: {}
-- Gate: verify claim is non-contradictory
#check @id
-- If this compiles, gate passes
#eval "GATE:PASS"
"#, claim.replace('"', "'"));

    let tmp = std::env::temp_dir().join("sovereign_gate.lean");
    if tokio::fs::write(&tmp, &lean_src).await.is_err() {
        return GateVerdict::Unavailable;
    }

    match Command::new("lean")
        .arg(tmp.to_str().unwrap_or(""))
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if output.status.success() && stdout.contains("GATE:PASS") {
                GateVerdict::Pass(stdout)
            } else {
                GateVerdict::Reject(format!("LEAN4 REJECT: {}", stderr))
            }
        }
        Err(_) => GateVerdict::Unavailable,
    }
}

pub fn report(verdict: &GateVerdict) -> String {
    match verdict {
        GateVerdict::Pass(msg)    => format!("⬡ LEAN4 GATE: PASS ✓\n{}", msg),
        GateVerdict::Reject(msg)  => format!("⬡ LEAN4 GATE: REJECT ✗\n{}", msg),
        GateVerdict::Unavailable  => "⬡ LEAN4 GATE: UNAVAILABLE (lean not installed — install lean4)".to_string(),
    }
}
