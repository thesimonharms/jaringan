// ---------------------------------------------------------------------------
// JRG page templates for the search engine
// ---------------------------------------------------------------------------

/// Main search form
pub const SEARCH_FORM: &str = r#"# 🔍 JRG Search

Find JRG pages across the network. All indexed pages are Ed25519-signed and encryption-capable.

?q label="Search query" placeholder="Search JRG pages..."
!do-search label="🔍 Search" method="POST" target="/actions/search"

---

=> /submit.jrg Submit Your Site
=> /status.jrg Index Status

~~~
title: JRG Search
~~~"#;

/// Domain submission form
pub const SUBMIT_FORM: &str = r#"# 📝 Submit Your Site

To get your JRG site indexed, enter your domain name below. You'll need to:

1. **Enter your domain** — e.g., `example.com`
2. **Create a DNS TXT record** at `_jrg-verify.<your-domain>` with the provided token
3. **Verify** — we'll check the DNS record to confirm ownership
4. **Publish `/.well-known/jrgidx`** — we'll fetch it and index your pages

## Requirements for indexed pages

- All pages must be **Ed25519-signed** with `signed-by:` and `signature:` metadata
- All pages must declare an **encryption capability** (`xchacha20poly1305`)
- Your server must serve a **`/.well-known/jrgidx`** file listing your pages

## Submit Your Domain

?domain label="Domain" placeholder="example.com"
!submit label="📤 Submit Domain" method="POST" target="/actions/submit"

---

=> / Back to Search

~~~
title: Submit Your Site
~~~"#;

/// Verification form (GET /actions/verify without domain param)
pub const VERIFY_FORM: &str = r#"# ✅ Verify Your Domain

Enter your domain to check verification status and complete the process.

?domain label="Domain" placeholder="example.com"
!check label="🔍 Check Verification" method="POST" target="/actions/check-verify"

---

=> /submit.jrg Submit Another Domain
=> / Back to Search

~~~
title: Verify Domain
~~~"#;

// ---------------------------------------------------------------------------
// Dynamic page builders
// ---------------------------------------------------------------------------

/// Show verification instructions after submission
pub fn verification_pending_page(domain: &str, token: &str, verify_record: &str) -> String {
    format!(
        r#"# 📝 Domain Submitted: {domain}

## Next Step: DNS Verification

Create a TXT record for `{verify_record}` with the following value:

> `{token}`

Once the record is live, click the button below to verify:

?domain label="Domain" value="{domain}" placeholder=""
!check label="✅ Verify DNS" method="POST" target="/actions/check-verify"

---

=> / Back to Search

~~~
title: Verification Pending — {domain}
~~~"#
    )
}

/// Show verify instructions (GET /actions/verify?domain=...)
pub fn verify_instruction_page(domain: &str, token: &str, verify_record: &str) -> String {
    format!(
        r#"# 📝 Verify: {domain}

Create a TXT record for `{verify_record}` with value:

> `{token}`

Then click below:

?domain label="Domain" value="{domain}" placeholder=""
!check label="✅ Verify DNS" method="POST" target="/actions/check-verify"

---

=> /submit.jrg Back to Submission

~~~
title: Verify {domain}
~~~"#
    )
}

/// Status page
pub fn status_page(total_pages: usize, verified_domains: usize, domain: &str, port: u16) -> String {
    format!(
        r#"# 📊 JRG Search Status

| Metric | Value |
|--------|-------|
| Indexed Pages | **{total_pages}** |
| Verified Domains | **{verified_domains}** |
| Engine | `{domain}` |
| Protocol | jrg://{domain}:{port} |

---

=> / Back to Search
=> /submit.jrg Submit Your Site

~~~
title: Search Status
~~~"#
    )
}

/// Show verified + indexed result page
pub fn verified_and_indexed_page(domain: &str, pages_count: usize, skipped: usize, indexed_at: &str) -> String {
    format!(
        r#"# ✅ Domain Verified and Indexed!

**{domain}** has been DNS-verified and indexed.

- Pages indexed: **{pages_count}**
- Entries skipped: **{skipped}** (invalid format)
- Indexed at: **{indexed_at}**

We'll re-check your site periodically for updates.

---

=> /search.jrg Search the Index
=> /status.jrg View Index Status

~~~
title: Indexed — {domain}
~~~"#
    )
}

/// Error page
pub fn error_page(message: &str) -> String {
    format!(
        r#"# ⚠️ Error

{message}

---

=> / Back to Search
=> /submit.jrg Submit Your Site

~~~
title: Error
~~~"#
    )
}

/// 404 page
pub fn not_found_page() -> String {
    r#"# 404 Not Found

The requested page was not found on this search engine.

=> / Back to Search
=> /submit.jrg Submit Your Site

~~~
title: Not Found
~~~"#
    .to_string()
}
