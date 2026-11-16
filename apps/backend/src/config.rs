// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Server configuration: defaults, then `airpost.toml`, then `AIRPOST_*` environment variables
//! (docs/backend-plan.md 3.2).

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

pub const DEFAULT_DATA_DIR: &str = "/data";
pub const DEFAULT_BIND: &str = "127.0.0.1:5101";
/// A proxy on the same host is trusted to report the client address by default.
pub const DEFAULT_TRUSTED_PROXIES: &[&str] = &["127.0.0.0/8", "::1"];
pub const DEFAULT_BACKUP_INTERVAL: Duration = Duration::from_secs(24 * 3600);
pub const DEFAULT_BACKUP_KEEP: usize = 7;
const MIN_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid {key}: {message}")]
    Invalid { key: &'static str, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Signup {
    #[default]
    Closed,
    Open,
}

/// Server-side switches for the optional features. A feature is advertised in
/// `/airpost/v1/server` and `/airpost/v1/me` only when it is both enabled here and implemented by
/// this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureSwitches {
    pub tracking: bool,
    pub sharing: bool,
}

impl Default for FeatureSwitches {
    fn default() -> Self {
        FeatureSwitches {
            tracking: true,
            sharing: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    data_dir: Option<PathBuf>,
    bind: Option<String>,
    public_url: Option<String>,
    signup: Option<Signup>,
    features: Option<FeatureSwitches>,
    trusted_proxies: Option<Vec<String>>,
    backup: Option<FileBackup>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileBackup {
    dir: Option<PathBuf>,
    interval: Option<String>,
    keep: Option<usize>,
}

/// Scheduled backups taken by `serve`. Off unless a directory
/// is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSchedule {
    pub dir: PathBuf,
    pub interval: Duration,
    /// How many scheduled backups to keep; older ones are deleted.
    pub keep: usize,
}

/// An address range, `a.b.c.d/n` or `x::/n`; a bare address is a single host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn parse_trusted_proxies<S: AsRef<str>>(list: &[S]) -> Result<Vec<IpNet>, ConfigError> {
    list.iter()
        .map(|s| s.as_ref().trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            IpNet::parse(s).ok_or_else(|| ConfigError::Invalid {
                key: "trusted_proxies",
                message: format!("{s:?}: expected an IP address or CIDR range"),
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    /// As configured, normalized (no trailing `/`). See [`Config::public_url`].
    pub public_url: Option<String>,
    pub signup: Signup,
    pub features: FeatureSwitches,
    /// Peers whose `X-Forwarded-For` is believed when rate limiting by client address.
    pub trusted_proxies: Vec<IpNet>,
    pub backup: Option<BackupSchedule>,
}

impl Config {
    /// Loads the optional file at `path`, then applies `AIRPOST_DATA_DIR`, `AIRPOST_BIND`,
    /// `AIRPOST_PUBLIC_URL`, `AIRPOST_SIGNUP`, `AIRPOST_TRUSTED_PROXIES` (comma-separated),
    /// `AIRPOST_BACKUP_DIR`, `AIRPOST_BACKUP_INTERVAL` and `AIRPOST_BACKUP_KEEP` from `env`.
    pub fn load(
        path: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Config, ConfigError> {
        let file = match path {
            Some(p) => {
                let text = std::fs::read_to_string(p).map_err(|source| ConfigError::Read {
                    path: p.to_owned(),
                    source,
                })?;
                toml::from_str::<FileConfig>(&text).map_err(|source| ConfigError::Parse {
                    path: p.to_owned(),
                    source,
                })?
            }
            None => FileConfig::default(),
        };

        let data_dir = env("AIRPOST_DATA_DIR")
            .map(PathBuf::from)
            .or(file.data_dir)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));

        let bind_str = env("AIRPOST_BIND")
            .or(file.bind)
            .unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let bind: SocketAddr = bind_str.parse().map_err(|e| ConfigError::Invalid {
            key: "bind",
            message: format!("{bind_str:?}: {e}"),
        })?;

        let signup = match env("AIRPOST_SIGNUP") {
            Some(s) => match s.as_str() {
                "closed" => Signup::Closed,
                "open" => Signup::Open,
                other => {
                    return Err(ConfigError::Invalid {
                        key: "signup",
                        message: format!("{other:?}, expected \"closed\" or \"open\""),
                    })
                }
            },
            None => file.signup.unwrap_or_default(),
        };

        let public_url = env("AIRPOST_PUBLIC_URL")
            .or(file.public_url)
            .map(|u| normalize_public_url(&u))
            .transpose()?;

        let trusted_proxies = match env("AIRPOST_TRUSTED_PROXIES") {
            Some(list) => parse_trusted_proxies(&list.split(',').collect::<Vec<_>>())?,
            None => match &file.trusted_proxies {
                Some(list) => parse_trusted_proxies(list)?,
                None => parse_trusted_proxies(DEFAULT_TRUSTED_PROXIES)?,
            },
        };

        let file_backup = file.backup.unwrap_or_default();
        let backup_interval = env("AIRPOST_BACKUP_INTERVAL")
            .or(file_backup.interval)
            .map(|s| parse_interval(&s))
            .transpose()?;
        let backup_keep = match env("AIRPOST_BACKUP_KEEP") {
            Some(s) => Some(
                s.trim()
                    .parse::<usize>()
                    .map_err(|e| ConfigError::Invalid {
                        key: "backup.keep",
                        message: format!("{s:?}: {e}"),
                    })?,
            ),
            None => file_backup.keep,
        };
        let backup = match env("AIRPOST_BACKUP_DIR")
            .map(PathBuf::from)
            .or(file_backup.dir)
            .filter(|d| !d.as_os_str().is_empty())
        {
            Some(dir) => {
                let keep = backup_keep.unwrap_or(DEFAULT_BACKUP_KEEP);
                if keep == 0 {
                    return Err(ConfigError::Invalid {
                        key: "backup.keep",
                        message: "must be at least 1".into(),
                    });
                }
                Some(BackupSchedule {
                    dir,
                    interval: backup_interval.unwrap_or(DEFAULT_BACKUP_INTERVAL),
                    keep,
                })
            }
            None => None,
        };

        Ok(Config {
            data_dir,
            bind,
            public_url,
            signup,
            features: file.features.unwrap_or_default(),
            trusted_proxies,
            backup,
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("airpost.db")
    }

    /// The URL recipients reach for tracking and shares (O8), returned as `airpost.publicURL`.
    /// Defaults to `http://<bind>` when that is a concrete address. Only `serve` needs it, so
    /// the other commands work without one.
    pub fn public_url(&self) -> Result<String, ConfigError> {
        match &self.public_url {
            Some(u) => Ok(u.clone()),
            None if self.bind.ip().is_unspecified() => Err(ConfigError::Invalid {
                key: "public_url",
                message: format!(
                    "must be set when binding {} (set AIRPOST_PUBLIC_URL to the URL clients and recipients use)",
                    self.bind
                ),
            }),
            None => Ok(format!("http://{}", self.bind)),
        }
    }

    /// Basic auth over plain HTTP leaks tokens, so a public URL must be `https` unless it
    /// points at loopback (3.2).
    pub fn check_public_url_is_safe(&self) -> Result<(), ConfigError> {
        let url = self.public_url()?;
        let (scheme, host) = split_url(&url).expect("normalized");
        if scheme == "https" || is_loopback_host(host) {
            Ok(())
        } else {
            Err(ConfigError::Invalid {
                key: "public_url",
                message: format!(
                    "{url} is plain http on a non-loopback host; use https behind a TLS proxy, or pass --insecure-public-url"
                ),
            })
        }
    }
}

/// `<n>s`, `<n>m`, `<n>h` or `<n>d`, at least a minute.
fn parse_interval(s: &str) -> Result<Duration, ConfigError> {
    let invalid = |message: &str| ConfigError::Invalid {
        key: "backup.interval",
        message: format!("{s:?}: {message}"),
    };
    let t = s.trim();
    let split = t.len().saturating_sub(1);
    let unit = match t.get(split..) {
        Some("s") => 1,
        Some("m") => 60,
        Some("h") => 3600,
        Some("d") => 86400,
        _ => return Err(invalid("expected a number and s, m, h or d, such as 24h")),
    };
    let n: u64 = t[..split]
        .parse()
        .map_err(|_| invalid("expected a number and s, m, h or d, such as 24h"))?;
    let d = Duration::from_secs(n.saturating_mul(unit));
    if d < MIN_BACKUP_INTERVAL {
        return Err(invalid("must be at least 1m"));
    }
    Ok(d)
}

fn normalize_public_url(url: &str) -> Result<String, ConfigError> {
    let invalid = |message: &str| ConfigError::Invalid {
        key: "public_url",
        message: format!("{url:?}: {message}"),
    };
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.contains(['?', '#']) || trimmed.chars().any(char::is_whitespace) {
        return Err(invalid("no query, fragment or spaces allowed"));
    }
    match split_url(trimmed) {
        Some((_, host)) if !host.is_empty() => Ok(trimmed.to_owned()),
        Some(_) => Err(invalid("missing host")),
        None => Err(invalid("expected http:// or https://")),
    }
}

/// Returns `(scheme, host)` for an `http(s)://host[:port][/path]` URL. `host` keeps IPv6
/// brackets and drops the port.
fn split_url(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') {
        return None;
    }
    let host = if authority.starts_with('[') {
        let end = authority.find(']')?;
        &authority[..=end]
    } else {
        authority.split(':').next().unwrap_or("")
    };
    Some((scheme, host))
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn defaults() {
        let c = Config::load(None, env(&[])).unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/data"));
        assert_eq!(c.bind.to_string(), "127.0.0.1:5101");
        assert_eq!(c.public_url, None);
        assert_eq!(c.public_url().unwrap(), "http://127.0.0.1:5101");
        assert_eq!(c.signup, Signup::Closed);
        assert_eq!(c.features, FeatureSwitches::default());
        assert!(c
            .trusted_proxies
            .iter()
            .any(|n| n.contains("127.0.0.1".parse().unwrap())));
        assert!(c
            .trusted_proxies
            .iter()
            .any(|n| n.contains("::1".parse().unwrap())));
        assert!(!c
            .trusted_proxies
            .iter()
            .any(|n| n.contains("10.0.0.1".parse().unwrap())));
        assert_eq!(c.backup, None);
        c.check_public_url_is_safe().unwrap();
    }

    #[test]
    fn backup_schedule() {
        let c = Config::load(None, env(&[("AIRPOST_BACKUP_DIR", "/backups")])).unwrap();
        assert_eq!(
            c.backup,
            Some(BackupSchedule {
                dir: "/backups".into(),
                interval: DEFAULT_BACKUP_INTERVAL,
                keep: DEFAULT_BACKUP_KEEP,
            })
        );
        // An interval or count alone does not turn backups on.
        let c = Config::load(None, env(&[("AIRPOST_BACKUP_INTERVAL", "1h")])).unwrap();
        assert_eq!(c.backup, None);
        let c = Config::load(None, env(&[("AIRPOST_BACKUP_DIR", "")])).unwrap();
        assert_eq!(c.backup, None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airpost.toml");
        std::fs::write(
            &path,
            "[backup]\ndir = \"/srv/backups\"\ninterval = \"30m\"\nkeep = 3\n",
        )
        .unwrap();
        let b = Config::load(Some(&path), env(&[])).unwrap().backup.unwrap();
        assert_eq!(
            (b.dir, b.interval, b.keep),
            ("/srv/backups".into(), Duration::from_secs(1800), 3)
        );
        let b = Config::load(
            Some(&path),
            env(&[
                ("AIRPOST_BACKUP_INTERVAL", "2d"),
                ("AIRPOST_BACKUP_KEEP", "10"),
            ]),
        )
        .unwrap()
        .backup
        .unwrap();
        assert_eq!((b.interval, b.keep), (Duration::from_secs(2 * 86400), 10));

        for (k, v) in [
            ("AIRPOST_BACKUP_INTERVAL", "30s"),
            ("AIRPOST_BACKUP_INTERVAL", "24"),
            ("AIRPOST_BACKUP_INTERVAL", "h"),
            ("AIRPOST_BACKUP_INTERVAL", "-1h"),
            ("AIRPOST_BACKUP_KEEP", "0"),
            ("AIRPOST_BACKUP_KEEP", "x"),
        ] {
            assert!(
                Config::load(None, env(&[("AIRPOST_BACKUP_DIR", "/b"), (k, v)])).is_err(),
                "{k}={v}"
            );
        }
    }

    #[test]
    fn trusted_proxies() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let n = IpNet::parse("172.16.0.0/12").unwrap();
        assert!(n.contains(ip("172.17.0.1")));
        assert!(n.contains(ip("::ffff:172.31.255.255")), "mapped IPv4");
        assert!(!n.contains(ip("172.32.0.1")));
        assert!(!n.contains(ip("::1")));
        assert!(IpNet::parse("0.0.0.0/0").unwrap().contains(ip("8.8.8.8")));
        let n = IpNet::parse("fd00::/8").unwrap();
        assert!(n.contains(ip("fd12::1")) && !n.contains(ip("fe80::1")));
        assert!(IpNet::parse("10.0.0.1").unwrap().contains(ip("10.0.0.1")));
        assert!(!IpNet::parse("10.0.0.1").unwrap().contains(ip("10.0.0.2")));
        for bad in ["10.0.0.0/33", "::/129", "nope", "10.0.0.0/x"] {
            assert!(IpNet::parse(bad).is_none(), "{bad}");
        }

        let c = Config::load(
            None,
            env(&[("AIRPOST_TRUSTED_PROXIES", "10.0.0.0/8, fd00::/8")]),
        )
        .unwrap();
        assert_eq!(c.trusted_proxies.len(), 2);
        assert!(!c
            .trusted_proxies
            .iter()
            .any(|n| n.contains(ip("127.0.0.1"))));
        let c = Config::load(None, env(&[("AIRPOST_TRUSTED_PROXIES", "")])).unwrap();
        assert!(c.trusted_proxies.is_empty());
        assert!(Config::load(None, env(&[("AIRPOST_TRUSTED_PROXIES", "x")])).is_err());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airpost.toml");
        std::fs::write(&path, "trusted_proxies = [\"172.16.0.0/12\"]\n").unwrap();
        let c = Config::load(Some(&path), env(&[])).unwrap();
        assert_eq!(c.trusted_proxies, [IpNet::parse("172.16.0.0/12").unwrap()]);
    }

    #[test]
    fn env_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airpost.toml");
        std::fs::write(
            &path,
            r#"
data_dir = "/srv/airpost"
bind = "127.0.0.1:6000"
public_url = "https://file.example/"
signup = "open"
[features]
sharing = false
"#,
        )
        .unwrap();
        let c = Config::load(Some(&path), env(&[])).unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/srv/airpost"));
        assert_eq!(c.bind.port(), 6000);
        assert_eq!(c.public_url().unwrap(), "https://file.example");
        assert_eq!(c.signup, Signup::Open);
        assert!(c.features.tracking && !c.features.sharing);

        let c = Config::load(
            Some(&path),
            env(&[
                ("AIRPOST_DATA_DIR", "/x"),
                ("AIRPOST_BIND", "0.0.0.0:5101"),
                ("AIRPOST_PUBLIC_URL", "https://env.example/airpost"),
                ("AIRPOST_SIGNUP", "closed"),
            ]),
        )
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/x"));
        assert_eq!(c.bind.to_string(), "0.0.0.0:5101");
        assert_eq!(c.public_url().unwrap(), "https://env.example/airpost");
        assert_eq!(c.signup, Signup::Closed);
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("airpost.toml");
        std::fs::write(&path, "bnd = \"x\"\n").unwrap();
        assert!(matches!(
            Config::load(Some(&path), env(&[])),
            Err(ConfigError::Parse { .. })
        ));
        assert!(Config::load(None, env(&[("AIRPOST_SIGNUP", "maybe")])).is_err());
        assert!(Config::load(None, env(&[("AIRPOST_BIND", "nope")])).is_err());
        assert!(Config::load(None, env(&[("AIRPOST_PUBLIC_URL", "ftp://x")])).is_err());
        assert!(Config::load(None, env(&[("AIRPOST_PUBLIC_URL", "https://x/?a=1")])).is_err());
        assert!(Config::load(None, env(&[("AIRPOST_PUBLIC_URL", "https://")])).is_err());
    }

    #[test]
    fn unspecified_bind_needs_public_url() {
        // Loading works (CLI commands need no public URL); resolving it does not.
        let c = Config::load(None, env(&[("AIRPOST_BIND", "0.0.0.0:5101")])).unwrap();
        assert!(c.public_url().is_err());
        assert!(c.check_public_url_is_safe().is_err());
    }

    #[test]
    fn public_url_safety() {
        let ok = [
            "https://mail.example.com",
            "https://10.0.0.1:8443/airpost",
            "http://localhost:5101",
            "http://127.0.0.1",
            "http://127.1.2.3:80/x",
            "http://[::1]:5101",
        ];
        let bad = [
            "http://mail.example.com",
            "http://10.0.0.1:5101",
            "http://[2001:db8::1]:5101",
            "http://localhost.example.com",
        ];
        for u in ok {
            let c = Config::load(None, env(&[("AIRPOST_PUBLIC_URL", u)])).unwrap();
            assert!(c.check_public_url_is_safe().is_ok(), "{u}");
        }
        for u in bad {
            let c = Config::load(None, env(&[("AIRPOST_PUBLIC_URL", u)])).unwrap();
            assert!(c.check_public_url_is_safe().is_err(), "{u}");
        }
    }
}
