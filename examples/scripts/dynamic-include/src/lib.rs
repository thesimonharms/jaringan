#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use jaringan_script_sdk as sdk;

fn script_main(input: &str) -> String {
    // ── 1. Parse input ──────────────────────────────────────────────
    let input_val: sdk::serde_json::Value =
        sdk::serde_json::from_str(input).unwrap_or(sdk::serde_json::Value::Null);

    let blocks = input_val["blocks"].as_array().cloned().unwrap_or_default();
    let mut output_blocks: Vec<sdk::serde_json::Value> = Vec::new();

    // ── 2. Process each block ───────────────────────────────────────
    for block in &blocks {
        let block_type = block["type"].as_str().unwrap_or("");
        let text = block["text"].as_str().unwrap_or("");

        // Look for an include directive in paragraph blocks
        if block_type == "paragraph" {
            if let Some(include_url) = text.strip_prefix("include: ") {
                // Fetch the included page
                sdk::js_log("info", &format!("fetching include: {include_url}"));

                match sdk::js_fetch(include_url) {
                    Ok(response_json) => {
                        let resp: sdk::serde_json::Value =
                            sdk::serde_json::from_str(&response_json)
                                .unwrap_or(sdk::serde_json::Value::Null);

                        if resp["error"].is_null() {
                            // Parse the response body as a ScriptInput JSON
                            // The body contains the JRG page content
                            let body = resp["body"].as_str().unwrap_or("");

                            // Try to parse the body as a ScriptInput with blocks
                            let body_val: sdk::serde_json::Value =
                                sdk::serde_json::from_str(body)
                                    .unwrap_or(sdk::serde_json::Value::Null);

                            if let Some(included_blocks) = body_val["blocks"].as_array() {
                                sdk::js_log("info", &format!("included {} blocks", included_blocks.len()));
                                // Add a heading before included content
                                output_blocks.push(sdk::serde_json::json!({
                                    "type": "heading",
                                    "level": 3,
                                    "text": alloc::format!("↳ Included from {include_url}")
                                }));
                                // Insert all included blocks
                                for ib in included_blocks {
                                    output_blocks.push(ib.clone());
                                }
                            } else {
                                // Plain text fallback
                                output_blocks.push(sdk::serde_json::json!({
                                    "type": "preformatted",
                                    "text": body,
                                    "language": null
                                }));
                            }
                        } else {
                            output_blocks.push(sdk::serde_json::json!({
                                "type": "paragraph",
                                "text": alloc::format!("[Include error: {}]", resp["error"])
                            }));
                        }
                    }
                    Err(e) => {
                        output_blocks.push(sdk::serde_json::json!({
                            "type": "paragraph",
                            "text": alloc::format!("[Fetch error: {e}]")
                        }));
                    }
                }
            } else {
                output_blocks.push(block.clone());
            }
        } else {
            output_blocks.push(block.clone());
        }
    }

    // ── 3. Return output JSON ───────────────────────────────────────
    let output = sdk::serde_json::json!({ "blocks": output_blocks });
    sdk::serde_json::to_string(&output).unwrap_or_default()
}

sdk::export_process!(script_main);
