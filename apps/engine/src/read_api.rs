// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The read methods, answered on the calling
//! connection's reader thread (or `serve` mode's stdin thread) from its own read-only
//! connection, never on an account's `main` thread, so a slow query delays neither task
//! local phases nor deltas. Opened on the connection's first read and kept for its lifetime.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tern_ipc::{
    code, method, CategoriesListParams, CategoryCounts, ContactsSearchParams, ContactsSearchResult,
    CountsGetParams, CountsGetResult, ErrorObject, FoldersListResult, LabelsListResult,
    MessagesGetParams, MessagesGetResult, MetadataListParams, MetadataListResult, QueryResult,
    Request as RpcRequest, SearchParams, SearchResult, TasksGetParams, TasksGetResult,
    TasksListParams, TasksListResult, ThreadsGetParams, ThreadsGetResult, ThreadsListParams,
    ThreadsListResult, READ_DEFAULT_LIMIT, READ_TIMEOUT_MS, REPLY_MAX_BYTES,
};
use tern_store::model_query::{ModelQuery, QueryError, QueryOutput};
use tern_store::read::{
    Category, CategoryTable, Cursor, InCategory, MetadataFilter, MetadataObject, ReadStore,
    TaskFilter, ThreadSearch,
};
use tern_store::search_dates::DateContext;

use crate::dispatch::Reply;
use crate::socket::{Accounts, DatabaseGeneration};

pub fn is_read_method(name: &str) -> bool {
    matches!(
        name,
        method::FOLDERS_LIST
            | method::LABELS_LIST
            | method::THREADS_LIST
            | method::THREADS_GET
            | method::MESSAGES_GET
            | method::SEARCH
            | method::CONTACTS_SEARCH
            | method::TASKS_GET
            | method::COUNTS_GET
            | method::TASKS_LIST
            | method::METADATA_LIST
            | method::QUERY
    )
}

pub struct ReadApi {
    config_dir: PathBuf,
    /// Reads are answered for every account the engine holds, running or stopped.
    accounts: Arc<dyn Accounts>,
    database: DatabaseGeneration,
    /// The connection, and the database generation it was opened on.
    store: Option<(ReadStore, u64)>,
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

fn not_found(what: &str, id: &str) -> ErrorObject {
    ErrorObject::new(code::NOT_FOUND, format!("no {what} {id}"))
}

fn invalid(why: &str) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {why}"))
}

fn query_error(e: QueryError) -> ErrorObject {
    match e {
        QueryError::Invalid(why) => invalid(&why),
        QueryError::Database(e) => internal(e),
    }
}

/// [`READ_TIMEOUT_MS`]; debug builds read `TERN_ENGINE_READ_TIMEOUT_MS` so the tests can
/// reach it.
pub(crate) fn read_timeout() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("TERN_ENGINE_READ_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_millis(READ_TIMEOUT_MS)
}

fn timeout_error() -> ErrorObject {
    ErrorObject::new(
        code::READ_TIMEOUT,
        format!(
            "the read ran longer than {} ms and was stopped",
            read_timeout().as_millis()
        ),
    )
}

/// The answer to a read whose encoded result `line` passes [`REPLY_MAX_BYTES`], if it does.
pub(crate) fn over_budget(line_len: usize) -> Option<ErrorObject> {
    (line_len > REPLY_MAX_BYTES).then(|| {
        ErrorObject::new(
            code::REPLY_TOO_LARGE,
            format!(
                "the answer is {line_len} bytes, over the {REPLY_MAX_BYTES} a read may send; \
                 ask for fewer rows"
            ),
        )
        .with_data(serde_json::json!({ "bytes": line_len, "limit": REPLY_MAX_BYTES }))
    })
}

fn to_reply(v: impl Serialize) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

fn split_counts(categories: Vec<Category>) -> (Vec<Value>, BTreeMap<String, CategoryCounts>) {
    let mut counts = BTreeMap::new();
    let mut models = Vec::with_capacity(categories.len());
    for c in categories {
        if let Some(n) = c.counts {
            counts.insert(
                c.id,
                CategoryCounts {
                    unread: n.unread,
                    total: n.total,
                },
            );
        }
        models.push(c.data);
    }
    (models, counts)
}

impl ReadApi {
    pub fn new(
        config_dir: PathBuf,
        accounts: Arc<dyn Accounts>,
        database: DatabaseGeneration,
    ) -> Self {
        ReadApi {
            config_dir,
            accounts,
            database,
            store: None,
        }
    }

    pub(crate) fn store(&mut self) -> Result<&ReadStore, ErrorObject> {
        let generation = self.database.get();
        if self.store.as_ref().is_none_or(|(_, g)| *g != generation) {
            self.store = None;
            let store = ReadStore::open(&self.config_dir)
                .map_err(|e| internal(format!("could not open the database: {e}")))?;
            self.store = Some((store, generation));
        }
        Ok(&self.store.as_ref().expect("opened above").0)
    }

    fn check_account(&self, account_id: &str) -> Result<(), ErrorObject> {
        self.accounts.check(account_id)
    }

    /// The accounts a multi-account read covers: the ones named, each of which the engine
    /// must hold, or every account it holds.
    fn resolve_accounts(&self, named: Option<Vec<String>>) -> Result<Vec<String>, ErrorObject> {
        match named {
            Some(list) => {
                for a in &list {
                    self.check_account(a)?;
                }
                Ok(list)
            }
            None => Ok(self
                .accounts
                .statuses()
                .into_iter()
                .map(|a| a.account_id)
                .collect()),
        }
    }

    /// The File row, for `files.*`.
    pub fn file(&mut self, account_id: &str, id: &str) -> Result<Option<Value>, ErrorObject> {
        self.store()?.file(account_id, id).map_err(internal)
    }

    /// The Message JSON, for `messages.unsubscribe`.
    pub fn message(&mut self, account_id: &str, id: &str) -> Result<Option<Value>, ErrorObject> {
        self.store()?.message(account_id, id).map_err(internal)
    }

    /// Whether the message's body has been fetched.
    pub fn has_body(&mut self, id: &str) -> Result<bool, ErrorObject> {
        Ok(self.body(id)?.is_some())
    }

    /// The message's body, once fetched.
    pub fn body(&mut self, id: &str) -> Result<Option<String>, ErrorObject> {
        self.store()?.message_body(id).map_err(internal)
    }

    /// Runs `f` with the reads it makes stopped after [`read_timeout`], answering
    /// [`code::READ_TIMEOUT`] then.
    pub(crate) fn timed<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ErrorObject>,
    ) -> Result<T, ErrorObject> {
        self.store()?.set_deadline(Some(read_timeout()));
        let out = f(self);
        let timed_out = self.store.as_ref().is_some_and(|(s, _)| s.end_deadline());
        if timed_out {
            return Err(timeout_error());
        }
        out
    }

    /// A [`ModelQuery`] as `query` runs it, with its timeout.
    pub(crate) fn run_query(&mut self, q: &ModelQuery) -> Result<QueryOutput, ErrorObject> {
        self.timed(|r| r.store()?.model_query(q).map_err(query_error))
    }

    /// Answers a method [`is_read_method`] accepts.
    pub fn call(&mut self, req: &RpcRequest) -> Reply {
        self.timed(|r| r.call_untimed(req))
    }

    fn call_untimed(&mut self, req: &RpcRequest) -> Reply {
        match req.method.as_str() {
            method::FOLDERS_LIST => {
                let p: CategoriesListParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let rows = self
                    .store()?
                    .categories(CategoryTable::Folder, &p.account_id)
                    .map_err(internal)?;
                let (folders, counts) = split_counts(rows);
                to_reply(FoldersListResult { folders, counts })
            }
            method::LABELS_LIST => {
                let p: CategoriesListParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let rows = self
                    .store()?
                    .categories(CategoryTable::Label, &p.account_id)
                    .map_err(internal)?;
                let (labels, counts) = split_counts(rows);
                to_reply(LabelsListResult { labels, counts })
            }
            method::THREADS_LIST => self.threads_list(req.params_as()?),
            method::THREADS_GET => {
                let p: ThreadsGetParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let store = self.store()?;
                let thread = store
                    .thread(&p.account_id, &p.id)
                    .map_err(internal)?
                    .ok_or_else(|| not_found("thread", &p.id))?;
                let messages = store
                    .thread_messages(&p.account_id, &p.id)
                    .map_err(internal)?
                    .into_iter()
                    .map(crate::warnings::with_warnings)
                    .collect();
                to_reply(ThreadsGetResult { thread, messages })
            }
            method::MESSAGES_GET => {
                let p: MessagesGetParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let store = self.store()?;
                let message = store
                    .message(&p.account_id, &p.id)
                    .map_err(internal)?
                    .map(crate::warnings::with_warnings)
                    .ok_or_else(|| not_found("message", &p.id))?;
                let body = if p.body {
                    store.message_body(&p.id).map_err(internal)?
                } else {
                    None
                };
                let files = message
                    .get("files")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                to_reply(MessagesGetResult {
                    message,
                    body,
                    files,
                })
            }
            method::SEARCH => self.search(req.params_as()?),
            method::CONTACTS_SEARCH => {
                let p: ContactsSearchParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let contacts = self
                    .store()?
                    .search_contacts(
                        &p.account_id,
                        &p.query,
                        p.limit.unwrap_or(READ_DEFAULT_LIMIT),
                    )
                    .map_err(internal)?;
                to_reply(ContactsSearchResult { contacts })
            }
            method::TASKS_GET => {
                let p: TasksGetParams = req.params_as()?;
                self.check_account(&p.account_id)?;
                let task = self
                    .store()?
                    .task(&p.account_id, &p.id)
                    .map_err(internal)?
                    .ok_or_else(|| not_found("task", &p.id))?;
                to_reply(TasksGetResult { task })
            }
            method::COUNTS_GET => {
                let p: CountsGetParams = req.params_as()?;
                let accounts = self.resolve_accounts(p.accounts)?;
                let counts = self
                    .store()?
                    .counts(&accounts)
                    .map_err(internal)?
                    .into_iter()
                    .map(|(id, n)| {
                        (
                            id,
                            CategoryCounts {
                                unread: n.unread,
                                total: n.total,
                            },
                        )
                    })
                    .collect();
                to_reply(CountsGetResult { counts })
            }
            method::TASKS_LIST => {
                let p: TasksListParams = req.params_as()?;
                let accounts = self.resolve_accounts(p.account_id.map(|a| vec![a]))?;
                let tasks = self
                    .store()?
                    .tasks(&TaskFilter {
                        accounts: &accounts,
                        statuses: p.status.as_deref(),
                        class: p.class.as_deref(),
                        header_message_id: p.header_message_id.as_deref(),
                        limit: p.limit.unwrap_or(READ_DEFAULT_LIMIT),
                    })
                    .map_err(internal)?;
                to_reply(TasksListResult { tasks })
            }
            method::METADATA_LIST => self.metadata_list(req.params_as()?),
            method::QUERY => self.query(req.params_as()?),
            other => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        }
    }

    /// `search`: `accountId`, `accounts` (each held by the engine) or every account.
    fn search(&mut self, p: SearchParams) -> Reply {
        let accounts = match (p.account_id, p.accounts) {
            (Some(_), Some(_)) => return Err(invalid("accountId and accounts exclude each other")),
            (Some(a), None) => self.resolve_accounts(Some(vec![a]))?,
            (None, named) => self.resolve_accounts(named)?,
        };
        let dates;
        let search = if p.grammar {
            dates = DateContext::new(p.tz.as_deref(), p.now);
            ThreadSearch::Grammar {
                text: &p.query,
                dates,
            }
        } else {
            ThreadSearch::Plain(&p.query)
        };
        let threads = self
            .store()?
            .search_threads(&accounts, &search, p.limit.unwrap_or(READ_DEFAULT_LIMIT))
            .map_err(query_error)?;
        to_reply(SearchResult { threads })
    }

    /// `query`: the params are a [`ModelQuery`], parsed strictly so a misspelt key is an
    /// error rather than a silently wider read. `accounts: null` reads every account in the
    /// database. A relayed connection's rows are capped first (`policy::cap_relayed_query`).
    fn query(&mut self, q: ModelQuery) -> Reply {
        if let Some(accounts) = &q.accounts {
            for a in accounts {
                self.check_account(a)?;
            }
        }
        let out = self.store()?.model_query(&q).map_err(query_error)?;
        to_reply(match out {
            QueryOutput::Rows { rows, next_cursor } => QueryResult {
                rows,
                next_cursor,
                ..Default::default()
            },
            QueryOutput::Ids(ids) => QueryResult {
                ids: Some(ids),
                ..Default::default()
            },
            QueryOutput::Count(n) => QueryResult {
                count: Some(n),
                ..Default::default()
            },
        })
    }

    fn metadata_list(&mut self, p: MetadataListParams) -> Reply {
        if p.plugin_id.is_empty() {
            return Err(invalid("pluginId is empty"));
        }
        let object = match p.object_type.as_deref() {
            None => MetadataObject::Message,
            Some(name) => MetadataObject::parse(name)
                .ok_or_else(|| invalid("objectType must be Message or Thread"))?,
        };
        if object == MetadataObject::Thread && p.draft.is_some() {
            return Err(invalid("draft only applies to messages"));
        }
        let accounts = self.resolve_accounts(p.accounts)?;
        let models = self
            .store()?
            .metadata_models(&MetadataFilter {
                accounts: &accounts,
                plugin_id: &p.plugin_id,
                object,
                draft: p.draft,
                expires_before: p.expires_before,
                has_expiration: p.has_expiration,
                after: p.after,
                before: p.before,
                limit: p.limit.unwrap_or(READ_DEFAULT_LIMIT),
                offset: p.offset.unwrap_or(0),
            })
            .map_err(internal)?;
        to_reply(MetadataListResult { models })
    }

    fn threads_list(&mut self, p: ThreadsListParams) -> Reply {
        self.check_account(&p.account_id)?;
        let filters = [&p.folder_id, &p.label_id, &p.role]
            .iter()
            .filter(|f| f.is_some())
            .count();
        if filters > 1 {
            return Err(invalid("give at most one of folderId, labelId and role"));
        }
        // Roleless folders and labels store "", which must not pick one of them.
        if p.role.as_deref() == Some("") {
            return Err(invalid("role is empty"));
        }
        let before = match &p.before {
            Some(s) => Some(Cursor::decode(s).ok_or_else(|| invalid("before is not a cursor"))?),
            None => None,
        };
        let account = p.account_id.as_str();
        let store = self.store()?;
        // (id, role) of the category to list, the role deciding whether threads outside
        // all mail show.
        let category: Option<(String, String)> = if let Some(id) = &p.folder_id {
            let role = store
                .category_role(CategoryTable::Folder, account, id)
                .map_err(internal)?
                .ok_or_else(|| not_found("folder", id))?;
            Some((id.clone(), role))
        } else if let Some(id) = &p.label_id {
            let role = store
                .category_role(CategoryTable::Label, account, id)
                .map_err(internal)?
                .ok_or_else(|| not_found("label", id))?;
            Some((id.clone(), role))
        } else if let Some(role) = &p.role {
            let id = store
                .category_for_role(account, role)
                .map_err(internal)?
                .ok_or_else(|| not_found("folder or label with role", role))?;
            Some((id, role.clone()))
        } else {
            None
        };
        let page = store
            .threads(
                account,
                category.as_ref().map(|(id, role)| InCategory { id, role }),
                p.limit.unwrap_or(READ_DEFAULT_LIMIT),
                before.as_ref(),
            )
            .map_err(internal)?;
        to_reply(ThreadsListResult {
            threads: page.threads,
            next: page.next.map(|c| c.encode()),
        })
    }
}
