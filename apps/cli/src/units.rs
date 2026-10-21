// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! User units for the detached engine and for service plugins:
//! a launchd LaunchAgent on macOS, a systemd `--user` unit on Linux. Rendering
//! is pure, so both kinds are generated and tested on every OS.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The service manager that runs user units on this kind of system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Launchd,
    Systemd,
}

impl Manager {
    /// This OS's manager; None where `tern` installs no units (Windows).
    pub fn host() -> Option<Manager> {
        if cfg!(target_os = "macos") {
            Some(Manager::Launchd)
        } else if cfg!(target_os = "linux") {
            Some(Manager::Systemd)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Manager::Launchd => "launchd",
            Manager::Systemd => "systemd",
        }
    }

    /// The directory the manager reads the user's units from: `~/Library/LaunchAgents`, or
    /// `$XDG_CONFIG_HOME/systemd/user` (default `~/.config/systemd/user`).
    pub fn default_dir(self) -> Option<PathBuf> {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        match self {
            Manager::Launchd => env("HOME").map(|h| h.join("Library").join("LaunchAgents")),
            Manager::Systemd => env("XDG_CONFIG_HOME")
                .or_else(|| env("HOME").map(|h| h.join(".config")))
                .map(|d| d.join("systemd").join("user")),
        }
    }

    /// The unit's file name in [`Manager::default_dir`].
    pub fn file_name(self, name: &str) -> String {
        match self {
            Manager::Launchd => format!("{name}.plist"),
            Manager::Systemd => format!("{name}.service"),
        }
    }
}

/// Names units after the profile they serve. The desktop's default profile gets the plain
/// names; any other profile (a dev profile, `--config-dir`) gets a tag from its path, so
/// installing for it never replaces the default profile's units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Names {
    tag: Option<String>,
}

impl Names {
    pub fn for_profile(config_dir: &Path, default_dir: Option<&Path>) -> Names {
        if default_dir == Some(config_dir) {
            return Names { tag: None };
        }
        let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
        Names {
            tag: Some(hex(&digest)[..8].to_owned()),
        }
    }

    fn prefix(&self, manager: Manager) -> String {
        match (manager, &self.tag) {
            (Manager::Launchd, None) => "com.ternmail.tern".to_owned(),
            (Manager::Launchd, Some(t)) => format!("com.ternmail.tern.profile-{t}"),
            (Manager::Systemd, None) => "tern".to_owned(),
            (Manager::Systemd, Some(t)) => format!("tern-profile-{t}"),
        }
    }

    pub fn engine(&self, manager: Manager) -> String {
        let p = self.prefix(manager);
        match manager {
            Manager::Launchd => format!("{p}.engine"),
            Manager::Systemd => format!("{p}-engine"),
        }
    }

    /// `package` is a valid package name (`[a-zA-Z0-9._-]+`), which is also valid in a
    /// launchd label and a systemd unit name.
    pub fn service(&self, manager: Manager, package: &str) -> String {
        let p = self.prefix(manager);
        match manager {
            Manager::Launchd => format!("{p}.service.{package}"),
            Manager::Systemd => format!("{p}-service-{package}"),
        }
    }
}

/// What a unit runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitSpec {
    /// The launchd label or the systemd unit name without `.service`.
    pub name: String,
    pub description: String,
    /// argv, the absolute path of the program first.
    pub program: Vec<String>,
    pub env: Vec<(String, String)>,
    /// A unit this one starts after (systemd `After=` and `Wants=`). launchd has no start
    /// order: there the program waits for the engine's socket itself, and restarts on a
    /// failed exit.
    pub after: Option<String>,
    /// launchd's stdout and stderr file; systemd units log to the journal.
    pub log: Option<PathBuf>,
}

/// Seconds between restarts after a failed exit.
pub const RESTART_SECS: u32 = 10;

pub fn render(manager: Manager, spec: &UnitSpec) -> Result<String, String> {
    let all = spec
        .program
        .iter()
        .chain(spec.env.iter().flat_map(|(k, v)| [k, v]))
        .map(String::as_str)
        .chain([spec.name.as_str(), spec.description.as_str()])
        .chain(spec.after.as_deref())
        .map(str::to_owned)
        .chain(spec.log.iter().map(|p| p.to_string_lossy().into_owned()));
    for value in all {
        if value.chars().any(char::is_control) {
            return Err(format!(
                "{value:?} has a control character; a unit can't hold it"
            ));
        }
    }
    if spec.program.is_empty() || !Path::new(&spec.program[0]).is_absolute() {
        return Err("a unit's program must be an absolute path".to_owned());
    }
    if let Some((k, _)) = spec
        .env
        .iter()
        .find(|(k, _)| k.is_empty() || k.contains('='))
    {
        return Err(format!("{k:?} is not an environment variable name"));
    }
    Ok(match manager {
        Manager::Launchd => launchd_plist(spec),
        Manager::Systemd => systemd_unit(spec),
    })
}

fn launchd_plist(spec: &UnitSpec) -> String {
    let mut s = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    let key = |s: &mut String, k: &str| s.push_str(&format!("\t<key>{}</key>\n", xml(k)));
    key(&mut s, "Label");
    s.push_str(&format!("\t<string>{}</string>\n", xml(&spec.name)));
    key(&mut s, "ProgramArguments");
    s.push_str("\t<array>\n");
    for arg in &spec.program {
        s.push_str(&format!("\t\t<string>{}</string>\n", xml(arg)));
    }
    s.push_str("\t</array>\n");
    if !spec.env.is_empty() {
        key(&mut s, "EnvironmentVariables");
        s.push_str("\t<dict>\n");
        for (k, v) in &spec.env {
            s.push_str(&format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml(k),
                xml(v)
            ));
        }
        s.push_str("\t</dict>\n");
    }
    key(&mut s, "RunAtLoad");
    s.push_str("\t<true/>\n");
    // Restarted after a crash or a failed start, not after a clean exit: the engine exits 0
    // on `engine.shutdown`, which the desktop sends to an engine it can't join (the
    // credentials are still in the app) before running its own. The desktop restarts the
    // engine unit instead after an update (`tern engine restart`), and starts it again when it
    // quits (`tern engine start`).
    key(&mut s, "KeepAlive");
    s.push_str("\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n");
    key(&mut s, "ThrottleInterval");
    s.push_str(&format!("\t<integer>{RESTART_SECS}</integer>\n"));
    if let Some(log) = &spec.log {
        let log = xml(&log.to_string_lossy());
        for k in ["StandardOutPath", "StandardErrorPath"] {
            key(&mut s, k);
            s.push_str(&format!("\t<string>{log}</string>\n"));
        }
    }
    s.push_str("</dict>\n</plist>\n");
    s
}

fn systemd_unit(spec: &UnitSpec) -> String {
    let mut s = String::from("[Unit]\n");
    s.push_str(&format!(
        "Description={}\n",
        spec.description.replace('%', "%%")
    ));
    if let Some(after) = &spec.after {
        s.push_str(&format!("After={after}.service\nWants={after}.service\n"));
    }
    s.push_str("\n[Service]\nType=simple\n");
    for (k, v) in &spec.env {
        s.push_str(&format!(
            "Environment={}\n",
            systemd_quote(&format!("{k}={v}"), false)
        ));
    }
    let exec: Vec<String> = spec
        .program
        .iter()
        .map(|a| systemd_quote(a, true))
        .collect();
    s.push_str(&format!("ExecStart={}\n", exec.join(" ")));
    s.push_str(&format!("Restart=on-failure\nRestartSec={RESTART_SECS}\n"));
    s.push_str("\n[Install]\nWantedBy=default.target\n");
    s
}

/// One double-quoted word of a systemd setting: `%` specifiers are always expanded, `$`
/// variables only in `ExecStart=`.
fn systemd_quote(s: &str, exec: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' if exec => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> UnitSpec {
        UnitSpec {
            name: "com.ternmail.tern.service.sample".into(),
            description: "Tern service plugin sample (100% ours)".into(),
            program: vec![
                "/Users/me/Library/Application Support/Tern/services/sample/svc".into(),
                "--greeting=<hi & \"bye\">".into(),
                "$HOME".into(),
            ],
            env: vec![("TERN_CONFIG_DIR".into(), "/p/Tern \"x\"".into())],
            after: Some("tern-engine".into()),
            log: Some(PathBuf::from("/p/Tern/services/sample/service.log")),
        }
    }

    #[test]
    fn launchd_plist_escapes_and_restarts_on_failure_only() {
        let text = render(Manager::Launchd, &spec()).unwrap();
        let expected = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
\t<key>Label</key>
\t<string>com.ternmail.tern.service.sample</string>
\t<key>ProgramArguments</key>
\t<array>
\t\t<string>/Users/me/Library/Application Support/Tern/services/sample/svc</string>
\t\t<string>--greeting=&lt;hi &amp; &quot;bye&quot;&gt;</string>
\t\t<string>$HOME</string>
\t</array>
\t<key>EnvironmentVariables</key>
\t<dict>
\t\t<key>TERN_CONFIG_DIR</key>
\t\t<string>/p/Tern &quot;x&quot;</string>
\t</dict>
\t<key>RunAtLoad</key>
\t<true/>
\t<key>KeepAlive</key>
\t<dict>
\t\t<key>SuccessfulExit</key>
\t\t<false/>
\t</dict>
\t<key>ThrottleInterval</key>
\t<integer>10</integer>
\t<key>StandardOutPath</key>
\t<string>/p/Tern/services/sample/service.log</string>
\t<key>StandardErrorPath</key>
\t<string>/p/Tern/services/sample/service.log</string>
</dict>
</plist>
";
        assert_eq!(text, expected);
    }

    #[test]
    fn systemd_unit_is_ordered_after_the_engine_and_quotes_words() {
        let mut s = spec();
        s.name = "tern-service-sample".into();
        let text = render(Manager::Systemd, &s).unwrap();
        let expected = "\
[Unit]
Description=Tern service plugin sample (100%% ours)
After=tern-engine.service
Wants=tern-engine.service

[Service]
Type=simple
Environment=\"TERN_CONFIG_DIR=/p/Tern \\\"x\\\"\"
ExecStart=\"/Users/me/Library/Application Support/Tern/services/sample/svc\" \"--greeting=<hi & \\\"bye\\\">\" \"$$HOME\"
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
";
        assert_eq!(text, expected);
    }

    #[test]
    fn engine_unit_has_no_order() {
        let s = UnitSpec {
            name: "tern-engine".into(),
            description: "Tern mail engine".into(),
            program: vec!["/opt/tern/tern-engine".into(), "--mode".into(), "serve".into()],
            env: vec![],
            after: None,
            log: None,
        };
        let text = render(Manager::Systemd, &s).unwrap();
        assert!(!text.contains("After="));
        assert!(text.contains("ExecStart=\"/opt/tern/tern-engine\" \"--mode\" \"serve\"\n"));
        let plist = render(Manager::Launchd, &s).unwrap();
        assert!(!plist.contains("EnvironmentVariables"));
        assert!(!plist.contains("StandardOutPath"));
    }

    #[test]
    fn refuses_what_a_unit_cant_hold() {
        let mut s = spec();
        s.program.push("two\nlines".into());
        assert!(render(Manager::Systemd, &s)
            .unwrap_err()
            .contains("control"));
        let mut s = spec();
        s.program[0] = "relative/svc".into();
        assert!(render(Manager::Launchd, &s).is_err());
        let mut s = spec();
        s.env.push(("A=B".into(), "c".into()));
        assert!(render(Manager::Launchd, &s).is_err());
    }

    #[test]
    fn names_carry_a_tag_outside_the_default_profile() {
        let default = PathBuf::from("/Users/me/Library/Application Support/Tern");
        let plain = Names::for_profile(&default, Some(&default));
        assert_eq!(plain.engine(Manager::Launchd), "com.ternmail.tern.engine");
        assert_eq!(plain.engine(Manager::Systemd), "tern-engine");
        assert_eq!(
            plain.service(Manager::Launchd, "x.y"),
            "com.ternmail.tern.service.x.y"
        );
        assert_eq!(plain.service(Manager::Systemd, "x"), "tern-service-x");

        let other = Names::for_profile(Path::new("/tmp/dev-profile"), Some(&default));
        let engine = other.engine(Manager::Systemd);
        assert!(engine.starts_with("tern-profile-") && engine.ends_with("-engine"));
        assert_eq!(engine.len(), "tern-profile-".len() + 8 + "-engine".len());
        assert_eq!(
            other,
            Names::for_profile(Path::new("/tmp/dev-profile"), None)
        );
        assert_ne!(
            other,
            Names::for_profile(Path::new("/tmp/other-profile"), None)
        );
        assert_eq!(Manager::Launchd.file_name("a"), "a.plist");
        assert_eq!(Manager::Systemd.file_name("a"), "a.service");
    }
}
