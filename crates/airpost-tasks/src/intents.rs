// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The tasks behind the `threads.*` and `categories.*` intents, built from one account's
//! store: what the desktop's `TaskFactory`, `MailboxPerspective.tasksForRemovingItems`, the
//! move and label pickers and the sidebar's "Mark all as read" queue for the same actions,
//! and what `airpost` resolves for `airpost move|label|folder`. Gmail's inbox is a label, so
//! archive drops it there and moves the copies to Archive (else All Mail) elsewhere.
//!
//! Everything is looked up before a task is built, so an unknown folder or label fails the
//! intent with nothing queued.

use airpost_store::derived::category_display_name;
use airpost_store::{Folder, Label, Message, Model, Query, Store, Task, Thread};
use serde_json::{json, Value};

use crate::mail_actions::ActionError;

type ActionResult<T> = std::result::Result<T, ActionError>;

/// Ids per task: a bulk change is a few tasks, each one local transaction and a bounded
/// remote phase.
pub const IDS_PER_TASK: usize = 500;

/// A `threads.*` intent on one account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verb {
    Archive,
    Trash,
    Spam,
    NotSpam,
    /// A folder, or on Gmail a label replacing the threads' labels, named as [`resolve`]
    /// takes it.
    Move(String),
    Label {
        add: Vec<String>,
        remove: Vec<String>,
    },
    MarkRead {
        unread: bool,
    },
    Star {
        starred: bool,
    },
    RemoveFromView {
        prefer_trash: bool,
    },
}

impl Verb {
    /// The [`airpost_ipc::undo_action`] name.
    pub fn action(&self) -> &'static str {
        match self {
            Verb::Archive => "archive",
            Verb::Trash => "trash",
            Verb::Spam => "spam",
            Verb::NotSpam => "notSpam",
            Verb::Move(_) => "move",
            Verb::Label { .. } => "label",
            Verb::MarkRead { unread: false } => "markRead",
            Verb::MarkRead { unread: true } => "markUnread",
            Verb::Star { starred: true } => "star",
            Verb::Star { starred: false } => "unstar",
            Verb::RemoveFromView { .. } => "removeFromView",
        }
    }
}

/// The list the user acted from ([`airpost_ipc::Perspective`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub category_ids: Vec<String>,
    pub starred: bool,
    pub unread: bool,
}

/// One account's threads or messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub thread_ids: Vec<String>,
    pub message_ids: Vec<String>,
}

/// One of the account's folders or labels, stored JSON at hand.
#[derive(Clone, Debug)]
pub struct Category {
    pub json: Value,
    pub is_label: bool,
}

impl Category {
    pub fn id(&self) -> &str {
        str_of(&self.json, "id")
    }

    pub fn role(&self) -> &str {
        str_of(&self.json, "role")
    }

    pub fn path(&self) -> &str {
        str_of(&self.json, "path")
    }

    pub fn display_name(&self) -> String {
        category_display_name(self.path())
    }

    fn is_archive(&self) -> bool {
        matches!(self.role(), "archive" | "all")
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// An account's folders and labels.
pub struct Categories {
    pub folders: Vec<Category>,
    pub labels: Vec<Category>,
}

impl Categories {
    pub fn load(store: &Store, account_id: &str) -> ActionResult<Categories> {
        let q = || Query::new().equal("accountId", account_id);
        let folders = store
            .find_all::<Folder>(&q())?
            .iter()
            .map(|f| Category {
                json: f.to_json(),
                is_label: false,
            })
            .collect();
        let labels = store
            .find_all::<Label>(&q())?
            .iter()
            .map(|l| Category {
                json: l.to_json(),
                is_label: true,
            })
            .collect();
        Ok(Categories { folders, labels })
    }

    /// `Account.usesLabels()`: Gmail, the only provider the engine keeps labels for.
    pub fn uses_labels(&self) -> bool {
        !self.labels.is_empty()
    }

    fn all(&self) -> impl Iterator<Item = &Category> {
        self.folders.iter().chain(self.labels.iter())
    }

    pub fn by_id(&self, id: &str) -> Option<&Category> {
        self.all().find(|c| c.id() == id)
    }

    fn folder(&self, role: &str) -> Option<&Category> {
        by_role(&self.folders, role)
    }

    fn label(&self, role: &str) -> Option<&Category> {
        by_role(&self.labels, role)
    }

    /// `CategoryStore.getInboxCategory`: the inbox label on Gmail, else the inbox folder.
    fn inbox(&self) -> Option<&Category> {
        self.label("inbox").or_else(|| self.folder("inbox"))
    }

    /// `CategoryStore.getArchiveCategory`: Archive, else All Mail.
    fn archive(&self) -> Option<&Category> {
        self.folder("archive").or_else(|| self.folder("all"))
    }

    /// The first category of the view this account has.
    fn of_view(&self, view: &View) -> Option<&Category> {
        view.category_ids.iter().find_map(|id| self.by_id(id))
    }

    /// `MailboxPerspective.sourceFolderIdsForAccount`: the view's folder on this account,
    /// so a move takes only the copies the user is looking at. A label is not a placement,
    /// and a view without a category leaves the choice to the engine.
    fn source_folder_ids(&self, view: Option<&View>) -> Vec<String> {
        view.and_then(|v| self.of_view(v))
            .filter(|c| !c.is_label)
            .map(|c| vec![c.id().to_owned()])
            .unwrap_or_default()
    }
}

fn by_role<'a>(list: &'a [Category], role: &str) -> Option<&'a Category> {
    list.iter().find(|c| c.role() == role)
}

/// A folder or label named by id, by path, or by the path as a person writes it (without
/// regard to case), else by role.
pub fn resolve<'a>(list: &'a [Category], name: &str) -> Option<&'a Category> {
    let lower = name.to_lowercase();
    list.iter()
        .find(|c| c.id() == name)
        .or_else(|| list.iter().find(|c| c.path().eq_ignore_ascii_case(name)))
        .or_else(|| {
            list.iter()
                .find(|c| c.display_name().to_lowercase() == lower)
        })
        .or_else(|| by_role(list, &lower))
}

fn unavailable(what: &str) -> ActionError {
    ActionError::Unavailable(format!("the account has no {what}"))
}

/// What one change does, resolved.
enum Change {
    Unread(bool),
    Starred(bool),
    Labels {
        add: Vec<Value>,
        remove: Vec<Value>,
    },
    /// Gmail's Move to on a label: the threads' labels are replaced by it.
    LabelMove(Value),
    Folder {
        folder: Value,
        sources: Vec<String>,
    },
}

/// The tasks for `verb` on `sel` (this account's), `source` naming the caller for the log.
/// Empty when the view means nothing happens (remove from Trash or Spam).
pub fn mail_tasks(
    store: &Store,
    account_id: &str,
    verb: &Verb,
    sel: &Selection,
    view: Option<&View>,
    source: &str,
) -> ActionResult<Vec<Task>> {
    if sel.thread_ids.is_empty() == sel.message_ids.is_empty() {
        return Err(ActionError::Invalid(
            "give threadIds or messageIds, not both".into(),
        ));
    }
    let cats = Categories::load(store, account_id)?;
    let mut changes = Vec::new();
    match verb {
        Verb::Archive => changes.push(archive(&cats, view)?),
        Verb::Trash => changes.push(to_folder(&cats, "trash", Vec::new())?),
        Verb::Spam => changes.push(to_folder(&cats, "spam", Vec::new())?),
        Verb::NotSpam => {
            // Gmail's inbox is a label, so the threads go to All Mail; only the Spam copies
            // move, or the engine would also pull a thread's archived copies into the Inbox.
            let target = if cats.label("inbox").is_some() {
                cats.folder("all")
            } else {
                cats.folder("inbox")
            }
            .ok_or_else(|| unavailable("folder to move it back to"))?;
            let sources = cats
                .folder("spam")
                .map(|s| vec![s.id().to_owned()])
                .unwrap_or_default();
            changes.push(Change::Folder {
                folder: target.json.clone(),
                sources,
            });
        }
        Verb::Move(name) => match resolve(&cats.folders, name) {
            Some(f) => changes.push(Change::Folder {
                folder: f.json.clone(),
                sources: cats.source_folder_ids(view),
            }),
            None if cats.uses_labels() => {
                let label = resolve(&cats.labels, name)
                    .ok_or_else(|| ActionError::NotFound(format!("no folder or label {name}")))?;
                changes.push(Change::LabelMove(label.json.clone()));
            }
            None => return Err(ActionError::NotFound(format!("no folder {name}"))),
        },
        Verb::Label { add, remove } => {
            if !cats.uses_labels() {
                return Err(ActionError::Unavailable(
                    "the account has folders, not labels (only Gmail has labels)".into(),
                ));
            }
            let named = |names: &[String]| -> ActionResult<Vec<Value>> {
                names
                    .iter()
                    .map(|n| {
                        resolve(&cats.labels, n)
                            .map(|l| l.json.clone())
                            .ok_or_else(|| ActionError::NotFound(format!("no label {n}")))
                    })
                    .collect()
            };
            changes.push(Change::Labels {
                add: named(add)?,
                remove: named(remove)?,
            });
        }
        Verb::MarkRead { unread } => changes.push(Change::Unread(*unread)),
        Verb::Star { starred } => changes.push(Change::Starred(*starred)),
        Verb::RemoveFromView { prefer_trash } => {
            remove_from_view(&cats, view, *prefer_trash, &mut changes)?
        }
    }

    let (key, ids) = if sel.thread_ids.is_empty() {
        ("messageIds", &sel.message_ids)
    } else {
        ("threadIds", &sel.thread_ids)
    };
    let mut tasks = Vec::new();
    for change in &changes {
        for chunk in ids.chunks(IDS_PER_TASK) {
            let mut data = json!({ key: chunk, "source": source, "canBeUndone": true });
            let class = match change {
                Change::Unread(u) => {
                    data["unread"] = json!(u);
                    "ChangeUnreadTask"
                }
                Change::Starred(s) => {
                    data["starred"] = json!(s);
                    "ChangeStarredTask"
                }
                Change::Labels { add, remove } => {
                    data["labelsToAdd"] = json!(add);
                    data["labelsToRemove"] = json!(remove);
                    "ChangeLabelsTask"
                }
                Change::LabelMove(target) => {
                    data["labelsToAdd"] = json!([target]);
                    data["labelsToRemove"] =
                        json!(labels_on(store, &cats, key, chunk, str_of(target, "id"))?);
                    "ChangeLabelsTask"
                }
                Change::Folder { folder, sources } => {
                    let dest = str_of(folder, "id");
                    data["folder"] = folder.clone();
                    data["sourceFolderIds"] =
                        json!(sources.iter().filter(|s| *s != dest).collect::<Vec<_>>());
                    "ChangeFolderTask"
                }
            };
            tasks.push(Task::create(class, account_id, &data));
        }
    }
    Ok(tasks)
}

fn to_folder(cats: &Categories, role: &str, sources: Vec<String>) -> ActionResult<Change> {
    let folder = cats
        .folder(role)
        .ok_or_else(|| unavailable(&format!("{role} folder")))?;
    Ok(Change::Folder {
        folder: folder.json.clone(),
        sources,
    })
}

/// `TaskFactory.tasksForArchiving`. Gmail archive removes the inbox label, which is not a
/// placement, so it is never scoped; elsewhere only the copies the view shows move.
fn archive(cats: &Categories, view: Option<&View>) -> ActionResult<Change> {
    if let Some(inbox) = cats.label("inbox") {
        return Ok(Change::Labels {
            add: Vec::new(),
            remove: vec![inbox.json.clone()],
        });
    }
    let folder = cats
        .archive()
        .ok_or_else(|| unavailable("Archive folder"))?;
    Ok(Change::Folder {
        folder: folder.json.clone(),
        sources: cats.source_folder_ids(view),
    })
}

/// `CategoryMailboxPerspective.tasksForRemovingItems` and its unread and starred variants.
fn remove_from_view(
    cats: &Categories,
    view: Option<&View>,
    prefer_trash: bool,
    changes: &mut Vec<Change>,
) -> ActionResult<()> {
    let Some(view) = view else {
        return Err(ActionError::Invalid(
            "removeFromView needs the perspective".into(),
        ));
    };
    if view.starred && view.category_ids.is_empty() {
        changes.push(Change::Starred(false));
        return Ok(());
    }
    let Some(cat) = cats.of_view(view) else {
        return Ok(());
    };
    if cat.is_archive() {
        changes.push(to_folder(cats, "trash", Vec::new())?);
    } else if matches!(cat.role(), "spam" | "trash") {
        return Ok(());
    } else {
        // Account.preferredRemovalDestination.
        let preferred = if prefer_trash || cats.archive().is_none() {
            cats.folder("trash")
        } else {
            cats.archive()
        };
        let Some(preferred) = preferred else {
            return Ok(());
        };
        if cat.is_label && preferred.role() != "trash" {
            let mut remove = vec![cat.json.clone()];
            if let Some(inbox) = cats.inbox().filter(|i| i.id() != cat.id()) {
                remove.push(inbox.json.clone());
            }
            changes.push(Change::Labels {
                add: Vec::new(),
                remove,
            });
        } else {
            changes.push(Change::Folder {
                folder: preferred.json.clone(),
                sources: cats.source_folder_ids(Some(view)),
            });
        }
    }
    if view.unread {
        changes.push(Change::Unread(false));
    }
    Ok(())
}

/// The stored Label JSON of every label on the threads (`key` = `threadIds`) or on the
/// messages' threads, but `keep`.
fn labels_on(
    store: &Store,
    cats: &Categories,
    key: &str,
    ids: &[String],
    keep: &str,
) -> ActionResult<Vec<Value>> {
    let thread_ids: Vec<String> = if key == "threadIds" {
        ids.to_vec()
    } else {
        let mut t: Vec<String> = store
            .find_large_set::<Message>("id", ids)?
            .iter()
            .map(|m| m.thread_id().to_owned())
            .collect();
        t.sort();
        t.dedup();
        t
    };
    let mut out: Vec<Value> = Vec::new();
    for thread in store.find_large_set::<Thread>("id", &thread_ids)? {
        for l in thread.labels().as_array().into_iter().flatten() {
            let id = str_of(l, "id");
            if id == keep || out.iter().any(|o| str_of(o, "id") == id) {
                continue;
            }
            out.push(cats.by_id(id).map_or_else(|| l.clone(), |c| c.json.clone()));
        }
    }
    Ok(out)
}

// -- categories.* ---------------------------------------------------------------------------

fn category(cats: &Categories, id: &str) -> ActionResult<Category> {
    cats.by_id(id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(format!("no folder or label {id}")))
}

/// A name for `categories.create|rename`: trimmed, without leading or trailing `/`.
fn category_name(name: &str) -> ActionResult<String> {
    let name = name.trim().trim_matches('/');
    if name.is_empty() {
        return Err(ActionError::Invalid(
            "the folder or label needs a name".into(),
        ));
    }
    Ok(name.to_owned())
}

fn free(list: &[Category], name: &str, except: Option<&str>) -> ActionResult<()> {
    let lower = name.to_lowercase();
    match list.iter().find(|c| {
        Some(c.id()) != except
            && (c.path().eq_ignore_ascii_case(name) || c.display_name().to_lowercase() == lower)
    }) {
        Some(c) => Err(ActionError::Invalid(format!(
            "there is already a folder or label {} ({})",
            c.display_name(),
            c.id()
        ))),
        None => Ok(()),
    }
}

fn without_role(c: &Category, doing: &str) -> ActionResult<()> {
    match c.role() {
        "" => Ok(()),
        role => Err(ActionError::Invalid(format!(
            "{} is the account's {role} {} and can't be {doing}",
            c.display_name(),
            if c.is_label { "label" } else { "folder" }
        ))),
    }
}

/// The account's list new folders or labels go in: labels on Gmail.
fn own_kind(cats: &Categories) -> &[Category] {
    if cats.uses_labels() {
        &cats.labels
    } else {
        &cats.folders
    }
}

/// `categories.markRead`: the sidebar's "Mark all as read", as ChangeUnread tasks over the
/// category's unread threads (in all mail, unless it is Trash or Spam). Empty when none is
/// unread.
pub fn category_mark_read_tasks(
    store: &Store,
    account_id: &str,
    category_id: &str,
    source: &str,
) -> ActionResult<Vec<Task>> {
    let cats = Categories::load(store, account_id)?;
    let cat = category(&cats, category_id)?;
    let all_mail = if matches!(cat.role(), "spam" | "trash") {
        ""
    } else {
        " AND inAllMail = 1"
    };
    let ids = (|| -> rusqlite::Result<Vec<String>> {
        store
            .conn()
            .prepare(&format!(
                "SELECT id FROM ThreadCategory WHERE value = ? AND unread = 1{all_mail}"
            ))?
            .query_map([category_id], |r| r.get(0))?
            .collect()
    })()
    .map_err(|e| ActionError::Database(e.into()))?;
    Ok(ids
        .chunks(IDS_PER_TASK)
        .map(|chunk| {
            Task::create(
                "ChangeUnreadTask",
                account_id,
                &json!({"threadIds": chunk, "unread": false, "source": source,
                        "canBeUndone": true}),
            )
        })
        .collect())
}

/// `categories.create`: `SyncbackCategoryTask.forCreating`.
pub fn create_category_task(store: &Store, account_id: &str, name: &str) -> ActionResult<Task> {
    let cats = Categories::load(store, account_id)?;
    let name = category_name(name)?;
    free(own_kind(&cats), &name, None)?;
    Ok(Task::create(
        "SyncbackCategoryTask",
        account_id,
        &json!({ "path": airpost_core::utf7::encode(&name) }),
    ))
}

/// `categories.rename`: `SyncbackCategoryTask.forRenaming`.
pub fn rename_category_task(
    store: &Store,
    account_id: &str,
    category_id: &str,
    name: &str,
) -> ActionResult<Task> {
    let cats = Categories::load(store, account_id)?;
    let cat = category(&cats, category_id)?;
    without_role(&cat, "renamed")?;
    let name = category_name(name)?;
    let list = if cat.is_label {
        &cats.labels
    } else {
        &cats.folders
    };
    free(list, &name, Some(cat.id()))?;
    Ok(Task::create(
        "SyncbackCategoryTask",
        account_id,
        &json!({ "existingPath": cat.path(), "path": airpost_core::utf7::encode(&name) }),
    ))
}

/// `categories.delete`: `DestroyCategoryTask` on the stored path.
pub fn delete_category_task(
    store: &Store,
    account_id: &str,
    category_id: &str,
) -> ActionResult<Task> {
    let cats = Categories::load(store, account_id)?;
    let cat = category(&cats, category_id)?;
    without_role(&cat, "deleted")?;
    Ok(Task::create(
        "DestroyCategoryTask",
        account_id,
        &json!({ "path": cat.path() }),
    ))
}

/// `categories.empty`: `ExpungeAllInFolderTask`, for Trash, Spam or a folder without a role.
pub fn empty_folder_task(store: &Store, account_id: &str, category_id: &str) -> ActionResult<Task> {
    let cats = Categories::load(store, account_id)?;
    let cat = category(&cats, category_id)?;
    if cat.is_label || !matches!(cat.role(), "" | "trash" | "spam") {
        return Err(ActionError::Invalid(format!(
            "{} can't be emptied: only Trash, Spam and folders without a role can",
            cat.display_name()
        )));
    }
    Ok(Task::create(
        "ExpungeAllInFolderTask",
        account_id,
        &json!({ "folder": cat.json }),
    ))
}

/// One account's part of an intent, built on its `main` thread.
#[derive(Clone, Debug)]
pub enum Plan {
    Mail {
        verb: Verb,
        selection: Selection,
        view: Option<View>,
        source: String,
    },
    CategoryMarkRead {
        category_id: String,
        source: String,
    },
    CategoryCreate {
        name: String,
    },
    CategoryRename {
        category_id: String,
        name: String,
    },
    CategoryDelete {
        category_id: String,
    },
    CategoryEmpty {
        category_id: String,
    },
}

impl Plan {
    /// The tasks, in the order to queue them.
    pub fn tasks(&self, store: &Store, account_id: &str) -> ActionResult<Vec<Task>> {
        match self {
            Plan::Mail {
                verb,
                selection,
                view,
                source,
            } => mail_tasks(store, account_id, verb, selection, view.as_ref(), source),
            Plan::CategoryMarkRead {
                category_id,
                source,
            } => category_mark_read_tasks(store, account_id, category_id, source),
            Plan::CategoryCreate { name } => {
                Ok(vec![create_category_task(store, account_id, name)?])
            }
            Plan::CategoryRename { category_id, name } => Ok(vec![rename_category_task(
                store,
                account_id,
                category_id,
                name,
            )?]),
            Plan::CategoryDelete { category_id } => {
                Ok(vec![delete_category_task(store, account_id, category_id)?])
            }
            Plan::CategoryEmpty { category_id } => {
                Ok(vec![empty_folder_task(store, account_id, category_id)?])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpost_store::DeltaStream;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let clock = airpost_core::ManualClock::new(1_700_000_000);
        airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
        (dir, store)
    }

    fn folder(store: &Store, id: &str, path: &str, role: &str) {
        let mut f = Folder::new(id, "a", 0);
        f.set_path(path);
        f.set_role(role);
        store.save(&mut f).unwrap();
    }

    fn label(store: &Store, id: &str, path: &str, role: &str) {
        let mut l = Label::new(id, "a", 0);
        l.set_path(path);
        l.set_role(role);
        store.save(&mut l).unwrap();
    }

    fn threads(ids: &[&str]) -> Selection {
        Selection {
            thread_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            message_ids: Vec::new(),
        }
    }

    fn view(ids: &[&str]) -> View {
        View {
            category_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            ..View::default()
        }
    }

    fn imap(store: &Store) {
        folder(store, "fi", "INBOX", "inbox");
        folder(store, "fa", "Archive", "archive");
        folder(store, "ft", "Trash", "trash");
        folder(store, "fs", "Spam", "spam");
        folder(store, "fw", "INBOX.Work", "");
    }

    fn gmail(store: &Store) {
        folder(store, "gall", "[Gmail]/All Mail", "all");
        folder(store, "gt", "[Gmail]/Trash", "trash");
        folder(store, "gs", "[Gmail]/Spam", "spam");
        label(store, "li", "INBOX", "inbox");
        label(store, "lw", "Work", "");
    }

    fn one(tasks: Vec<Task>) -> Task {
        assert_eq!(tasks.len(), 1, "{tasks:?}");
        tasks.into_iter().next().unwrap()
    }

    #[test]
    fn archive_follows_the_task_factory() {
        let (_d, s) = store();
        imap(&s);
        let t = one(mail_tasks(
            &s,
            "a",
            &Verb::Archive,
            &threads(&["t1"]),
            Some(&view(&["fi"])),
            "x",
        )
        .unwrap());
        assert_eq!(t.class(), "ChangeFolderTask");
        assert_eq!(t.data["folder"]["id"], json!("fa"));
        assert_eq!(t.data["sourceFolderIds"], json!(["fi"]));
        assert_eq!(t.data["threadIds"], json!(["t1"]));
        assert_eq!(t.data["canBeUndone"], json!(true));

        let (_d, s) = store();
        gmail(&s);
        let t = one(mail_tasks(&s, "a", &Verb::Archive, &threads(&["t1"]), None, "x").unwrap());
        assert_eq!(t.class(), "ChangeLabelsTask");
        assert_eq!(t.data["labelsToRemove"][0]["id"], json!("li"));
        assert_eq!(t.data["labelsToAdd"], json!([]));
    }

    #[test]
    fn spam_and_not_spam() {
        let (_d, s) = store();
        imap(&s);
        let t = one(mail_tasks(&s, "a", &Verb::NotSpam, &threads(&["t1"]), None, "x").unwrap());
        assert_eq!(t.data["folder"]["id"], json!("fi"));
        assert_eq!(t.data["sourceFolderIds"], json!(["fs"]));
        let t = one(mail_tasks(
            &s,
            "a",
            &Verb::Spam,
            &threads(&["t1"]),
            Some(&view(&["fi"])),
            "x",
        )
        .unwrap());
        assert_eq!(t.data["folder"]["id"], json!("fs"));
        assert_eq!(t.data["sourceFolderIds"], json!([]));

        let (_d, s) = store();
        gmail(&s);
        let t = one(mail_tasks(&s, "a", &Verb::NotSpam, &threads(&["t1"]), None, "x").unwrap());
        assert_eq!(t.data["folder"]["id"], json!("gall"));
    }

    #[test]
    fn move_resolves_names_and_fails_before_anything_is_built() {
        let (_d, s) = store();
        imap(&s);
        for name in ["fw", "inbox.work", "Work", "WORK"] {
            let t = one(mail_tasks(
                &s,
                "a",
                &Verb::Move(name.into()),
                &threads(&["t1"]),
                None,
                "x",
            )
            .unwrap());
            assert_eq!(t.data["folder"]["id"], json!("fw"), "{name}");
        }
        let t = one(mail_tasks(
            &s,
            "a",
            &Verb::Move("archive".into()),
            &threads(&["t1"]),
            None,
            "x",
        )
        .unwrap());
        assert_eq!(t.data["folder"]["id"], json!("fa"));
        assert!(matches!(
            mail_tasks(
                &s,
                "a",
                &Verb::Move("Nope".into()),
                &threads(&["t1"]),
                None,
                "x"
            ),
            Err(ActionError::NotFound(_))
        ));
        assert!(matches!(
            mail_tasks(
                &s,
                "a",
                &Verb::Label {
                    add: vec!["Work".into()],
                    remove: vec![]
                },
                &threads(&["t1"]),
                None,
                "x"
            ),
            Err(ActionError::Unavailable(_))
        ));
    }

    #[test]
    fn remove_from_view() {
        let (_d, s) = store();
        imap(&s);
        let rm = |v: View, trash: bool| {
            mail_tasks(
                &s,
                "a",
                &Verb::RemoveFromView {
                    prefer_trash: trash,
                },
                &threads(&["t1"]),
                Some(&v),
                "x",
            )
            .unwrap()
        };
        let t = one(rm(view(&["fi"]), false));
        assert_eq!(
            (
                t.data["folder"]["id"].clone(),
                t.data["sourceFolderIds"].clone()
            ),
            (json!("fa"), json!(["fi"]))
        );
        let t = one(rm(view(&["fi"]), true));
        assert_eq!(t.data["folder"]["id"], json!("ft"));
        let t = one(rm(view(&["fa"]), false));
        assert_eq!(t.data["folder"]["id"], json!("ft"));
        assert!(rm(view(&["ft"]), false).is_empty());
        assert!(rm(view(&[]), false).is_empty());
        let mut unread = view(&["fi"]);
        unread.unread = true;
        let tasks = rm(unread, false);
        assert_eq!(
            tasks
                .iter()
                .map(|t| t.class().to_owned())
                .collect::<Vec<_>>(),
            ["ChangeFolderTask", "ChangeUnreadTask"]
        );
        assert_eq!(tasks[1].data["unread"], json!(false));
        let starred = View {
            starred: true,
            ..View::default()
        };
        let t = one(rm(starred, false));
        assert_eq!(
            (t.class(), t.data["starred"].clone()),
            ("ChangeStarredTask", json!(false))
        );

        let (_d, s) = store();
        gmail(&s);
        let t = one(mail_tasks(
            &s,
            "a",
            &Verb::RemoveFromView {
                prefer_trash: false,
            },
            &threads(&["t1"]),
            Some(&view(&["lw"])),
            "x",
        )
        .unwrap());
        assert_eq!(t.class(), "ChangeLabelsTask");
        let removed: Vec<&str> = t.data["labelsToRemove"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect();
        assert_eq!(removed, ["lw", "li"]);
        let t = one(mail_tasks(
            &s,
            "a",
            &Verb::RemoveFromView { prefer_trash: true },
            &threads(&["t1"]),
            Some(&view(&["li"])),
            "x",
        )
        .unwrap());
        assert_eq!(t.data["folder"]["id"], json!("gt"));
    }

    #[test]
    fn bulk_changes_are_batched() {
        let (_d, s) = store();
        imap(&s);
        let ids: Vec<String> = (0..1001).map(|i| format!("t{i}")).collect();
        let sel = Selection {
            thread_ids: ids,
            message_ids: Vec::new(),
        };
        let tasks = mail_tasks(&s, "a", &Verb::Star { starred: true }, &sel, None, "x").unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|t| t.data["threadIds"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            [500, 500, 1]
        );
    }

    #[test]
    fn categories() {
        let (_d, s) = store();
        imap(&s);
        let t = create_category_task(&s, "a", " Projects/2026 ").unwrap();
        assert_eq!(t.data["path"], json!("Projects/2026"));
        assert!(matches!(
            create_category_task(&s, "a", "work"),
            Err(ActionError::Invalid(_))
        ));
        assert!(matches!(
            create_category_task(&s, "a", " / "),
            Err(ActionError::Invalid(_))
        ));
        let t = rename_category_task(&s, "a", "fw", "Jobs").unwrap();
        assert_eq!(
            (t.data["existingPath"].clone(), t.data["path"].clone()),
            (json!("INBOX.Work"), json!("Jobs"))
        );
        assert!(matches!(
            rename_category_task(&s, "a", "fi", "X"),
            Err(ActionError::Invalid(_))
        ));
        assert!(matches!(
            delete_category_task(&s, "a", "fa"),
            Err(ActionError::Invalid(_))
        ));
        assert_eq!(
            delete_category_task(&s, "a", "fw").unwrap().data["path"],
            json!("INBOX.Work")
        );
        assert!(empty_folder_task(&s, "a", "ft").is_ok());
        assert!(empty_folder_task(&s, "a", "fw").is_ok());
        assert!(matches!(
            empty_folder_task(&s, "a", "fi"),
            Err(ActionError::Invalid(_))
        ));
        assert!(matches!(
            empty_folder_task(&s, "a", "nope"),
            Err(ActionError::NotFound(_))
        ));
    }
}
