use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct DdgResult {
    #[serde(rename = "Text")]
    text:     Option<String>,
    #[serde(rename = "FirstURL")]
    first_url: Option<String>,
}

#[derive(Deserialize, Debug)]
struct DdgResponse {
    #[serde(rename = "AbstractText")]
    abstract_text: Option<String>,
    #[serde(rename = "RelatedTopics")]
    related_topics: Vec<DdgResult>,
}

pub async fn search(client: &reqwest::Client, query: &str) -> Result<String> {
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        urlencoding::encode(query)
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "sovereign-35/1.0")
        .send()
        .await?
        .json::<DdgResponse>()
        .await?;

    let mut out = String::new();
    if let Some(abs) = &resp.abstract_text {
        if !abs.is_empty() {
            out.push_str(&format!("DDG ABSTRACT: {}\n\n", abs));
        }
    }
    for (i, r) in resp.related_topics.iter().take(3).enumerate() {
        if let (Some(text), Some(url)) = (&r.text, &r.first_url) {
            out.push_str(&format!("[DDG {}] {}\n{}\n\n", i + 1, text, url));
        }
    }
    if out.is_empty() {
        out = "DDG: no results".to_string();
    }
    Ok(out)
}
