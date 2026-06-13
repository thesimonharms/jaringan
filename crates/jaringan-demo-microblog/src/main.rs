use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jaringan_protocol::{
    PageResolver, Request, RequestMethod, ResolveError, Response, ResponseTag, StatusCode,
};

// ---------------------------------------------------------------------------
// Page templates (embedded as const strings)
// ---------------------------------------------------------------------------

const REGISTER_JRG: &str = r#"# 📡 Microblog — Sign Up

Enter a username to register.

?username label="Username" placeholder="Choose a username"
!register label="📝 Sign Up" target="/actions/register" method="POST" auth="microblog"

---

=> /microblog Already have an account? View the feed

~~~
title: Register — Microblog Demo
~~~"#;

const MICROBLOG_JRG: &str = r#"# 📡 Microblog Demo

A live demo of Jaringan's native auth system.  
Sign up with `jaringan auth register localhost:{PORT} -f username=YOUR_NAME`  
then post messages.

@{AUTH_SERVICE} scope="post read" ttl="{TTL}"

---

## Post a message

?content label="" placeholder="What's happening?"
!post label="📤 Post" target="/actions/post" method="POST" auth="microblog"

---

## 📰 Feed

{FEED}

---

**Built with Jaringan** — terminal-native, AI-friendly protocol.
=> https://github.com/thesimonharms/jaringan ⚡ Get Jaringan on GitHub

~~~
title: Microblog Demo
~~~"#;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct Post {
    username: String,
    content: String,
    timestamp: Instant,
}

#[derive(Clone)]
struct TokenInfo {
    username: String,
    expires_at: Instant,
}

struct MicroblogResolver {
    posts: Mutex<Vec<Post>>,
    tokens: Mutex<HashMap<String, TokenInfo>>,
    port: u16,
}

impl MicroblogResolver {
    fn new(port: u16) -> Self {
        Self {
            posts: Mutex::new(Vec::new()),
            tokens: Mutex::new(HashMap::new()),
            port,
        }
    }

    /// Check if we're at :00 of the current hour — if so, wipe all posts and tokens.
    fn check_wipe(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let minute_of_hour = (now % 3600) / 60;
        let second_of_minute = now % 60;
        if minute_of_hour == 0 && second_of_minute < 5 {
            self.posts.lock().unwrap().clear();
            self.tokens.lock().unwrap().clear();
        }
    }

    fn render_feed(&self) -> String {
        let posts = self.posts.lock().unwrap();
        if posts.is_empty() {
            return "Nothing yet — be the first to post!".to_string();
        }
        let mut feed = String::new();
        for (i, post) in posts.iter().enumerate() {
            let secs_ago = post.timestamp.elapsed().as_secs();
            let time_str = if secs_ago < 60 {
                "just now".into()
            } else {
                format!("{}m ago", secs_ago / 60)
            };
            feed.push_str(&format!("### **{}** — {}\n\n", post.username, time_str));
            feed.push_str(&format!("{}\n\n---\n\n", post.content));
            if i >= 20 {
                break;
            }
        }
        feed
    }

    fn microblog_page(&self) -> String {
        let feed = self.render_feed();
        MICROBLOG_JRG
            .replace("{PORT}", &self.port.to_string())
            .replace("{AUTH_SERVICE}", &format!("microblog.localhost:{}", self.port))
            .replace("{TTL}", "session")
            .replace("{FEED}", &feed)
    }

    fn handle_register(&self, body: &str) -> Response {
        // Generate random hex token (16 bytes → 32 hex chars)
        let token_bytes: [u8; 16] = rand::random();
        let token = hex::encode(token_bytes);

        let username = parse_form_value(body, "username").unwrap_or("anonymous");

        // Store token with 1-hour expiry
        let expires = Instant::now() + Duration::from_secs(3600);
        self.tokens.lock().unwrap().insert(
            token.clone(),
            TokenInfo {
                username: username.to_string(),
                expires_at: expires,
            },
        );

        let body = format!(
            "# ✅ Registered!\n\nYou're signed in as **{}**.\n\n=> /microblog View the feed\n\n~~~\ntitle: Registered\n~~~",
            username
        );

        Response::page(StatusCode::Ok, body).with_tag(ResponseTag::Token {
            service: format!("microblog.localhost:{}", self.port),
            value: token,
            expires_at: None, // session — no explicit expiry in tag
        })
    }

    fn handle_post(&self, request: &Request) -> Response {
        // Try to get the auth token: first from request.action_token (wire-level),
        // then from form body fields
        let token = request
            .action_token
            .as_deref()
            .or_else(|| parse_form_value(&request.body, "action_token"))
            .unwrap_or("")
            .to_string();

        // Validate token
        let tokens = self.tokens.lock().unwrap();
        let token_info = tokens.get(&token).cloned();
        drop(tokens);

        match token_info {
            Some(info) if info.expires_at > Instant::now() => {
                let content = parse_form_value(&request.body, "content").unwrap_or("");

                // Store post
                let mut posts = self.posts.lock().unwrap();
                posts.insert(
                    0,
                    Post {
                        username: info.username,
                        content: content.to_string(),
                        timestamp: Instant::now(),
                    },
                );
                if posts.len() > 50 {
                    posts.truncate(50);
                }
                drop(posts);

                // Return updated page
                let page = self.microblog_page();
                Response::page(StatusCode::Ok, page)
            }
            _ => {
                // Token invalid — return page with error
                let mut page = self.microblog_page();
                page = format!(
                    "> ⚠️ **Auth required.** Register first with `jaringan auth register localhost:{} -f username=YOUR_NAME`\n\n{}",
                    self.port, page
                );
                Response::page(StatusCode::Ok, page)
            }
        }
    }
}

impl PageResolver for MicroblogResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        // 1. Hourly wipe
        self.check_wipe();

        // 2. Route by path + method
        let path = request.url.path();

        match (request.method, path) {
            // GET / or /microblog or /microblog.jrg
            (RequestMethod::Get, p)
                if p == "/" || p == "/microblog" || p == "/microblog.jrg" =>
            {
                Ok(Response::page(StatusCode::Ok, self.microblog_page()))
            }

            // GET /register or /register.jrg
            (RequestMethod::Get, p) if p == "/register" || p == "/register.jrg" => {
                Ok(Response::page(StatusCode::Ok, REGISTER_JRG.to_string()))
            }

            // POST /actions/register
            (RequestMethod::Post, "/actions/register") => {
                Ok(self.handle_register(&request.body))
            }

            // POST /actions/post
            (RequestMethod::Post, "/actions/post") => Ok(self.handle_post(request)),

            _ => Ok(Response::page(StatusCode::NotFound, "not found".to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_form_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    for pair in body.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        let v = parts.next()?;
        if k == key {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7072);

    let resolver = MicroblogResolver::new(port);
    let listener = std::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .expect("failed to bind");

    eprintln!("📡 Microblog demo listening on jrg://127.0.0.1:{port}");
    eprintln!("   Register:  jaringan auth register localhost:{port} -f username=YOUR_NAME");
    eprintln!("   View feed: jaringan get jrg://127.0.0.1:{port}/microblog");
    eprintln!("   HTTP:      curl http://localhost:18080/proxy/jrg://127.0.0.1:{port}/microblog");

    if let Err(e) = jaringan_protocol::serve(listener, resolver) {
        eprintln!("Server error: {e}");
    }
}
