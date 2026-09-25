//! Local control-API authorization.
//!
//! The node's HTTP port serves two audiences: remote peers (`/api/p2p/*`) and the
//! local dashboard (everything else). Local endpoints must be protected against
//! other processes' web pages (CSRF), DNS rebinding and remote callers, so every
//! local request must come from loopback, carry a loopback `Host`, carry no
//! foreign `Origin`, and present the per-run token.

use rand::RngCore;

pub const TOKEN_HEADER: &str = "x-aether-token";
const TOKEN_META_PLACEHOLDER: &str = "__AETHER_TOKEN__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// Missing or wrong token.
    Unauthorized,
    /// Request did not come from this machine, or Host/Origin is foreign.
    Forbidden,
}

impl Denial {
    pub fn status_line(self) -> &'static str {
        match self {
            Denial::Unauthorized => "401 Unauthorized",
            Denial::Forbidden => "403 Forbidden",
        }
    }
}

pub struct ApiGuard {
    token: String,
    port: u16,
}

impl ApiGuard {
    pub fn new(token: String, port: u16) -> Self {
        ApiGuard { token, port }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Host must name this node on loopback. Blocks DNS-rebinding attacks.
    pub fn host_allowed(&self, host: Option<&str>) -> bool {
        let Some(host) = host else { return false };
        let host = host.trim().to_ascii_lowercase();
        [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
            format!("[::1]:{}", self.port),
        ]
        .iter()
        .any(|allowed| *allowed == host)
    }

    /// Absent Origin (same-origin GET, curl) is fine; a present Origin must be ours.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        let Some(origin) = origin else { return true };
        let origin = origin.trim().to_ascii_lowercase();
        [
            format!("http://127.0.0.1:{}", self.port),
            format!("http://localhost:{}", self.port),
            format!("http://[::1]:{}", self.port),
        ]
        .iter()
        .any(|allowed| *allowed == origin)
    }

    pub fn token_ok(&self, provided: Option<&str>) -> bool {
        match provided {
            Some(p) => constant_time_eq(p.trim().as_bytes(), self.token.as_bytes()),
            None => false,
        }
    }

    /// Authorize a local-control request (dashboard API).
    pub fn authorize_local(&self, request: &str, from_loopback: bool) -> Result<(), Denial> {
        if !from_loopback
            || !self.host_allowed(header(request, "host"))
            || !self.origin_allowed(header(request, "origin"))
        {
            return Err(Denial::Forbidden);
        }
        if !self.token_ok(header(request, TOKEN_HEADER)) {
            return Err(Denial::Unauthorized);
        }
        Ok(())
    }

    /// Authorize the dashboard page itself (no token yet: the page carries it).
    pub fn authorize_page(&self, request: &str, from_loopback: bool) -> Result<(), Denial> {
        if from_loopback && self.host_allowed(header(request, "host")) {
            Ok(())
        } else {
            Err(Denial::Forbidden)
        }
    }

    /// Put the token into the dashboard's `<meta name="aether-token">` tag.
    pub fn inject_token(&self, html: &str) -> String {
        html.replace(TOKEN_META_PLACEHOLDER, &self.token)
    }
}

/// Case-insensitive header lookup in a raw HTTP request (headers section only).
pub fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let head = request.split("\r\n\r\n").next().unwrap_or("");
    head.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    crate::types::hex::encode(&bytes)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> ApiGuard {
        ApiGuard::new("secret".to_string(), 8080)
    }

    fn req(headers: &[(&str, &str)]) -> String {
        let mut r = String::from("POST /api/tx HTTP/1.1\r\n");
        for (k, v) in headers {
            r.push_str(&format!("{}: {}\r\n", k, v));
        }
        r.push_str("\r\n{}");
        r
    }

    #[test]
    fn accepts_loopback_request_with_token() {
        let r = req(&[("Host", "127.0.0.1:8080"), ("X-Aether-Token", "secret")]);
        assert_eq!(guard().authorize_local(&r, true), Ok(()));
    }

    #[test]
    fn rejects_missing_or_wrong_token() {
        let g = guard();
        let r = req(&[("Host", "127.0.0.1:8080")]);
        assert_eq!(g.authorize_local(&r, true), Err(Denial::Unauthorized));
        let r = req(&[("Host", "127.0.0.1:8080"), ("X-Aether-Token", "wrong!")]);
        assert_eq!(g.authorize_local(&r, true), Err(Denial::Unauthorized));
    }

    #[test]
    fn rejects_non_loopback_peer() {
        let r = req(&[("Host", "127.0.0.1:8080"), ("X-Aether-Token", "secret")]);
        assert_eq!(guard().authorize_local(&r, false), Err(Denial::Forbidden));
    }

    #[test]
    fn rejects_foreign_origin_csrf() {
        let r = req(&[
            ("Host", "127.0.0.1:8080"),
            ("Origin", "https://evil.example"),
            ("X-Aether-Token", "secret"),
        ]);
        assert_eq!(guard().authorize_local(&r, true), Err(Denial::Forbidden));
    }

    #[test]
    fn rejects_dns_rebinding_host() {
        let r = req(&[("Host", "evil.example:8080"), ("X-Aether-Token", "secret")]);
        assert_eq!(guard().authorize_local(&r, true), Err(Denial::Forbidden));
        let r = req(&[("X-Aether-Token", "secret")]);
        assert_eq!(guard().authorize_local(&r, true), Err(Denial::Forbidden));
    }

    #[test]
    fn accepts_own_origin_and_localhost_forms() {
        let g = guard();
        for (host, origin) in [
            ("localhost:8080", "http://localhost:8080"),
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("[::1]:8080", "http://[::1]:8080"),
        ] {
            let r = req(&[("Host", host), ("Origin", origin), ("X-Aether-Token", "secret")]);
            assert_eq!(g.authorize_local(&r, true), Ok(()), "{host}");
        }
    }

    #[test]
    fn header_lookup_ignores_body() {
        let r = "GET / HTTP/1.1\r\nHost: a\r\n\r\nOrigin: evil";
        assert_eq!(header(r, "origin"), None);
        assert_eq!(header(r, "HOST"), Some("a"));
    }

    #[test]
    fn page_requires_loopback_host() {
        let g = guard();
        assert!(g.authorize_page("GET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n", true).is_ok());
        assert!(g.authorize_page("GET / HTTP/1.1\r\nHost: evil:8080\r\n\r\n", true).is_err());
        assert!(g.authorize_page("GET / HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n", false).is_err());
    }

    #[test]
    fn token_is_random_hex() {
        let (a, b) = (generate_token(), generate_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
