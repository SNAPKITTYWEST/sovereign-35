use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct TavilyRequest {
    api_key:        String,
    query:          String,
    search_depth:   String,
    max_results:    u8,
    include_answer: bool,
}

#[derive(Deserialize, Debug)]
pub struct TavilyResult {
    pub title:   String,
    pub url:     String,
    pub content: String,
    pub score:   f32,
}

#[derive(Deserialize, Debug)]
pub struct TavilyResponse {
    pub answer:  Option<String>,
    pub results: Vec<TavilyResult>,
}

pub async fn search(client: &reqwest::Client, query: &str) -> Result<TavilyResponse> {
    let api_key = std::env::var("TAVILY_API_KEY")?;
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&TavilyRequest {
            api_key,
            query:          query.to_string(),
            search_depth:   "advanced".to_string(),
            max_results:    5,
            include_answer: true,
        })
        .send()
        .await?
        .json::<TavilyResponse>()
        .await?;
    Ok(resp)
}

pub fn format_results(resp: &TavilyResponse) -> String {
    let mut out = String::new();
    if let Some(ans) = &resp.answer {
        out.push_str(&format!("TAVILY ANSWER: {}\n\n", ans));
    }
    for (i, r) in resp.results.iter().enumerate() {
        out.push_str(&format!(
            "[{}] {} (score: {:.2})\n{}\n{}\n\n",
            i + 1, r.title, r.score, r.url, r.content
        ));
    }
    out
}
