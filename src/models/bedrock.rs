use anyhow::Result;
use aws_sdk_bedrockruntime::{Client, primitives::Blob};
use serde_json::{json, Value};

pub const MODEL_ID: &str = "us.anthropic.claude-sonnet-4-6";

pub async fn invoke(
    client: &Client,
    system:   &str,
    messages: &[(&str, &str)],  // (role, content)
    max_tokens: u32,
) -> Result<String> {
    let msgs: Vec<Value> = messages.iter().map(|(role, content)| {
        json!({ "role": role, "content": content })
    }).collect();

    let body = json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": max_tokens,
        "system": system,
        "messages": msgs,
    });

    let resp = client
        .invoke_model()
        .model_id(MODEL_ID)
        .content_type("application/json")
        .body(Blob::new(serde_json::to_vec(&body)?))
        .send()
        .await?;

    let resp_body: Value = serde_json::from_slice(resp.body().as_ref())?;
    let text = resp_body["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    Ok(text)
}
