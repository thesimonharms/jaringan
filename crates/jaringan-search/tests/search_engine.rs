/// Integration test for the search engine.
/// Starts the server in a thread, tests all endpoints, then shuts down.
use std::net::TcpListener;
use std::sync::Arc;

use jaringan_protocol::{JaringanUrl, StatusCode};

use jaringan_search::SearchEngine;

fn get(base: &str, path: &str) -> String {
    let url = JaringanUrl::parse(&format!("{base}{path}")).unwrap();
    let resp = jaringan_protocol::fetch_tcp(&url).expect("fetch failed");
    assert_eq!(resp.status, StatusCode::Ok, "GET {path} returned non-200");
    resp.body
}

fn post(base: &str, path: &str, body: &str) -> String {
    let url = JaringanUrl::parse(&format!("{base}{path}")).unwrap();
    let resp = jaringan_protocol::post_tcp(&url, body.into()).expect("post failed");
    assert_eq!(resp.status, StatusCode::Ok, "POST {path} returned non-200");
    resp.body
}

#[test]
fn test_search_engine_endpoints() {
    // Start server on a random port
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let tmp = std::env::temp_dir().join(format!("jrg-search-test-{port}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let engine = Arc::new(SearchEngine::new(tmp.clone(), "search.test".into(), port));

    // Serve in a background thread
    let engine_clone = engine.clone();
    let _handle = std::thread::spawn(move || {
        let _ = jaringan_protocol::serve(listener, engine_clone);
    });

    // Give it a moment to start
    std::thread::sleep(std::time::Duration::from_millis(200));

    let base = format!("jrg://127.0.0.1:{port}");

    // 1. GET / - Search form
    let root = get(&base, "/");
    assert!(root.contains("JRG Search"), "Root should contain 'JRG Search'");
    assert!(root.contains("?q"), "Root should have search input");
    assert!(root.contains("!do-search"), "Root should have search button");
    println!("✓ GET / - Search form");

    // 2. GET /search.jrg - Same as root
    let search_page = get(&base, "/search.jrg");
    assert!(search_page.contains("JRG Search"));
    println!("✓ GET /search.jrg - Same as root");

    // 3. GET /submit.jrg - Submission form
    let submit_page = get(&base, "/submit.jrg");
    assert!(submit_page.contains("Submit Your Site"));
    assert!(submit_page.contains("Ed25519-signed"));
    assert!(submit_page.contains("xchacha20poly1305"));
    assert!(submit_page.contains("?domain"), "Submit form should have domain input");
    assert!(submit_page.contains("!submit"), "Submit form should have submit button");
    println!("✓ GET /submit.jrg - Submission form with requirements");

    // 4. GET /status.jrg - Status page
    let status = get(&base, "/status.jrg");
    assert!(status.contains("Status"), "Status page should contain 'Status'");
    assert!(status.contains("**0**"), "Should show 0 pages indexed");
    println!("✓ GET /status.jrg - Status page");

    // 5. GET 404
    let url = JaringanUrl::parse(&format!("{base}/nonexistent.jrg")).unwrap();
    let resp = jaringan_protocol::fetch_tcp(&url).expect("fetch failed");
    assert_eq!(resp.status, StatusCode::NotFound, "Non-existent path should return 404");
    assert!(resp.body.contains("Not Found"));
    println!("✓ GET /nonexistent.jrg - 404 handling");

    // 6. POST /actions/submit with domain
    let submit_result = post(&base, "/actions/submit", "domain=example.com");
    assert!(
        submit_result.contains("DNS Verification"),
        "Submit should show DNS verification instructions. Got: {}",
        &submit_result[..100.min(submit_result.len())]
    );
    assert!(
        submit_result.contains("_jrg-verify.example.com"),
        "Should include DNS record name"
    );
    assert!(submit_result.contains("value"), "Should show the verification value");
    println!("✓ POST /actions/submit - Domain submission generates verification token");

    // 7. POST /actions/submit with invalid domain
    let bad_submit = post(&base, "/actions/submit", "domain=");
    assert!(
        bad_submit.contains("Error") || bad_submit.contains("Invalid"),
        "Empty domain should show error. Got: {}",
        &bad_submit[..100.min(bad_submit.len())]
    );
    println!("✓ POST /actions/submit with empty domain - Error handling");

    // 8. POST /actions/search without query - should return search form
    let empty_search = post(&base, "/actions/search", "q=");
    assert!(empty_search.contains("Search"), "Empty search should show form");
    println!("✓ POST /actions/search empty - Returns search form");

    // 9. GET /actions/verify without domain - should show verify form
    let verify_form = get(&base, "/actions/verify");
    assert!(verify_form.contains("Verify"), "Verify page should mention Verify");
    println!("✓ GET /actions/verify - Shows verify form");

    println!("\n✅ All 9 endpoint tests passed!");
}

#[test]
fn validate_entry_rejects_wrong_length_keys() {
    use jaringan_search::engine::validate_entry;
    use jaringan_core::IndexEntryV1;

    // 0-byte key (empty base64)
    let entry_short = IndexEntryV1 {
        url: "jrg://example.com/p.jrg".into(),
        title: "Test".into(),
        public_key: "ed25519:".into(),
        encryption: "xchacha20poly1305;key-id=k1".into(),
        signature: "ed25519:".to_owned() + &"A".repeat(86), // 64 bytes base64
        last_modified: "2026-01-01T00:00:00Z".into(),
    };
    assert!(!validate_entry(&entry_short), "should reject empty public key");

    // 64-byte signature field empty
    let entry_bad_sig = IndexEntryV1 {
        url: "jrg://example.com/p.jrg".into(),
        title: "Test".into(),
        public_key: "ed25519:".to_owned() + &"A".repeat(43), // 32 bytes base64
        encryption: "xchacha20poly1305;key-id=k1".into(),
        signature: "ed25519:".into(),
        last_modified: "2026-01-01T00:00:00Z".into(),
    };
    assert!(!validate_entry(&entry_bad_sig), "should reject empty signature");
}

#[test]
fn test_verify_page_signature_crypto() {
    use ed25519_dalek::{Signer, SigningKey};
    use jaringan_core::IndexEntryV1;
    use jaringan_search::engine::verify_page_signature;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use base64::Engine;
    use rand::Rng;

    // 1. Generate an Ed25519 keypair
    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let pk_base64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.as_bytes());

    // 2. Build the page body (without signature) and sign it
    let signer_name = "test-signer";
    let page_body = "# Test Page\n\nThis is a signed page.\n\n=> jrg://example.com/ Home";
    // The canonical payload for signing uses ~~~~~ as metadata separator
    // canonical_signature_payload produces: "{body}~~~~~\n{metadata_without_signature}\n"
    let metadata_without_sig = format!("signed-by: {signer_name}");
    let canonical_payload = format!("{page_body}~~~~~\n{metadata_without_sig}\n");
    let signature = signing_key.sign(canonical_payload.as_bytes());
    let sig_base64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    // Full page text including the signature in metadata
    let page_text = format!(
        "{page_body}~~~~~\nsigned-by: {signer_name}\nsignature: ed25519:{sig_base64}\n"
    );

    // 3. Start a simple HTTP server to serve the page
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let page_text_clone = page_text.clone();

    let _server = thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        page_text_clone.len(),
                        page_text_clone
                    );
                    let _ = s.write_all(response.as_bytes());
                }
                Err(_) => break,
            }
        }
    });

    // Give the server a moment
    thread::sleep(std::time::Duration::from_millis(100));

    // 4. Create an IndexEntryV1 with the CORRECT public key
    let valid_entry = IndexEntryV1 {
        url: format!("jrg://127.0.0.1:{port}/test.jrg"),
        title: "Test Page".into(),
        public_key: format!("ed25519:{pk_base64}"),
        encryption: "xchacha20poly1305;key-id=k1".into(),
        signature: "ed25519:".to_owned() + &"A".repeat(86),
        last_modified: "2026-01-01T00:00:00Z".into(),
    };

    // 5. verify_page_signature should succeed with the correct key
    let result = verify_page_signature(&valid_entry);
    assert!(
        result.is_ok(),
        "verify_page_signature should succeed with correct public key. Got: {:?}",
        result
    );
    println!("✓ verify_page_signature succeeds with correct public key");

    // 6. Create an IndexEntryV1 with a WRONG public key
    let mut wrong_seed = [0u8; 32];
    rand::thread_rng().fill(&mut wrong_seed);
    let wrong_key = SigningKey::from_bytes(&wrong_seed);
    let wrong_pk_b64 =
        base64::engine::general_purpose::STANDARD.encode(wrong_key.verifying_key().as_bytes());

    let invalid_entry = IndexEntryV1 {
        url: format!("jrg://127.0.0.1:{port}/test.jrg"),
        title: "Test Page".into(),
        public_key: format!("ed25519:{wrong_pk_b64}"),
        encryption: "xchacha20poly1305;key-id=k1".into(),
        signature: "ed25519:".to_owned() + &"A".repeat(86),
        last_modified: "2026-01-01T00:00:00Z".into(),
    };

    // 7. verify_page_signature should fail with a different public key
    let result = verify_page_signature(&invalid_entry);
    assert!(
        result.is_err(),
        "verify_page_signature should fail with wrong public key"
    );
    println!(
        "✓ verify_page_signature rejects wrong public key: {}",
        result.unwrap_err()
    );
}
