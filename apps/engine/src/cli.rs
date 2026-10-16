//! Command-line parsing. The C++ used optionparser; this accepts the same spellings
//! (`--mode sync`, `--mode=sync`, `-m sync`) and ignores options it does not know, such as
//! the client's `--info <email>`.

use std::path::PathBuf;

use ms_core::constants::{EXIT_FAILURE, EXIT_OK};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Test,
    Reset,
    Migrate,
    InstallCheck,
    ImportLegacy,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "sync" => Mode::Sync,
            "test" => Mode::Test,
            "reset" => Mode::Reset,
            "migrate" => Mode::Migrate,
            "install-check" => Mode::InstallCheck,
            "import-legacy" => Mode::ImportLegacy,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub mode: Mode,
    /// `--account` JSON; read from stdin after a prompt when absent.
    pub account: Option<String>,
    /// `--identity` JSON (a Tern server object, or a Mailspring identity, which is ignored)
    /// or `null`; read from stdin after a prompt when absent. Tests only: argv is visible to
    /// other users.
    pub identity: Option<String>,
    pub orphan: bool,
    pub verbose: bool,
    pub config_dir: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    /// Print usage and exit with this code.
    Usage(i32),
}

pub fn usage() -> String {
    "USAGE: CONFIG_DIR_PATH=/path mailsync [options]\n\n\
     Options:\n\
     \x20 --help  \tPrint usage and exit.\n\
     \x20 --identity, -i  \tIdentity JSON, or null.\n\
     \x20 --account, -a  \tRequired: Account JSON with credentials.\n\
     \x20 --mode, -m  \tRequired: sync, test, reset, migrate, install-check or import-legacy.\n\
     \x20 --orphan, -o  \tOptional: allow the process to run without a parent bound to stdin.\n\
     \x20 --verbose, -v  \tOptional: log all IMAP and SMTP traffic for debugging purposes.\n"
        .to_owned()
}

/// Parses everything after argv[0]. `CONFIG_DIR_PATH` is read here too; `IDENTITY_SERVER`
/// is not read: the Tern server comes in the identity line (docs/backend-plan.md 4.1).
pub fn parse(args: &[String]) -> Parsed {
    parse_with_env(args, std::env::var_os("CONFIG_DIR_PATH").map(PathBuf::from))
}

fn parse_with_env(args: &[String], config_dir: Option<PathBuf>) -> Parsed {
    if args.is_empty() {
        return Parsed::Usage(EXIT_OK);
    }
    let mut mode = None;
    let mut account = None;
    let mut identity = None;
    let (mut orphan, mut verbose, mut help) = (false, false, false);

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        i += 1;
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_owned())),
            _ => (arg, None),
        };
        // The value of an option that takes one: inline after `=`, else the next argument.
        let value = |i: &mut usize| -> Option<String> {
            if inline.is_some() {
                return inline.clone();
            }
            let v = args.get(*i).cloned();
            if v.is_some() {
                *i += 1;
            }
            v
        };
        match name {
            "--help" => help = true,
            "--mode" | "-m" => mode = value(&mut i),
            "--account" | "-a" => account = value(&mut i),
            "--identity" | "-i" => identity = value(&mut i),
            "--orphan" | "-o" => orphan = true,
            "--verbose" | "-v" => verbose = true,
            // Unknown options and stray values (`--info <email>`) are ignored.
            _ => {}
        }
    }

    if help {
        return Parsed::Usage(EXIT_OK);
    }
    let Some(config_dir) = config_dir.filter(|d| !d.as_os_str().is_empty()) else {
        return Parsed::Usage(EXIT_FAILURE);
    };
    let Some(mode) = mode.as_deref().and_then(Mode::parse) else {
        return Parsed::Usage(EXIT_FAILURE);
    };
    Parsed::Run(Options {
        mode,
        account,
        identity,
        orphan,
        verbose,
        config_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn run(a: &[&str]) -> Options {
        match parse_with_env(&args(a), Some("/cfg".into())) {
            Parsed::Run(o) => o,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn harness_and_client_spellings() {
        let o = run(&[
            "--mode",
            "sync",
            "--verbose",
            "--identity",
            "null",
            "--account",
            "{\"id\":1}",
        ]);
        assert_eq!(o.mode, Mode::Sync);
        assert!(o.verbose && !o.orphan);
        assert_eq!(o.identity.as_deref(), Some("null"));
        assert_eq!(o.account.as_deref(), Some("{\"id\":1}"));

        let o = run(&["--mode", "sync", "--verbose", "--info", "me@example.com"]);
        assert_eq!(o.account, None);
        assert_eq!(o.identity, None);

        let o = run(&["--mode=migrate", "-o"]);
        assert_eq!(o.mode, Mode::Migrate);
        assert!(o.orphan);
    }

    #[test]
    fn usage_cases() {
        assert_eq!(parse_with_env(&[], Some("/c".into())), Parsed::Usage(0));
        assert_eq!(parse_with_env(&args(&["--help"]), None), Parsed::Usage(0));
        assert_eq!(
            parse_with_env(&args(&["--mode", "sync"]), None),
            Parsed::Usage(1)
        );
        assert_eq!(
            parse_with_env(&args(&["--mode", "bogus"]), Some("/c".into())),
            Parsed::Usage(1)
        );
        assert_eq!(
            parse_with_env(&args(&["--verbose"]), Some("/c".into())),
            Parsed::Usage(1)
        );
    }
}
