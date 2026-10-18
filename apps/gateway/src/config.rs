// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The public URL, allowed origins and hosts, and trusted proxies (docs/web-gateway-plan.md
//! G5, G7).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{key}: {message}")]
    Invalid { key: &'static str, message: String },
}

fn invalid(key: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key,
        message: message.into(),
    }
}

/// An origin as browsers send it in `Origin`: scheme, lowercase host, and the port only when
/// it isn't the scheme's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub https: bool,
    /// `host` or `host:port`, as in a `Host` header.
    pub host: String,
}

impl Origin {
    /// Accepts `scheme://host[:port]` with an optional trailing `/`. No path, user info,
    /// query or fragment.
    pub fn parse(s: &str) -> Option<Origin> {
        let s = s.trim().trim_end_matches('/').to_ascii_lowercase();
        let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("http://") {
            (false, r)
        } else {
            return None;
        };
        if rest.is_empty() || rest.contains(['/', '@', '?', '#', ' ']) {
            return None;
        }
        let default_port = if https { ":443" } else { ":80" };
        let host = rest.strip_suffix(default_port).unwrap_or(rest).to_owned();
        Some(Origin { https, host })
    }

    pub fn as_header(&self) -> String {
        format!(
            "{}://{}",
            if self.https { "https" } else { "http" },
            self.host
        )
    }

    fn hostname(&self) -> &str {
        if self.host.starts_with('[') {
            return self
                .host
                .split_once(']')
                .map_or(&self.host, |(h, _)| &h[1..]);
        }
        self.host.rsplit_once(':').map_or(&self.host, |(h, _)| h)
    }

    fn port(&self) -> Option<&str> {
        let after = if self.host.starts_with('[') {
            self.host.split_once(']')?.1
        } else {
            &self.host
        };
        after.rsplit_once(':').map(|(_, p)| p)
    }

    pub fn is_loopback(&self) -> bool {
        let h = self.hostname();
        h == "localhost"
            || h.parse::<IpAddr>()
                .is_ok_and(|ip| ip.to_canonical().is_loopback())
    }

    /// The same origin spelled with the other loopback names, so a user can open
    /// `http://localhost:5180` or `http://127.0.0.1:5180` alike.
    fn loopback_aliases(&self) -> Vec<Origin> {
        if !self.is_loopback() {
            return Vec::new();
        }
        let port = self.port().map(|p| format!(":{p}")).unwrap_or_default();
        ["localhost", "127.0.0.1", "[::1]"]
            .iter()
            .map(|h| Origin {
                https: self.https,
                host: format!("{h}{port}"),
            })
            .filter(|o| o != self)
            .collect()
    }
}

/// An IP address or CIDR range (`trusted_proxies`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    pub fn parse(s: &str) -> Option<IpNet> {
        let (addr, prefix) = match s.trim().split_once('/') {
            Some((a, p)) => (a.parse::<IpAddr>().ok()?, Some(p.parse::<u8>().ok()?)),
            None => (s.trim().parse::<IpAddr>().ok()?, None),
        };
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(IpNet { addr, prefix })
    }

    /// IPv4-mapped IPv6 addresses (a dual-stack listener's IPv4 peers) match IPv4 ranges.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

pub fn parse_trusted_proxies<S: AsRef<str>>(list: &[S]) -> Result<Vec<IpNet>, ConfigError> {
    list.iter()
        .map(|s| s.as_ref().trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            IpNet::parse(s).ok_or_else(|| {
                invalid(
                    "trusted_proxies",
                    format!("{s:?}: expected an IP address or CIDR range"),
                )
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub public: Origin,
    /// The public origin, its loopback aliases and `--allow-origin` entries.
    pub allowed_origins: Vec<Origin>,
    pub trusted_proxies: Vec<IpNet>,
    pub web_root: Option<PathBuf>,
    /// The engine's profile directory, where `run/engine.json` names its socket.
    pub config_dir: PathBuf,
    /// `<config_dir>/gateway`.
    pub state_dir: PathBuf,
    /// WebSocket pings. Not a setting: tests shorten it.
    pub heartbeat: Heartbeat,
}

/// The gateway pings each browser WebSocket every `interval` and closes it when nothing has
/// come from the browser for `interval + timeout` (docs/web-gateway-plan.md 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heartbeat {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Heartbeat {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(15),
        }
    }
}

pub struct ConfigInput<'a> {
    pub bind: SocketAddr,
    pub public_url: Option<&'a str>,
    pub insecure_public_url: bool,
    pub allow_origins: &'a [String],
    pub trusted_proxies: &'a [String],
    pub web_root: Option<PathBuf>,
    pub config_dir: PathBuf,
}

impl Config {
    pub fn new(input: ConfigInput<'_>) -> Result<Config, ConfigError> {
        let public = match input.public_url {
            Some(u) => Origin::parse(u).ok_or_else(|| {
                invalid(
                    "public_url",
                    format!("{u:?}: expected http(s)://host[:port] with no path"),
                )
            })?,
            None if input.bind.ip().is_loopback() => Origin {
                https: false,
                host: input.bind.to_string(),
            },
            None => {
                return Err(invalid(
                    "public_url",
                    "required when binding a non-loopback address",
                ))
            }
        };
        if !public.https && !public.is_loopback() && !input.insecure_public_url {
            return Err(invalid(
                "public_url",
                format!(
                    "{} is plain http on a non-loopback host; put a TLS proxy in front or pass \
                     --insecure-public-url",
                    public.as_header()
                ),
            ));
        }
        let mut allowed_origins = vec![public.clone()];
        allowed_origins.extend(public.loopback_aliases());
        for o in input.allow_origins {
            let o = Origin::parse(o).ok_or_else(|| {
                invalid(
                    "allow_origin",
                    format!("{o:?}: expected http(s)://host[:port]"),
                )
            })?;
            if !allowed_origins.contains(&o) {
                allowed_origins.push(o);
            }
        }
        Ok(Config {
            bind: input.bind,
            public,
            allowed_origins,
            trusted_proxies: parse_trusted_proxies(input.trusted_proxies)?,
            web_root: input.web_root,
            state_dir: input.config_dir.join("gateway"),
            config_dir: input.config_dir,
            heartbeat: Heartbeat::default(),
        })
    }

    /// The cookie needs `Secure` (and may then use the `__Host-` prefix) only on https;
    /// browsers drop `Secure` cookies set over plain http.
    pub fn secure_cookie(&self) -> bool {
        self.public.https
    }

    pub fn origin_allowed(&self, origin: &str) -> bool {
        Origin::parse(origin).is_some_and(|o| self.allowed_origins.contains(&o))
    }

    /// Against DNS rebinding: a page on another name resolving to this gateway sends its own
    /// name in `Host`.
    pub fn host_allowed(&self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        self.allowed_origins.iter().any(|o| {
            o.host == host
                || host
                    .strip_suffix(if o.https { ":443" } else { ":80" })
                    .is_some_and(|h| h == o.host)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(public_url: Option<&'a str>, origins: &'a [String]) -> ConfigInput<'a> {
        ConfigInput {
            bind: "127.0.0.1:5180".parse().unwrap(),
            public_url,
            insecure_public_url: false,
            allow_origins: origins,
            trusted_proxies: &[],
            web_root: None,
            config_dir: "/tmp/x".into(),
        }
    }

    #[test]
    fn origins() {
        let o = Origin::parse("HTTPS://Tern.Example.com:443/").unwrap();
        assert_eq!(o.as_header(), "https://tern.example.com");
        assert_eq!(
            Origin::parse("http://[::1]:5180").unwrap().host,
            "[::1]:5180"
        );
        assert!(Origin::parse("http://[::1]:5180").unwrap().is_loopback());
        assert!(Origin::parse("https://a.example/tern").is_none());
        assert!(Origin::parse("https://user@a.example").is_none());
        assert!(Origin::parse("ftp://a.example").is_none());
    }

    #[test]
    fn default_public_url_is_the_loopback_bind() {
        let c = Config::new(input(None, &[])).unwrap();
        assert_eq!(c.public.as_header(), "http://127.0.0.1:5180");
        assert!(!c.secure_cookie());
        assert!(c.origin_allowed("http://localhost:5180"));
        assert!(c.host_allowed("[::1]:5180"));
        assert!(!c.origin_allowed("http://localhost:5181"));
        assert!(!c.host_allowed("evil.example:5180"));
    }

    #[test]
    fn https_public_url() {
        let dev = ["http://localhost:5173".to_owned()];
        let c = Config::new(input(Some("https://tern.example.com"), &dev)).unwrap();
        assert!(c.secure_cookie());
        assert!(c.origin_allowed("https://tern.example.com"));
        assert!(c.host_allowed("tern.example.com"));
        assert!(c.host_allowed("tern.example.com:443"));
        assert!(!c.origin_allowed("http://tern.example.com"));
        assert!(c.origin_allowed("http://localhost:5173"));
        assert!(c.host_allowed("localhost:5173"));
        assert!(!c.origin_allowed("null"));
    }

    #[test]
    fn plain_http_needs_the_flag() {
        assert!(Config::new(input(Some("http://tern.lan:5180"), &[])).is_err());
        let mut i = input(Some("http://tern.lan:5180"), &[]);
        i.insecure_public_url = true;
        assert!(Config::new(i).is_ok());
        let mut i = input(None, &[]);
        i.bind = "0.0.0.0:5180".parse().unwrap();
        assert!(Config::new(i).is_err());
    }
}
