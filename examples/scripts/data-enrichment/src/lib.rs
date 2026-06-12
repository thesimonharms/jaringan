#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::vec;
use alloc::format;
use jaringan_script_sdk as sdk;

fn script_main(input: &str) -> String {
    let input_val: sdk::serde_json::Value =
        sdk::serde_json::from_str(input).unwrap_or(sdk::serde_json::Value::Null);

    let mut blocks: Vec<sdk::serde_json::Value> = Vec::new();
    let title = input_val["title"].as_str().unwrap_or("Data Enrichment");
    let metadata = input_val["metadata"].as_object()
        .cloned()
        .unwrap_or_default();

    // Get the enrichment URL from metadata
    let enrich_url = metadata.get("enrich_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if enrich_url.is_empty() {
        blocks.push(sdk::serde_json::json!({
            "type": "heading", "level": 1, "text": title
        }));
        blocks.push(sdk::serde_json::json!({
            "type": "paragraph",
            "text": "No enrich_url found in page metadata. Add `~~~~~\nenrich_url: <url>` to the page."
        }));
        let output = sdk::serde_json::json!({ "blocks": blocks });
        return sdk::serde_json::to_string(&output).unwrap_or_default();
    }

    // Include existing blocks first
    if let Some(existing) = input_val["blocks"].as_array() {
        for b in existing {
            blocks.push(b.clone());
        }
    }

    // Fetch enrichment data
    sdk::js_log("info", &format!("enriching from: {enrich_url}"));

    match sdk::js_fetch(enrich_url) {
        Ok(response_json) => {
            let resp: sdk::serde_json::Value =
                sdk::serde_json::from_str(&response_json)
                    .unwrap_or(sdk::serde_json::Value::Null);

            if resp["error"].is_null() {
                let status = resp["status"].as_u64().unwrap_or(0);
                let body = resp["body"].as_str().unwrap_or("");

                // Try to parse body as JSON (API-style response)
                let data: sdk::serde_json::Value =
                    sdk::serde_json::from_str(body)
                        .unwrap_or(sdk::serde_json::Value::Null);

                blocks.push(sdk::serde_json::json!({
                    "type": "heading", "level": 2, "text": "Enriched Data"
                }));

                if data.is_object() {
                    // Render JSON keys as a table
                    let headers = vec!["Key".to_string(), "Value".to_string()];
                    let mut rows: Vec<Vec<String>> = Vec::new();
                    if let Some(obj) = data.as_object() {
                        for (key, val) in obj {
                            let val_str = match val {
                                sdk::serde_json::Value::String(s) => s.clone(),
                                v => sdk::serde_json::to_string(v).unwrap_or_default(),
                            };
                            // Truncate long values
                            let display = if val_str.len() > 60 {
                                format!("{}...", &val_str[..57])
                            } else {
                                val_str
                            };
                            rows.push(vec![key.clone(), display]);
                        }
                    }
                    blocks.push(sdk::serde_json::json!({
                        "type": "table",
                        "headers": headers,
                        "rows": rows
                    }));
                } else if data.is_array() {
                    blocks.push(sdk::serde_json::json!({
                        "type": "paragraph",
                        "text": format!("Received {} items", data.as_array().map(|a| a.len()).unwrap_or(0))
                    }));
                    // Show first 3 items
                    for (_i, item) in data.as_array().unwrap().iter().take(3).enumerate() {
                        blocks.push(sdk::serde_json::json!({
                            "type": "preformatted",
                            "language": Some("json"),
                            "text": sdk::serde_json::to_string_pretty(item).unwrap_or_default()
                        }));
                    }
                } else {
                    // Plain text
                    blocks.push(sdk::serde_json::json!({
                        "type": "preformatted",
                        "language": None::<&str>,
                        "text": body.to_string()
                    }));
                }

                blocks.push(sdk::serde_json::json!({
                    "type": "paragraph",
                    "text": format!("Fetched from {enrich_url} (status {status})")
                }));
            } else {
                blocks.push(sdk::serde_json::json!({
                    "type": "paragraph",
                    "text": format!("[Enrichment error: {}]", resp["error"])
                }));
            }
        }
        Err(e) => {
            blocks.push(sdk::serde_json::json!({
                "type": "paragraph",
                "text": format!("[Fetch error: {e}]")
            }));
        }
    }

    let output = sdk::serde_json::json!({ "blocks": blocks });
    sdk::serde_json::to_string(&output).unwrap_or_default()
}

sdk::export_process!(script_main);
