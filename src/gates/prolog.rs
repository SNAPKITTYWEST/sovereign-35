use tokio::process::Command;

pub async fn constrain(_query: &str, facts: &[&str]) -> String {
    // Build Prolog constraint program from query + facts
    let mut program = String::new();
    program.push_str(":- initialization(main, main).\n\n");

    // Encode facts
    for fact in facts {
        let safe = fact.replace(' ', "_").replace('"', "").to_lowercase();
        program.push_str(&format!("fact({}).\n", safe));
    }

    // Sovereign constraints
    program.push_str(r#"
% Sovereign constraints
sovereign(X) :- fact(X).
evidence(X)  :- fact(X), X \= unknown.
silence      :- \+ fact(_).

main :-
    (fact(_) ->
        write('PROLOG:EVIDENCE'), nl
    ;
        write('PROLOG:SILENCE'), nl
    ),
    halt.
"#);

    let tmp = std::env::temp_dir().join("sovereign_prolog.pl");
    if tokio::fs::write(&tmp, &program).await.is_err() {
        return "PROLOG: unavailable (write error)".to_string();
    }

    match Command::new("swipl")
        .arg("-g").arg("main")
        .arg("-t").arg("halt")
        .arg(tmp.to_str().unwrap_or(""))
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.contains("EVIDENCE") {
                format!("Λ PROLOG CONSTRAINT: EVIDENCE — facts satisfy constraint\n{}", stdout)
            } else if stdout.contains("SILENCE") {
                "Λ PROLOG CONSTRAINT: SILENCE — no facts provided".to_string()
            } else {
                format!("Λ PROLOG CONSTRAINT: {}", stdout)
            }
        }
        Err(_) => "Λ PROLOG: UNAVAILABLE (install SWI-Prolog: swipl)".to_string(),
    }
}
