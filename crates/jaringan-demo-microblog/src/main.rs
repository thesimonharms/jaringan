use std::net::TcpListener;

use jaringan_demo_microblog::MicroblogResolver;
use jaringan_protocol::serve;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7072);

    let resolver = MicroblogResolver::new(port);
    let listener =
        TcpListener::bind(format!("127.0.0.1:{port}")).expect("failed to bind");

    eprintln!("📡 Microblog demo listening on jrg://127.0.0.1:{port}");
    eprintln!(
        "   Register:  jaringan auth register localhost:{port} -f username=YOUR_NAME"
    );
    eprintln!("   View feed: jaringan get jrg://127.0.0.1:{port}/microblog");
    eprintln!(
        "   HTTP:      curl http://localhost:18080/proxy/jrg://127.0.0.1:{port}/microblog"
    );

    if let Err(e) = serve(listener, resolver) {
        eprintln!("Server error: {e}");
    }
}
