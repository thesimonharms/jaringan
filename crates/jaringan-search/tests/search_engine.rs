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
