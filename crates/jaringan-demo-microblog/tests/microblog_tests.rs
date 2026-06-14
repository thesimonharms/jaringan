use jaringan_protocol::{JaringanUrl, PageResolver, Request, ResponseTag, StatusCode};

use jaringan_demo_microblog::MicroblogResolver;

#[test]
fn microblog_feed_page_served() {
    let resolver = MicroblogResolver::new(7072);

    // GET / - should serve the microblog feed page
    let req = Request::new(
        JaringanUrl::parse("jrg://127.0.0.1:7072/").unwrap(),
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);
    assert!(
        resp.body.contains("Microblog"),
        "Feed page should contain 'Microblog', got: {}",
        &resp.body[..200.min(resp.body.len())]
    );
}

#[test]
fn microblog_feed_via_microblog_path() {
    let resolver = MicroblogResolver::new(7072);

    // GET /microblog - should serve the feed page
    let req = Request::new(
        JaringanUrl::parse("jrg://127.0.0.1:7072/microblog").unwrap(),
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);
    assert!(resp.body.contains("Microblog"));
}

#[test]
fn microblog_register_form_served() {
    let resolver = MicroblogResolver::new(7072);

    // GET /register - should serve the registration form
    let req = Request::new(
        JaringanUrl::parse("jrg://127.0.0.1:7072/register").unwrap(),
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);
    assert!(
        resp.body.contains("Sign Up") || resp.body.contains("Register"),
        "Register page should contain 'Sign Up' or 'Register', got: {}",
        &resp.body[..200.min(resp.body.len())]
    );
}

#[test]
fn microblog_register_action_produces_token() {
    let resolver = MicroblogResolver::new(7072);

    // POST /actions/register with a username
    let req = Request::post(
        JaringanUrl::parse("jrg://127.0.0.1:7072/actions/register").unwrap(),
        "username=testuser",
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);

    // Should return a Token tag
    let has_token = resp
        .tags
        .iter()
        .any(|t| matches!(t, ResponseTag::Token { .. }));
    assert!(
        has_token,
        "Register should return a Tag-Token, got tags: {:?}",
        resp.tags
    );
}

#[test]
fn microblog_register_returns_success_message() {
    let resolver = MicroblogResolver::new(7072);

    let req = Request::post(
        JaringanUrl::parse("jrg://127.0.0.1:7072/actions/register").unwrap(),
        "username=alice",
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);
    assert!(
        resp.body.contains("Registered"),
        "Response should indicate registration succeeded, got: {}",
        &resp.body[..200.min(resp.body.len())]
    );
    assert!(
        resp.body.contains("alice"),
        "Response should contain the username 'alice', got: {}",
        &resp.body[..200.min(resp.body.len())]
    );
}

#[test]
fn microblog_post_requires_token() {
    let resolver = MicroblogResolver::new(7072);

    // POST /actions/post without a token — should show auth error
    let req = Request::post(
        JaringanUrl::parse("jrg://127.0.0.1:7072/actions/post").unwrap(),
        "content=Hello",
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::Ok);

    // Should contain an auth-required message
    assert!(
        resp.body.contains("Auth required")
            || resp.body.contains("Register")
            || resp.body.contains("auth"),
        "Response should indicate auth is required, got: {}",
        &resp.body[..300.min(resp.body.len())]
    );
}

#[test]
fn microblog_post_with_token_succeeds() {
    let resolver = MicroblogResolver::new(7072);

    // First register to get a token
    let register_req = Request::post(
        JaringanUrl::parse("jrg://127.0.0.1:7072/actions/register").unwrap(),
        "username=bob",
    );
    let register_resp = resolver.fetch(&register_req).unwrap();

    // Extract the token from the response
    let token = register_resp
        .tags
        .iter()
        .find_map(|t| {
            if let ResponseTag::Token { value, .. } = t {
                Some(value.clone())
            } else {
                None
            }
        })
        .expect("Register should return a Token tag");

    // Now post with the token
    let post_req = Request::post(
        JaringanUrl::parse("jrg://127.0.0.1:7072/actions/post").unwrap(),
        "content=Hello from Bob!",
    )
    .with_action_token(token);
    let post_resp = resolver.fetch(&post_req).unwrap();
    assert_eq!(post_resp.status, StatusCode::Ok);

    // Should contain the post content in the feed
    assert!(
        post_resp.body.contains("Hello from Bob"),
        "Post should appear in the feed, got: {}",
        &post_resp.body[..300.min(post_resp.body.len())]
    );
}

#[test]
fn microblog_404_for_unknown_path() {
    let resolver = MicroblogResolver::new(7072);

    let req = Request::new(
        JaringanUrl::parse("jrg://127.0.0.1:7072/doesnotexist").unwrap(),
    );
    let resp = resolver.fetch(&req).unwrap();
    assert_eq!(resp.status, StatusCode::NotFound);
    assert!(
        resp.body.contains("not found"),
        "404 page should say 'not found', got: {}",
        &resp.body[..100.min(resp.body.len())]
    );
}
