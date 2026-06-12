#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use jaringan_script_sdk as sdk;

fn script_main(input: &str) -> String {
    let input_val: sdk::serde_json::Value =
        sdk::serde_json::from_str(input).unwrap_or(sdk::serde_json::Value::Null);

    let mut blocks: Vec<sdk::serde_json::Value> = Vec::new();
    let _title = input_val["title"].as_str().unwrap_or("Form Processor");

    // Get form inputs
    let inputs = input_val["inputs"].as_array().cloned().unwrap_or_default();

    // Include any existing blocks first (the form UI)
    if let Some(existing) = input_val["blocks"].as_array() {
        for b in existing {
            blocks.push(b.clone());
        }
    }

    // ── Validate inputs ────────────────────────────────────────────
    let mut errors: Vec<String> = Vec::new();
    let mut url_params: Vec<String> = Vec::new();

    for input_field in &inputs {
        let name = input_field["name"].as_str().unwrap_or("");
        let value = input_field["value"].as_str().unwrap_or("");
        let label = input_field["label"].as_str().unwrap_or(name);

        url_params.push(format!("{}={}", name, value));

        if value.is_empty() {
            errors.push(format!("{label} is required"));
        }
    }

    if !errors.is_empty() {
        // Show validation errors
        blocks.push(sdk::serde_json::json!({
            "type": "heading", "level": 2, "text": "Validation Errors"
        }));
        for err in &errors {
            blocks.push(sdk::serde_json::json!({
                "type": "paragraph",
                "text": format!("⚠ {err}")
            }));
        }
        let output = sdk::serde_json::json!({ "blocks": blocks });
        return sdk::serde_json::to_string(&output).unwrap_or_default();
    }

    // ── Simulate form submission via fetch ──────────────────────────
    let _query_string = url_params.join("&");
    let submit_url = format!("https://httpbin.org/post");
    sdk::js_log("info", &format!("submitting to {submit_url}"));

    match sdk::js_fetch(&submit_url) {
        Ok(response_json) => {
            let resp: sdk::serde_json::Value =
                sdk::serde_json::from_str(&response_json)
                    .unwrap_or(sdk::serde_json::Value::Null);

            if resp["error"].is_null() {
                let status = resp["status"].as_u64().unwrap_or(0);

                blocks.push(sdk::serde_json::json!({
                    "type": "heading", "level": 2, "text": "Submission Result"
                }));
                blocks.push(sdk::serde_json::json!({
                    "type": "paragraph",
                    "text": format!("✓ Form submitted successfully (HTTP {status})")
                }));
                blocks.push(sdk::serde_json::json!({
                    "type": "preformatted",
                    "language": Some("json"),
                    "text": response_json
                }));
            } else {
                blocks.push(sdk::serde_json::json!({
                    "type": "heading", "level": 2, "text": "Submission Failed"
                }));
                blocks.push(sdk::serde_json::json!({
                    "type": "paragraph",
                    "text": format!("✗ {}", resp["error"])
                }));
            }
        }
        Err(e) => {
            blocks.push(sdk::serde_json::json!({
                "type": "heading", "level": 2, "text": "Network Error"
            }));
            blocks.push(sdk::serde_json::json!({
                "type": "paragraph",
                "text": format!("✗ {e}")
            }));
        }
    }

    let output = sdk::serde_json::json!({ "blocks": blocks });
    sdk::serde_json::to_string(&output).unwrap_or_default()
}

sdk::export_process!(script_main);
