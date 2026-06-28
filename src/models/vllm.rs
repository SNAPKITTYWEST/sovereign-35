use anyhow::Result;
use serde::{Deserialize, Serialize};

// vLLM OpenAI-compatible endpoint — default localhost:8000
// Start with: python -m vllm.entrypoints.openai.api_server \
//   --model Qwen/Qwen2.5-Coder-7B-Instruct --port 8000

const VLLM_URL: &str = "http://localhost:8000/v1/completions";

#[derive(Serialize)]
struct VllmRequest {
    model:       String,
    prompt:      String,
    max_tokens:  u32,
    temperature: f32,
    stop:        Vec<String>,
}

#[derive(Deserialize)]
struct VllmChoice {
    text: String,
}

#[derive(Deserialize)]
struct VllmResponse {
    choices: Vec<VllmChoice>,
}

pub async fn complete_code(
    client: &reqwest::Client,
    prompt: &str,
    model:  &str,
) -> Result<String> {
    let resp = client
        .post(VLLM_URL)
        .json(&VllmRequest {
            model:       model.to_string(),
            prompt:      prompt.to_string(),
            max_tokens:  512,
            temperature: 0.1,
            stop:        vec!["```".to_string(), "# END".to_string()],
        })
        .send()
        .await;

    match resp {
        Ok(r) => {
            let body = r.json::<VllmResponse>().await?;
            Ok(body.choices.first().map(|c| c.text.clone()).unwrap_or_default())
        }
        Err(e) => {
            Ok(format!(
                "vLLM OFFLINE — start with:\npython -m vllm.entrypoints.openai.api_server --model Qwen/Qwen2.5-Coder-7B-Instruct --port 8000\n\nError: {}",
                e
            ))
        }
    }
}
