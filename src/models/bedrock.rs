use anyhow::Result;
use aws_sdk_bedrockruntime::{Client, primitives::Blob};
use serde_json::{json, Value};

pub const MODEL_ID: &str = "us.anthropic.claude-sonnet-4-6";

pub async fn invoke(
    client: &Client,
    system:   &str,
    messages: &[(&str, &str)],
    max_tokens: u32,
) -> Result<String> {
    invoke_with_model(client, MODEL_ID, system, messages, max_tokens).await
}

pub async fn invoke_with_model(
    client:    &Client,
    model_id:  &str,
    system:    &str,
    messages:  &[(&str, &str)],
    max_tokens: u32,
) -> Result<String> {
    let msgs: Vec<Value> = messages.iter().map(|(role, content)| {
        json!({ "role": role, "content": content })
    }).collect();

    // Mistral models use a different request format
    let is_mistral = model_id.starts_with("mistral.");
    let body = if is_mistral {
        // Devstral uses OpenAI-compatible messages format
        let mut mistral_msgs = vec![json!({"role": "system", "content": system})];
        for (role, content) in messages {
            mistral_msgs.push(json!({"role": role, "content": content}));
        }
        json!({
            "messages": mistral_msgs,
            "max_tokens": max_tokens,
            "temperature": 0.7,
        })
    } else {
        json!({
            "anthropic_version": "bedrock-2023-05-31",
            "max_tokens": max_tokens,
            "system": system,
            "messages": msgs,
        })
    };

    let resp = client
        .invoke_model()
        .model_id(model_id)
        .content_type("application/json")
        .body(Blob::new(serde_json::to_vec(&body)?))
        .send()
        .await?;

    let resp_body: Value = serde_json::from_slice(resp.body().as_ref())?;

    // Mistral returns choices[0].message.content, Claude returns content[0].text
    let text = if is_mistral {
        resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string()
    } else {
        resp_body["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };

    Ok(text)
}
