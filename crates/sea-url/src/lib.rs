//! Pure EastSea custom-link and browser-input grammar. Custom authorities
//! are inspected as raw bytes before a URL library can normalize them.
//! Classification never requests a payment, signature or network read.

extern crate alloc;

use alloc::{format, string::String};
use core::fmt;
use serde::Serialize;

pub const RESERVED_HOSTS: [&str; 14] = [
    "pay", "call", "connect", "tx", "app", "follow", "name", "wallet", "settings", "send",
    "receive", "sign", "deploy", "open",
];
const LEGACY_ACTIONS: [&str; 4] = ["pay", "call", "connect", "tx"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NameLink {
    /// Canonical DNS hostname, always ending in .sea.
    pub name: String,
    #[serde(rename = "canonicalURL")]
    pub canonical_url: String,
    /// Raw path and query, including the spelling of percent escapes.
    pub path: String,
    pub query: Option<String>,
    #[serde(rename = "isLegacy")]
    pub is_legacy: bool,
    /// The .aeth spelling for an explicitly requested 7780 alias.
    #[serde(rename = "registryName")]
    pub registry_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Name(NameLink),
    Action { host: String, raw: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserInput {
    Name(NameLink),
    Action { host: String, raw: String },
    Web(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidUrl,
    InvalidName,
    ExternalTld,
    ReservedName,
    LegacyNameUnsupported,
    UnsupportedScheme,
}

impl ParseError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalidURL",
            Self::InvalidName => "invalidName",
            Self::ExternalTld => "externalTLD",
            Self::ReservedName => "reservedName",
            Self::LegacyNameUnsupported => "legacyNameUnsupported",
            Self::UnsupportedScheme => "unsupportedScheme",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Custom URL classification. Action parameters are never decoded or
/// changed; a caller must still present its own approval UI.
pub fn parse(raw: &str, chain_id: u64) -> Result<Link, ParseError> {
    check_raw(raw)?;
    let (raw_scheme, body) = scheme_parts(raw).ok_or(ParseError::InvalidUrl)?;
    let scheme = raw_scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "sea" | "eastsea" | "aether") {
        return Err(ParseError::UnsupportedScheme);
    }
    let Some(authority) = body.strip_prefix("//") else {
        // The old URLComponents path fallback also accepted these compact
        // legacy actions. Preserve even their malformed percent bytes.
        let end = body.find(['?', '#']).unwrap_or(body.len());
        let raw_host = &body[..end];
        let host = if scheme == "sea" {
            raw_host
        } else {
            legacy_action_host(raw_host, false).unwrap_or(raw_host)
        };
        if scheme != "sea" && RESERVED_HOSTS.contains(&host) {
            return Ok(Link::Action {
                host: String::from(host),
                raw: String::from(raw),
            });
        }
        return Err(ParseError::InvalidUrl);
    };
    if scheme != "sea" {
        if let Some(host) = legacy_action_host(authority, true) {
            return Ok(Link::Action {
                host: String::from(host),
                raw: String::from(raw),
            });
        }
    }
    let (host, _) = authority_parts(authority)?;
    if RESERVED_HOSTS.contains(&host) {
        return Ok(Link::Action {
            host: String::from(host),
            raw: String::from(raw),
        });
    }
    if scheme == "aether" {
        return Err(ParseError::UnsupportedScheme);
    }
    Ok(Link::Name(name_link(authority, chain_id)?))
}

/// Match the former URLComponents interpretation only for the old actions.
/// Names and the new sea scheme still inspect an untouched authority.
fn legacy_action_host(raw: &str, authority: bool) -> Option<&'static str> {
    let mut host = raw;
    if authority {
        host = &raw[..raw.find(['/', '?', '#']).unwrap_or(raw.len())];
        if !valid_percent_escapes(host) {
            return None;
        }
        let mut fields = host.split('@');
        let first = fields.next()?;
        host = match fields.next() {
            Some(value) => {
                if fields.next().is_some()
                    || !first.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || b"!$&'()*+,;=:-._~%".contains(&byte)
                    })
                {
                    return None;
                }
                value
            }
            None => first,
        };
        if let Some((name, port)) = host.split_once(':') {
            if !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            host = name;
        }
    }
    let bytes = host.as_bytes();
    let mut decoded = String::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(byte) = bytes.get(index) {
        let value = if *byte == b'%' {
            let hex = core::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            index += 3;
            u8::from_str_radix(hex, 16).ok()?
        } else {
            index += 1;
            *byte
        };
        if !value.is_ascii() {
            return None;
        }
        decoded.push(char::from(value));
    }
    LEGACY_ACTIONS.into_iter().find(|action| *action == decoded)
}

/// Address-bar input. Bare action words remain reserved names; only an
/// explicit custom link can ask for an action's approval screen.
pub fn browser_input(raw: &str, chain_id: u64) -> Result<BrowserInput, ParseError> {
    let input = trim_input(raw);
    check_raw(input)?;
    if let Some((scheme, body)) = scheme_parts(input) {
        if (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            && body.starts_with("//")
        {
            if !web_has_host(body) {
                return Err(ParseError::InvalidUrl);
            }
            return Ok(BrowserInput::Web(String::from(input)));
        }
        return match parse(input, chain_id)? {
            Link::Name(name) => Ok(BrowserInput::Name(name)),
            Link::Action { host, raw } => Ok(BrowserInput::Action { host, raw }),
        };
    }
    Ok(BrowserInput::Name(name_link(input, chain_id)?))
}

/// Offer HTTPS only for an otherwise valid external DNS hostname. An
/// invalid authority must never be repaired into a different site's URL.
pub fn suggested_https(raw: &str) -> Option<String> {
    let input = trim_input(raw);
    match browser_input(input, 1) {
        Err(ParseError::ExternalTld) => {}
        _ => return None,
    }
    let body = match scheme_parts(input) {
        Some((scheme, body))
            if scheme.eq_ignore_ascii_case("sea") || scheme.eq_ignore_ascii_case("eastsea") =>
        {
            body.strip_prefix("//")?
        }
        _ => input,
    };
    let (host, tail) = authority_parts(body).ok()?;
    if tail.starts_with('/') {
        Some(format!("https://{host}{tail}"))
    } else {
        Some(format!("https://{host}/{tail}"))
    }
}

// Resolution accepts DNS hostname labels. The registry separately enforces
// the 3–32 character limit for a registrable second-level name.
fn name_link(raw: &str, chain_id: u64) -> Result<NameLink, ParseError> {
    check_raw(raw)?;
    let (host, tail) = authority_parts(raw)?;
    if tail.contains('#') || !valid_percent_escapes(tail) {
        return Err(ParseError::InvalidUrl);
    }
    if host.len() > 253 || !host.split('.').all(valid_label) {
        return Err(ParseError::InvalidName);
    }
    let (name, is_legacy) = if !host.contains('.') {
        (format!("{host}.sea"), false)
    } else if host.ends_with(".sea") {
        (String::from(host), false)
    } else if let Some(prefix) = host.strip_suffix(".aeth") {
        if chain_id != 7780 {
            return Err(ParseError::LegacyNameUnsupported);
        }
        (format!("{prefix}.sea"), true)
    } else {
        return Err(ParseError::ExternalTld);
    };
    if name.len() > 253 {
        return Err(ParseError::InvalidName);
    }
    let second_level = name.rsplit('.').nth(1).ok_or(ParseError::InvalidName)?;
    if RESERVED_HOSTS.contains(&second_level) {
        return Err(ParseError::ReservedName);
    }
    let (raw_path, query) = match tail.split_once('?') {
        Some((path, query)) => (path, Some(String::from(query))),
        None => (tail, None),
    };
    let path = String::from(if raw_path.is_empty() { "/" } else { raw_path });
    let canonical_url = match &query {
        Some(query) => format!("sea://{name}{path}?{query}"),
        None => format!("sea://{name}{path}"),
    };
    let registry_name = if is_legacy {
        String::from(host)
    } else {
        name.clone()
    };
    Ok(NameLink {
        name,
        canonical_url,
        path,
        query,
        is_legacy,
        registry_name,
    })
}

fn authority_parts(raw: &str) -> Result<(&str, &str), ParseError> {
    let end = raw.find(['/', '?', '#']).unwrap_or(raw.len());
    let (host, tail) = raw.split_at(end);
    if host.is_empty()
        || host
            .bytes()
            .any(|byte| matches!(byte, b'@' | b':' | b'[' | b']' | b'\\'))
    {
        return Err(ParseError::InvalidUrl);
    }
    Ok((host, tail))
}

fn check_raw(raw: &str) -> Result<(), ParseError> {
    if raw.is_empty()
        || raw
            .bytes()
            .any(|byte| byte <= 32 || byte == 127 || byte == b'\\')
    {
        return Err(ParseError::InvalidUrl);
    }
    Ok(())
}

fn scheme_parts(raw: &str) -> Option<(&str, &str)> {
    let (scheme, body) = raw.split_once(':')?;
    if !scheme.as_bytes().first()?.is_ascii_alphabetic()
        || !scheme
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return None;
    }
    Some((scheme, body))
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_percent_escapes(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'%'
            || (bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

/// Match ECMAScript String.trim rather than platform Unicode whitespace
/// tables, which disagree about NEL and the zero-width BOM.
fn trim_input(raw: &str) -> &str {
    raw.trim_matches(|ch| {
        matches!(ch,
            '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
            '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
            '\u{205f}' | '\u{3000}' | '\u{feff}'
        )
    })
}

/// Explicit HTTP(S) is ordinary browser input, separate from Sea names.
/// Preserve its bytes; reject empty hosts, malformed IPv6 and invalid ports
/// without adding a URL-library dependency to this client helper.
fn web_has_host(body: &str) -> bool {
    let Some(authority) = body.strip_prefix("//") else {
        return false;
    };
    let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
    let host_port = authority[..end].rsplit('@').next().unwrap_or("");
    if let Some(ipv6) = host_port.strip_prefix('[') {
        let Some((host, suffix)) = ipv6.split_once(']') else {
            return false;
        };
        if host.parse::<core::net::Ipv6Addr>().is_err() {
            return false;
        }
        return suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_web_port);
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    };
    !host.is_empty()
        && !host
            .bytes()
            .any(|byte| matches!(byte, b'[' | b']' | b':' | b'<' | b'>' | b'^' | b'|'))
        && port.is_none_or(valid_web_port)
}

fn valid_web_port(port: &str) -> bool {
    port.is_empty()
        || (port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u16>().is_ok())
}
