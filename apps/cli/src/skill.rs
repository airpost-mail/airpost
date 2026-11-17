// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost skill install|uninstall|print`: the agent skill in `apps/cli/skills/airpost`, built into the
//! binary so an installed copy always describes the airpost that wrote it. The copy's SKILL.md
//! frontmatter carries `metadata.airpost-version` (the Agent Skills spec's free-form map),
//! which marks the directory as airpost's to replace or remove and lets `airpost status` notice a
//! copy older than the binary.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::app::Ctx;
use crate::cli::{SkillCommand, SkillPlace};
use crate::error::{CliError, Result};
use crate::output;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = "airpost";
const STAMP_KEY: &str = "airpost-version";

/// Every file of `apps/cli/skills/airpost`, by its path in the skill directory.
pub const FILES: &[(&str, &str)] =
    &[("SKILL.md", include_str!("../skills/airpost/SKILL.md"))];

/// Where an agent looks for skills, under the home or project directory.
const AGENTS: &[(&str, &str)] = &[("claude", ".claude"), ("codex", ".agents")];

pub fn run(ctx: &Ctx, command: &SkillCommand) -> Result<()> {
    match command {
        SkillCommand::Print => {
            output::text(skill_md());
            Ok(())
        }
        SkillCommand::Install { place, force } => {
            let dirs = dirs(place)?;
            let done = install(&dirs, VERSION, *force)?;
            report(ctx, &done);
            Ok(())
        }
        SkillCommand::Uninstall { place, force } => {
            let dirs = dirs(place)?;
            let done = uninstall(&dirs, *force)?;
            report(ctx, &done);
            Ok(())
        }
    }
}

fn skill_md() -> &'static str {
    FILES
        .iter()
        .find(|(p, _)| *p == "SKILL.md")
        .map(|(_, c)| *c)
        .unwrap_or_default()
}

fn home() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The skill directory of each agent `place` names.
fn dirs(place: &SkillPlace) -> Result<Vec<(&'static str, PathBuf)>> {
    let base = if place.project {
        std::env::current_dir()
            .map_err(|e| CliError::failed(format!("can't read the current directory: {e}")))?
    } else {
        home().ok_or_else(|| {
            CliError::failed("HOME is not set, so there is no skills directory; try --project")
        })?
    };
    Ok(agent_dirs(&base, &place.agent))
}

fn agent_dirs(base: &Path, agent: &str) -> Vec<(&'static str, PathBuf)> {
    AGENTS
        .iter()
        .filter(|(name, _)| agent == "all" || agent == *name)
        .map(|(name, dot)| (*name, base.join(dot).join("skills").join(NAME)))
        .collect()
}

/// SKILL.md with the version in its frontmatter.
fn stamped(skill_md: &str, version: &str) -> String {
    // A checkout with CRLF line ends embeds them.
    let skill_md = skill_md.replace("\r\n", "\n");
    let Some(rest) = skill_md.strip_prefix("---\n") else {
        return skill_md;
    };
    let Some(end) = rest.find("\n---\n") else {
        return skill_md;
    };
    format!(
        "---\n{}\nmetadata:\n  {STAMP_KEY}: \"{version}\"{}",
        &rest[..end],
        &rest[end..]
    )
}

/// The version an installed SKILL.md was stamped with; None for a skill airpost didn't write.
fn stamp_of(skill_md: &str) -> Option<String> {
    let skill_md = skill_md.replace("\r\n", "\n");
    let rest = skill_md.strip_prefix("---\n")?;
    let front = &rest[..rest.find("\n---")?];
    front.lines().find_map(|l| {
        let v = l.trim_start().strip_prefix(STAMP_KEY)?.strip_prefix(':')?;
        Some(v.trim().trim_matches('"').to_owned())
    })
}

/// What is at a skill directory now.
#[derive(Debug, PartialEq)]
enum Found {
    Nothing,
    Airpost(String),
    /// A directory, file or link airpost didn't write.
    Other,
}

fn found(dir: &Path) -> Found {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return Found::Nothing;
    };
    if !meta.is_dir() {
        return Found::Other;
    }
    match std::fs::read_to_string(dir.join("SKILL.md"))
        .ok()
        .as_deref()
        .and_then(stamp_of)
    {
        Some(v) => Found::Airpost(v),
        None => Found::Other,
    }
}

/// One directory's outcome: `installed`, `updated`, `removed` or `absent`.
struct Done {
    agent: &'static str,
    dir: PathBuf,
    result: &'static str,
    /// The version an `updated` copy had.
    was: Option<String>,
}

fn foreign(dir: &Path, what: &str) -> CliError {
    CliError::failed(format!(
        "{} exists and isn't a skill airpost installed; --force {what} it",
        dir.display()
    ))
}

fn remove(dir: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(dir),
        Ok(_) => std::fs::remove_file(dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Writes the skill into every directory, after checking all of them, so a refusal leaves
/// each one as it was.
fn install(dirs: &[(&'static str, PathBuf)], version: &str, force: bool) -> Result<Vec<Done>> {
    let mut plan = Vec::new();
    for (agent, dir) in dirs {
        let was = match found(dir) {
            Found::Other if !force => return Err(foreign(dir, "replaces")),
            Found::Airpost(v) => Some(v),
            Found::Other | Found::Nothing => None,
        };
        let existed = std::fs::symlink_metadata(dir).is_ok();
        plan.push(Done {
            agent,
            dir: dir.clone(),
            result: if existed { "updated" } else { "installed" },
            was,
        });
    }
    for d in &plan {
        write(&d.dir, version)
            .map_err(|e| CliError::failed(format!("can't write {}: {e}", d.dir.display())))?;
    }
    Ok(plan)
}

/// Writes the files beside `dir`, then swaps them in, so a failed write keeps the old copy.
fn write(dir: &Path, version: &str) -> std::io::Result<()> {
    let parent = dir.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{NAME}.tmp-{}", std::process::id()));
    remove(&tmp)?;
    let written = (|| {
        for (path, contents) in FILES {
            let to = tmp.join(path);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            let contents = if *path == "SKILL.md" {
                stamped(contents, version)
            } else {
                (*contents).to_owned()
            };
            std::fs::write(&to, contents)?;
        }
        remove(dir)?;
        std::fs::rename(&tmp, dir)
    })();
    if written.is_err() {
        let _ = remove(&tmp);
    }
    written
}

fn uninstall(dirs: &[(&'static str, PathBuf)], force: bool) -> Result<Vec<Done>> {
    let mut plan = Vec::new();
    for (agent, dir) in dirs {
        let (result, was) = match found(dir) {
            Found::Nothing => ("absent", None),
            Found::Other if !force => return Err(foreign(dir, "removes")),
            Found::Other => ("removed", None),
            Found::Airpost(v) => ("removed", Some(v)),
        };
        plan.push(Done {
            agent,
            dir: dir.clone(),
            result,
            was,
        });
    }
    for d in plan.iter().filter(|d| d.result == "removed") {
        remove(&d.dir)
            .map_err(|e| CliError::failed(format!("can't remove {}: {e}", d.dir.display())))?;
    }
    Ok(plan)
}

fn report(ctx: &Ctx, done: &[Done]) {
    if ctx.json {
        let skills: Vec<Value> = done
            .iter()
            .map(|d| {
                json!({
                    "agent": d.agent,
                    "path": d.dir,
                    "result": d.result,
                    "previousVersion": d.was,
                    "version": (d.result != "removed" && d.result != "absent").then_some(VERSION),
                })
            })
            .collect();
        output::json(&json!({ "skills": skills }));
        return;
    }
    for d in done {
        let dir = d.dir.display();
        output::line(&match (d.result, &d.was) {
            ("installed", _) => format!("Installed the airpost skill {VERSION} in {dir}"),
            ("updated", Some(was)) => {
                format!("Updated the airpost skill {was} to {VERSION} in {dir}")
            }
            ("updated", None) => format!("Replaced {dir} with the airpost skill {VERSION}"),
            ("removed", _) => format!("Removed {dir}"),
            _ => format!("No airpost skill in {dir}"),
        });
    }
}

/// `a.b.c` with an optional `-pre` suffix, which sorts before the release.
fn parse_version(v: &str) -> Option<(u64, u64, u64, bool)> {
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let (a, b, c) = (parts.next()??, parts.next()??, parts.next()??);
    Some((a, b, c, !pre))
}

fn is_older(stamp: &str, binary: &str) -> bool {
    match (parse_version(stamp), parse_version(binary)) {
        (Some(s), Some(b)) => s.cmp(&b) == Ordering::Less,
        // An unreadable stamp can't be trusted to match.
        _ => stamp != binary,
    }
}

/// Every skill airpost installed in the home and current directories, for `airpost status`.
pub fn installed() -> Vec<Value> {
    let mut bases: Vec<(bool, PathBuf)> = home().into_iter().map(|h| (false, h)).collect();
    if let Ok(cwd) = std::env::current_dir() {
        if bases.iter().all(|(_, b)| *b != cwd) {
            bases.push((true, cwd));
        }
    }
    let mut out = Vec::new();
    for (project, base) in bases {
        for (agent, dir) in agent_dirs(&base, "all") {
            if let Found::Airpost(v) = found(&dir) {
                out.push(json!({
                    "agent": agent,
                    "path": dir,
                    "project": project,
                    "version": v,
                    "stale": is_older(&v, VERSION),
                }));
            }
        }
    }
    out
}

/// Warns about each installed skill older than this airpost.
pub fn warn_stale(skills: &[Value]) {
    for s in skills.iter().filter(|s| s["stale"] == true) {
        output::warn(&format!(
            "the airpost skill in {} is from airpost {}; this is {VERSION}: run `airpost skill install{}` \
             to update it",
            s["path"].as_str().unwrap_or_default(),
            s["version"].as_str().unwrap_or_default(),
            if s["project"] == true {
                " --project"
            } else {
                ""
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_of_the_skill_is_built_in() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/airpost");
        let mut on_disk = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(&root).unwrap().to_string_lossy();
                    on_disk.push(rel.replace('\\', "/"));
                }
            }
        }
        on_disk.sort();
        let mut built: Vec<String> = FILES.iter().map(|(p, _)| (*p).to_owned()).collect();
        built.sort();
        assert_eq!(on_disk, built);
    }

    #[test]
    fn the_source_has_frontmatter_without_a_stamp() {
        let md = skill_md();
        assert!(md.starts_with("---\nname: airpost\n"));
        assert!(!md.contains("\nmetadata:"));
        assert_eq!(stamp_of(md), None);
        let s = stamped(md, "1.2.3");
        assert_eq!(stamp_of(&s).as_deref(), Some("1.2.3"));
        assert!(s.contains("\nmetadata:\n  airpost-version: \"1.2.3\"\n---\n"));
        assert_eq!(
            s.len(),
            md.len() + "\nmetadata:\n  airpost-version: \"1.2.3\"".len()
        );
    }

    #[test]
    fn versions_compare() {
        assert!(is_older("0.1.0", "0.2.0"));
        assert!(is_older("0.9.9", "0.10.0"));
        assert!(is_older("1.0.0-dev", "1.0.0"));
        assert!(!is_older("1.0.0", "1.0.0"));
        assert!(!is_older("1.1.0", "1.0.0"));
        assert!(is_older("junk", "1.0.0"));
    }

    #[test]
    fn install_replaces_its_own_copy_and_refuses_others() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = agent_dirs(tmp.path(), "all");
        assert_eq!(dirs.len(), 2);
        let done = install(&dirs, "0.0.1", false).unwrap();
        assert!(done.iter().all(|d| d.result == "installed"));
        assert_eq!(found(&dirs[0].1), Found::Airpost("0.0.1".into()));
        assert!(dirs[1].1.ends_with(".agents/skills/airpost"));

        let done = install(&dirs, "0.0.2", false).unwrap();
        assert_eq!(done[0].result, "updated");
        assert_eq!(done[0].was.as_deref(), Some("0.0.1"));
        assert_eq!(found(&dirs[1].1), Found::Airpost("0.0.2".into()));

        // Someone else's skill in the way: nothing is written anywhere without --force.
        let theirs = &dirs[1].1;
        std::fs::remove_dir_all(theirs).unwrap();
        std::fs::create_dir_all(theirs).unwrap();
        std::fs::write(theirs.join("SKILL.md"), "---\nname: airpost\n---\nmine\n").unwrap();
        std::fs::write(theirs.join("notes.txt"), "mine").unwrap();
        assert!(install(&dirs, "0.0.3", false).is_err());
        assert_eq!(found(&dirs[0].1), Found::Airpost("0.0.2".into()));
        assert!(uninstall(&dirs, false).is_err());
        assert_eq!(found(&dirs[0].1), Found::Airpost("0.0.2".into()));
        install(&dirs, "0.0.3", true).unwrap();
        assert_eq!(found(theirs), Found::Airpost("0.0.3".into()));
        assert!(!theirs.join("notes.txt").exists());

        let done = uninstall(&dirs, false).unwrap();
        assert!(done.iter().all(|d| d.result == "removed"));
        assert!(dirs.iter().all(|(_, d)| !d.exists()));
        let done = uninstall(&dirs, false).unwrap();
        assert!(done.iter().all(|d| d.result == "absent"));
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path().join(".claude/skills"))
            .unwrap()
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn one_agent_only() {
        let base = Path::new("/p");
        let claude = agent_dirs(base, "claude");
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].1, Path::new("/p/.claude/skills/airpost"));
        assert_eq!(
            agent_dirs(base, "codex")[0].1,
            Path::new("/p/.agents/skills/airpost")
        );
    }
}
