// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `query` read: the JSON form of the desktop's
//! `ModelQuery` (`Query.toJSON()`), compiled to SQL on a [`ReadStore`] the way the desktop's
//! `Query.sql()` builds it, so both return the same rows.
//!
//! Derived from Mailspring `app/src/flux/models/query.ts` and `app/src/flux/attributes/`
//! (`matcher.ts`, `attribute-*.ts`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Classes and attributes come from `model_query_allowlist.json`, generated from tern-ipc's
//! `query_attrs.rs` (`cargo run -p tern-ipc --bin tern-ipc-schema`);
//! `apps/desktop/tests/integration/model-query-capture.test.ts` fails when the desktop models'
//! `queryable` attributes and the table differ. Nothing outside that table reaches the SQL
//! text, and every value is a bound parameter. `tests/model_query_parity.rs` runs the queries
//! the desktop's call sites build through both compilers on one database.
//!
//! What differs from `Query.sql()`, on purpose:
//! - `accounts` adds `accountId IN (…)`; `null` reads every account in the database, as the
//!   desktop's own SQLite reads do. The thread list's subselect also keeps only the rows of
//!   those accounts' folders and labels, so its LIMIT and OFFSET count their threads.
//! - `search` with `grammar: true` is the search grammar ([`crate::search_grammar`]) on
//!   Thread, its dates read in `search.tz` at `search.now`; other classes refuse it.
//! - Booleans compared with `=`/`!=` against `true`/`false` mean "non-zero"/"zero", as
//!   `AttributeBoolean.equal` sends them; `Thread.unread` and `starred` are counts.
//! - `cursor` pages by key instead of `offset` (T5); the desktop never sends one. A cursor
//!   query's subselect returns each thread once, so a page isn't cut short by a thread in
//!   two of the listed categories.
//! - The subselect plan keeps the joined-data join that `Query.sql()` leaves out of it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, Row};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::read::{with_hidden, ReadStore};
use crate::search_dates::DateContext;
use crate::search_grammar::{self, GrammarError};

/// A `query` request (`tern_ipc::QueryParams` on the wire).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelQuery {
    pub class: String,
    /// `None` reads every account.
    #[serde(default)]
    pub accounts: Option<Vec<String>>,
    /// A matcher, or a list of them (all must match).
    #[serde(default, rename = "where")]
    pub where_: Option<Value>,
    #[serde(default)]
    pub search: Option<Search>,
    /// Empty or absent: the class's natural order.
    #[serde(default)]
    pub order: Option<Vec<Order>>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    /// Keyset paging: `""` for the first page, then the previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub distinct: Option<bool>,
    /// Joined-data attributes to load (`body` on Message).
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub ids_only: Option<bool>,
    #[serde(default)]
    pub count: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Search {
    pub text: String,
    #[serde(default)]
    pub grammar: bool,
    /// The IANA zone date terms in the grammar are read in; `None` is UTC.
    #[serde(default)]
    pub tz: Option<String>,
    /// Unix seconds that relative date terms (`yesterday`, `3d`) count back from.
    #[serde(default)]
    pub now: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    pub attr: String,
    pub dir: Direction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Direction {
    #[serde(rename = "asc", alias = "ASC")]
    Asc,
    #[serde(rename = "desc", alias = "DESC")]
    Desc,
}

impl Direction {
    fn sql(self) -> &'static str {
        match self {
            Direction::Asc => "ASC",
            Direction::Desc => "DESC",
        }
    }
}

/// What a query returns: models, ids or a count.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryOutput {
    /// `next_cursor` is `Some` only for a cursor query: the cursor of the next page, `None`
    /// on the last one.
    Rows {
        rows: Vec<Value>,
        next_cursor: Option<Option<String>>,
    },
    Ids(Vec<String>),
    Count(i64),
}

#[derive(Debug)]
pub enum QueryError {
    /// The query names something the allowlist doesn't have, is malformed, or its search
    /// text doesn't parse.
    Invalid(String),
    Database(tern_core::Error),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Invalid(m) => f.write_str(m),
            QueryError::Database(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for QueryError {}

impl From<rusqlite::Error> for QueryError {
    fn from(e: rusqlite::Error) -> Self {
        QueryError::Database(e.into())
    }
}

fn invalid(msg: impl Into<String>) -> QueryError {
    QueryError::Invalid(msg.into())
}

// -- Allowlist ------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Allowlist {
    pub classes: BTreeMap<String, ClassDef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassDef {
    pub attributes: BTreeMap<String, AttrDef>,
    pub natural_order: Option<NaturalOrder>,
}

#[derive(Debug, Deserialize)]
pub struct NaturalOrder {
    pub attr: String,
    pub dir: Direction,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttrDef {
    String {
        column: String,
    },
    Number {
        column: String,
    },
    Boolean {
        column: String,
    },
    Datetime {
        column: String,
    },
    Object {
        column: String,
    },
    #[serde(rename_all = "camelCase")]
    Collection {
        join_table: String,
        join_queryable_by: Vec<String>,
    },
    JoinedData {
        table: String,
    },
}

impl AttrDef {
    /// The main-table column of a plain attribute.
    fn column(&self) -> Option<&str> {
        match self {
            AttrDef::String { column }
            | AttrDef::Number { column }
            | AttrDef::Boolean { column }
            | AttrDef::Datetime { column } => Some(column),
            _ => None,
        }
    }
}

/// The checked-in table of the desktop's queryable attributes.
pub fn allowlist() -> &'static Allowlist {
    static TABLE: OnceLock<Allowlist> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("model_query_allowlist.json"))
            .expect("model_query_allowlist.json parses")
    })
}

/// Attributes the desktop declares queryable that have no column in the engine's schema.
const UNSTORED: &[(&str, &str)] = &[("Event", "status")];

const THREAD_CATEGORY: &str = "ThreadCategory";

/// The FTS table `.search(text)` matches against, for the classes that have one.
fn search_table(class: &str) -> Option<&'static str> {
    match class {
        "Thread" => Some("ThreadSearch"),
        "Contact" => Some("ContactSearch"),
        "Event" => Some("EventSearch"),
        _ => None,
    }
}

// -- Matchers -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    NotIn,
    Contains,
    ContainsAny,
    StartsWith,
    Like,
}

impl Op {
    fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "=" => Op::Eq,
            "!=" => Op::Ne,
            "<" => Op::Lt,
            ">" => Op::Gt,
            "<=" => Op::Le,
            ">=" => Op::Ge,
            "in" => Op::In,
            "not in" => Op::NotIn,
            "contains" => Op::Contains,
            "containsAny" => Op::ContainsAny,
            "startsWith" => Op::StartsWith,
            "like" => Op::Like,
            _ => return None,
        })
    }

    fn comparison(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Gt => ">",
            Op::Le => "<=",
            Op::Ge => ">=",
            Op::Like => "like",
            Op::In => "IN",
            Op::NotIn => "NOT IN",
            Op::Contains | Op::ContainsAny | Op::StartsWith => unreachable!("not a comparison"),
        }
    }
}

#[derive(Clone, Debug)]
struct Leaf {
    attr: String,
    def: AttrDef,
    op: Op,
    value: Value,
    /// The join alias of a collection matcher.
    alias: Option<usize>,
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(Leaf),
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Vec<Node>),
}

impl Node {
    fn leaves<'a>(&'a self, out: &mut Vec<&'a Leaf>) {
        match self {
            Node::Leaf(l) => out.push(l),
            Node::And(c) | Node::Or(c) | Node::Not(c) => c.iter().for_each(|n| n.leaves(out)),
        }
    }
}

struct Parser<'a> {
    class: &'a str,
    def: &'a ClassDef,
    joins: Vec<String>,
}

impl Parser<'_> {
    /// The top-level matchers: a list, a top-level `and`, or one matcher.
    fn top(&mut self, v: &Value) -> Result<Vec<Node>, QueryError> {
        match v {
            Value::Array(list) => list.iter().map(|m| self.node(m)).collect(),
            Value::Object(o) if o.len() == 1 && o.contains_key("and") => match self.node(v)? {
                Node::And(children) => Ok(children),
                other => Ok(vec![other]),
            },
            _ => Ok(vec![self.node(v)?]),
        }
    }

    fn node(&mut self, v: &Value) -> Result<Node, QueryError> {
        let Value::Object(o) = v else {
            return Err(invalid("a matcher must be an object"));
        };
        if o.contains_key("attr") {
            return self.leaf(o).map(Node::Leaf);
        }
        let (key, children) = match o.iter().next() {
            Some((k, Value::Array(c))) if o.len() == 1 => (k.as_str(), c),
            _ => {
                return Err(invalid(
                    "a matcher is {attr, op, value} or one of {and|or|not: [matchers]}",
                ))
            }
        };
        let children = children
            .iter()
            .map(|c| self.node(c))
            .collect::<Result<Vec<_>, _>>()?;
        match key {
            "and" => Ok(Node::And(children)),
            "or" => Ok(Node::Or(children)),
            "not" => Ok(Node::Not(children)),
            other => Err(invalid(format!("unknown matcher {other:?}"))),
        }
    }

    fn leaf(&mut self, o: &Map<String, Value>) -> Result<Leaf, QueryError> {
        for k in o.keys() {
            if !matches!(k.as_str(), "attr" | "op" | "value") {
                return Err(invalid(format!("unknown matcher field {k:?}")));
            }
        }
        let attr = o
            .get("attr")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("attr must be a string"))?;
        let def = self
            .def
            .attributes
            .get(attr)
            .filter(|_| !UNSTORED.contains(&(self.class, attr)))
            .ok_or_else(|| {
                invalid(format!(
                    "{} has no queryable attribute {attr:?}",
                    self.class
                ))
            })?;
        let op_name = o
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("op must be a string"))?;
        let op = Op::parse(op_name).ok_or_else(|| invalid(format!("unknown op {op_name:?}")))?;
        let value = o.get("value").cloned().unwrap_or(Value::Null);
        let collection = matches!(def, AttrDef::Collection { .. });
        let alias = match (collection, op) {
            (true, Op::Contains | Op::ContainsAny) => {
                let AttrDef::Collection { join_table, .. } = def else {
                    unreachable!()
                };
                self.joins.push(join_table.clone());
                Some(self.joins.len() - 1)
            }
            (true, _) => {
                return Err(invalid(format!(
                    "{attr} is a collection: use contains or containsAny"
                )))
            }
            (false, Op::Contains | Op::ContainsAny) => {
                return Err(invalid(format!("{attr} is not a collection")))
            }
            (false, _) if def.column().is_none() => {
                return Err(invalid(format!("{attr} can't be compared")))
            }
            _ => None,
        };
        Ok(Leaf {
            attr: attr.to_owned(),
            def: def.clone(),
            op,
            value,
            alias,
        })
    }
}

// -- SQL ------------------------------------------------------------------------------------

/// SQL text and its parameters, in order.
#[derive(Default)]
struct Sql {
    text: String,
    params: Vec<SqlValue>,
}

impl Sql {
    fn push(&mut self, s: &str) -> &mut Self {
        self.text.push_str(s);
        self
    }

    fn bind(&mut self, v: SqlValue) -> &mut Self {
        self.text.push('?');
        self.params.push(v);
        self
    }

    fn append(&mut self, other: Sql) -> &mut Self {
        self.text.push_str(&other.text);
        self.params.extend(other.params);
        self
    }

    /// `(?,?,…)`, or a `json_each` over one parameter for long lists, which would otherwise
    /// run into SQLite's limit on parameters per statement.
    fn bind_list(&mut self, values: Vec<SqlValue>, json: &[Value]) -> &mut Self {
        if values.len() > 500 {
            self.push("(SELECT value FROM json_each(")
                .bind(SqlValue::Text(Value::Array(json.to_vec()).to_string()))
                .push("))")
        } else {
            self.push("(");
            for (i, v) in values.into_iter().enumerate() {
                if i > 0 {
                    self.push(",");
                }
                self.bind(v);
            }
            self.push(")")
        }
    }
}

fn scalar(v: &Value) -> Result<SqlValue, QueryError> {
    Ok(match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        _ => {
            return Err(invalid(
                "a matcher value must be a string, number, boolean or null",
            ))
        }
    })
}

fn list(v: &Value) -> Result<(Vec<SqlValue>, &[Value]), QueryError> {
    let Value::Array(items) = v else {
        return Err(invalid("in, not in and containsAny take a list"));
    };
    let values = items.iter().map(scalar).collect::<Result<_, _>>()?;
    Ok((values, items))
}

fn quoted(name: &str) -> String {
    format!("`{name}`")
}

/// Where a matcher's columns are read from: the class's table and each join's alias, or (in
/// the thread-list subselect) the join table for both.
#[derive(Clone, Copy)]
enum Tables<'a> {
    Main(&'a str),
    JoinTable(&'a str),
}

impl Tables<'_> {
    fn main(&self) -> String {
        match self {
            Tables::Main(t) | Tables::JoinTable(t) => quoted(t),
        }
    }

    fn join(&self, alias: usize) -> String {
        match self {
            Tables::Main(_) => quoted(&format!("M{alias}")),
            Tables::JoinTable(t) => quoted(t),
        }
    }
}

fn leaf_sql(l: &Leaf, t: Tables<'_>) -> Result<Sql, QueryError> {
    let mut s = Sql::default();
    if let Some(alias) = l.alias {
        let col = format!("{}.`value`", t.join(alias));
        match l.op {
            Op::Contains => s.push(&col).push(" = ").bind(scalar(&l.value)?),
            _ => {
                let (values, json) = list(&l.value)?;
                s.push(&col).push(" IN ").bind_list(values, json)
            }
        };
        return Ok(s);
    }
    let column = l.def.column().expect("checked when parsed");
    let col = format!("{}.{}", t.main(), quoted(column));
    // `AttributeBoolean.equal(true)` is `!= 0`: the counts behind `unread` and `starred`.
    let zero_test = match (&l.def, l.op, &l.value) {
        (AttrDef::Boolean { .. }, Op::Eq, Value::Bool(b)) => Some(if *b { Op::Ne } else { Op::Eq }),
        (AttrDef::Boolean { .. }, Op::Ne, Value::Bool(b)) => Some(if *b { Op::Eq } else { Op::Ne }),
        _ => None,
    };
    if let Some(op) = zero_test {
        s.push(&col)
            .push(" ")
            .push(op.comparison())
            .push(" ")
            .bind(SqlValue::Integer(0));
        return Ok(s);
    }
    let op = l.op;
    match op {
        Op::Eq if l.value.is_null() => s.push(&col).push(" IS NULL"),
        Op::Ne if l.value.is_null() => s.push(&col).push(" IS NOT NULL"),
        Op::In | Op::NotIn => {
            let (values, json) = list(&l.value)?;
            s.push(&col)
                .push(" ")
                .push(op.comparison())
                .push(" ")
                .bind_list(values, json)
        }
        Op::StartsWith => {
            let Value::String(prefix) = &l.value else {
                return Err(invalid("startsWith takes a string"));
            };
            let escaped: String = prefix
                .chars()
                .flat_map(|c| match c {
                    '\\' | '%' | '_' => vec!['\\', c],
                    c => vec![c],
                })
                .collect();
            s.push(&col)
                .push(" LIKE ")
                .bind(SqlValue::Text(format!("{escaped}%")))
                .push(" ESCAPE '\\'")
        }
        Op::Like => {
            let text = match &l.value {
                Value::String(v) => v.clone(),
                other => other.to_string(),
            };
            s.push(&col)
                .push(" like ")
                .bind(SqlValue::Text(format!("%{text}%")))
        }
        _ => s
            .push(&col)
            .push(" ")
            .push(op.comparison())
            .push(" ")
            .bind(scalar(&l.value)?),
    };
    Ok(s)
}

fn node_sql(n: &Node, t: Tables<'_>) -> Result<Sql, QueryError> {
    let mut s = Sql::default();
    let join = |children: &[Node], sep: &str, empty: &str| -> Result<Sql, QueryError> {
        let mut s = Sql::default();
        s.push("(");
        if children.is_empty() {
            s.push(empty);
        }
        for (i, c) in children.iter().enumerate() {
            if i > 0 {
                s.push(sep);
            }
            s.append(node_sql(c, t)?);
        }
        s.push(")");
        Ok(s)
    };
    match n {
        Node::Leaf(l) => return leaf_sql(l, t),
        Node::And(c) => s.append(join(c, " AND ", "1")?),
        Node::Or(c) => s.append(join(c, " OR ", "0")?),
        Node::Not(c) => s.push("NOT (").append(join(c, " AND ", "1")?).push(")"),
    };
    Ok(s)
}

/// The text `.search(text)` matches: `SearchMatcher`'s trimming, then one FTS phrase as a
/// prefix. `None` when nothing is left to match.
fn search_expression(text: &str) -> Option<String> {
    let t = text.trim();
    let t = t.strip_prefix(['\'', '"']).unwrap_or(t);
    let t = t.strip_suffix(['\'', '"']).unwrap_or(t);
    (!t.is_empty()).then(|| format!("\"{}\"*", t.replace('"', "\"\"")))
}

// -- Cursor ---------------------------------------------------------------------------------

/// A page boundary: the last row's value of each order column, the tiebreak id last.
fn decode_cursor(s: &str, columns: usize) -> Result<Option<Vec<Value>>, QueryError> {
    if s.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<Vec<Value>>(s) {
        Ok(v) if v.len() == columns => Ok(Some(v)),
        _ => Err(invalid("cursor is not one this query returned")),
    }
}

/// Rows after `values` in `orders`, as SQLite orders them (NULL is the smallest value).
fn after_cursor(
    orders: &[(String, Direction)],
    values: &[Value],
    t: Tables<'_>,
) -> Result<Sql, QueryError> {
    let col = |c: &str| format!("{}.{}", t.main(), quoted(c));
    let mut s = Sql::default();
    s.push("(");
    for i in 0..orders.len() {
        if i > 0 {
            s.push(" OR ");
        }
        s.push("(");
        for (c, v) in orders[..i].iter().map(|(c, _)| c).zip(values) {
            s.push(&col(c)).push(" IS ").bind(scalar(v)?).push(" AND ");
        }
        let (c, dir) = &orders[i];
        let v = &values[i];
        match (dir, v.is_null()) {
            (Direction::Desc, false) => s
                .push("(")
                .push(&col(c))
                .push(" < ")
                .bind(scalar(v)?)
                .push(" OR ")
                .push(&col(c))
                .push(" IS NULL)"),
            (Direction::Desc, true) => s.push("0"),
            (Direction::Asc, false) => s.push(&col(c)).push(" > ").bind(scalar(v)?),
            (Direction::Asc, true) => s.push(&col(c)).push(" IS NOT NULL"),
        };
        s.push(")");
    }
    s.push(")");
    Ok(s)
}

// -- Compile --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Rows,
    Ids,
    Count,
}

/// `query.search`: the plain prefix match of `.search(text)` on the class's FTS table, or
/// the search grammar's condition on `Thread`.
enum SearchClause {
    Fts(&'static str, String),
    Grammar(search_grammar::Where),
}

/// A compiled query, for running or for `EXPLAIN QUERY PLAN`.
pub struct Compiled {
    pub sql: String,
    pub params: Vec<SqlValue>,
    output: Output,
    class: String,
    /// The joined-data attributes selected after `data`, by result key.
    includes: Vec<String>,
    /// Cursor queries: the order columns selected after the includes, and the page size.
    cursor: Option<(usize, i64)>,
}

/// Compiles `q` without running it.
pub fn compile(q: &ModelQuery) -> Result<Compiled, QueryError> {
    let table = allowlist();
    let class = q.class.as_str();
    let def = table
        .classes
        .get(class)
        .ok_or_else(|| invalid(format!("query has no class {class:?}")))?;

    let output = match (q.count == Some(true), q.ids_only == Some(true)) {
        (true, true) => return Err(invalid("count and idsOnly exclude each other")),
        (true, false) => Output::Count,
        (false, true) => Output::Ids,
        (false, false) => Output::Rows,
    };
    if q.limit.is_some_and(|l| l < 0) || q.offset.is_some_and(|o| o < 0) {
        return Err(invalid("limit and offset can't be negative"));
    }

    let mut parser = Parser {
        class,
        def,
        joins: Vec::new(),
    };
    let nodes = match &q.where_ {
        Some(Value::Null) | None => Vec::new(),
        Some(v) => parser.top(v)?,
    };
    let joins = parser.joins;

    let search = match &q.search {
        None => None,
        Some(s) if s.grammar => {
            if class != "Thread" {
                return Err(invalid(format!(
                    "the search grammar finds threads; search {class} with grammar: false"
                )));
            }
            let grammar_error = |e: GrammarError| invalid(format!("search: {e}"));
            let dates = DateContext::new(s.tz.as_deref(), s.now);
            let w = search_grammar::thread_condition(&s.text, &dates).map_err(grammar_error)?;
            Some(SearchClause::Grammar(w))
        }
        Some(s) => {
            let fts = search_table(class)
                .ok_or_else(|| invalid(format!("{class} has no search index")))?;
            let expr = search_expression(&s.text)
                .ok_or_else(|| invalid("search.text has nothing to match"))?;
            Some(SearchClause::Fts(fts, expr))
        }
    };

    let mut orders: Vec<(String, Direction)> = Vec::new();
    let requested = q.order.as_deref().unwrap_or_default();
    if requested.is_empty() {
        if let Some(n) = &def.natural_order {
            orders.push((n.attr.clone(), n.dir));
        }
    }
    for o in requested {
        let column = def
            .attributes
            .get(&o.attr)
            .filter(|_| !UNSTORED.contains(&(class, o.attr.as_str())))
            .and_then(AttrDef::column);
        let natural = def.natural_order.as_ref().is_some_and(|n| n.attr == o.attr);
        match column {
            Some(c) => orders.push((c.to_owned(), o.dir)),
            None if natural => orders.push((o.attr.clone(), o.dir)),
            None => return Err(invalid(format!("{class} can't be ordered by {:?}", o.attr))),
        }
    }

    let mut includes: Vec<(String, String)> = Vec::new();
    for name in q.include.as_deref().unwrap_or_default() {
        match def.attributes.get(name) {
            Some(AttrDef::JoinedData { table }) => {
                if !includes.iter().any(|(n, _)| n == name) {
                    includes.push((name.clone(), table.clone()));
                }
            }
            _ => return Err(invalid(format!("{class} has no joined data {name:?}"))),
        }
    }
    if output != Output::Rows {
        includes.clear();
    }

    let cursor = match &q.cursor {
        None => None,
        Some(c) => {
            if output != Output::Rows {
                return Err(invalid("cursor only pages rows, not ids or counts"));
            }
            if q.offset.is_some_and(|o| o > 0) {
                return Err(invalid("cursor and offset exclude each other"));
            }
            let limit = q
                .limit
                .filter(|l| *l > 0)
                .ok_or_else(|| invalid("a cursor query needs a limit"))?;
            if !orders.iter().any(|(c, _)| c == "id") {
                let dir = orders.last().map_or(Direction::Desc, |(_, d)| *d);
                orders.push(("id".into(), dir));
            }
            Some((decode_cursor(c, orders.len())?, limit))
        }
    };

    let distinct = q.distinct == Some(true);
    let quoted_table = quoted(class);
    let main = Tables::Main(class);

    // `Query._canSubselectForJoin`: one collection matcher, a limit, and every other
    // matcher and order on the join table's columns. The thread list's plan, which reads
    // ThreadListCategoryIndex instead of sorting the category's threads.
    let mut leaves = Vec::new();
    nodes.iter().for_each(|n| n.leaves(&mut leaves));
    let collections: Vec<&Leaf> = leaves
        .iter()
        .copied()
        .filter(|l| l.alias.is_some())
        .collect();
    let subselect = match collections.as_slice() {
        [only] if search.is_none() && q.limit.is_some() => {
            let AttrDef::Collection {
                join_table,
                join_queryable_by,
            } = &only.def
            else {
                unreachable!()
            };
            let on_join = |attr: &str| join_queryable_by.iter().any(|a| a == attr);
            let matchers_ok = leaves
                .iter()
                .all(|l| l.alias.is_some() || l.attr == "id" || on_join(&l.attr));
            let orders_ok = orders
                .iter()
                .all(|(c, _)| on_join(c) || (c == "id" && cursor.is_some()));
            // `accounts` can only be narrowed inside the subselect on ThreadCategory (below);
            // on other join tables the inner LIMIT would count other accounts' rows.
            let accounts_ok = q.accounts.is_none() || join_table == THREAD_CATEGORY;
            // Note: duplicate rows from overlapping categories collapse in the subselect, so
            // a distinct cursor query reads through the join to keep its page size exact.
            (matchers_ok && orders_ok && accounts_ok && !(cursor.is_some() && distinct))
                .then(|| join_table.clone())
        }
        _ => None,
    };

    let page_limit = cursor.as_ref().map(|(_, l)| l + 1).or(q.limit);
    let limit_sql = |s: &mut Sql| {
        match (page_limit, q.offset) {
            (Some(l), Some(o)) => s
                .push(" LIMIT ")
                .bind(SqlValue::Integer(l))
                .push(" OFFSET ")
                .bind(SqlValue::Integer(o)),
            (Some(l), None) => s.push(" LIMIT ").bind(SqlValue::Integer(l)),
            (None, Some(o)) => s.push(" LIMIT -1 OFFSET ").bind(SqlValue::Integer(o)),
            (None, None) => s,
        };
    };
    let order_sql = |s: &mut Sql, t: Tables<'_>| {
        if output == Output::Count || orders.is_empty() {
            return;
        }
        s.push(" ORDER BY ");
        for (i, (c, dir)) in orders.iter().enumerate() {
            if i > 0 {
                s.push(", ");
            }
            s.push(&format!("{}.{} {}", t.main(), quoted(c), dir.sql()));
        }
    };
    let accounts_sql = |s: &mut Sql| {
        if let Some(accounts) = &q.accounts {
            let json: Vec<Value> = accounts.iter().cloned().map(Value::String).collect();
            let values = accounts.iter().cloned().map(SqlValue::Text).collect();
            s.push(&format!("{quoted_table}.`accountId` IN "))
                .bind_list(values, &json);
        }
    };

    let mut sql = Sql::default();
    sql.push("SELECT ");
    if distinct {
        sql.push("DISTINCT ");
    }
    match output {
        Output::Count => sql.push("COUNT(*)"),
        Output::Ids => sql.push(&format!("{quoted_table}.`id`")),
        Output::Rows => sql.push(&format!("{quoted_table}.`data`")),
    };
    for (_, t) in &includes {
        sql.push(&format!(", {}.`value`", quoted(t)));
    }
    if cursor.is_some() {
        for (c, _) in &orders {
            sql.push(&format!(", {quoted_table}.{}", quoted(c)));
        }
    }
    sql.push(&format!(" FROM {quoted_table}"));
    if subselect.is_none() {
        for (i, jt) in joins.iter().enumerate() {
            let alias = quoted(&format!("M{i}"));
            sql.push(&format!(
                " INNER JOIN {} AS {alias} ON {alias}.`id` = {quoted_table}.`id`",
                quoted(jt)
            ));
        }
    }
    for (_, t) in &includes {
        let t = quoted(t);
        sql.push(&format!(
            " LEFT OUTER JOIN {t} ON {t}.`id` = {quoted_table}.`id`"
        ));
    }

    match &subselect {
        Some(jt) => {
            let inner_tables = Tables::JoinTable(jt);
            // A cursor page counts threads, so the subselect must not return one twice: it
            // can unless its only collection matcher is top level with one value, as each
            // (thread, value) pair is one row. Every row of a thread carries the same order
            // keys, so deduping on `id` keeps the order.
            let one_row_per_id = nodes.iter().any(|n| {
                matches!(n, Node::Leaf(l) if l.alias.is_some()
                    && (l.op == Op::Contains
                        || l.value.as_array().is_some_and(|v| v.len() == 1)))
            });
            let mut inner = Sql::default();
            inner.push(if cursor.is_some() && !one_row_per_id {
                "SELECT DISTINCT `id` FROM "
            } else {
                "SELECT `id` FROM "
            });
            inner.push(&format!("{} WHERE ", quoted(jt)));
            let mut first = true;
            for n in &nodes {
                if !first {
                    inner.push(" AND ");
                }
                first = false;
                inner.append(node_sql(n, inner_tables)?);
            }
            if let Some((Some(values), _)) = &cursor {
                if !first {
                    inner.push(" AND ");
                }
                inner.append(after_cursor(&orders, values, inner_tables)?);
            }
            if let Some(accounts) = &q.accounts {
                // A thread's categories are its own account's, so keeping the rows of the
                // accounts' folders and labels keeps exactly the accounts' threads, before
                // the LIMIT and OFFSET count them.
                let json: Vec<Value> = accounts.iter().cloned().map(Value::String).collect();
                let values: Vec<SqlValue> = accounts.iter().cloned().map(SqlValue::Text).collect();
                inner
                    .push(&format!(
                        " AND {}.`value` IN (SELECT `id` FROM `Folder` WHERE `accountId` IN ",
                        quoted(jt)
                    ))
                    .bind_list(values.clone(), &json)
                    .push(" UNION ALL SELECT `id` FROM `Label` WHERE `accountId` IN ")
                    .bind_list(values, &json)
                    .push(")");
            }
            order_sql(&mut inner, inner_tables);
            limit_sql(&mut inner);
            sql.push(&format!(" WHERE {quoted_table}.`id` IN ("))
                .append(inner)
                .push(")");
            if q.accounts.is_some() {
                sql.push(" AND ");
                accounts_sql(&mut sql);
            }
            order_sql(&mut sql, main);
            if cursor.is_some() {
                // The subselect returns each thread once (above), so this is the page.
                sql.push(" LIMIT ")
                    .bind(SqlValue::Integer(page_limit.unwrap_or(-1)));
            }
        }
        None => {
            let mut wheres: Vec<Sql> = Vec::new();
            for n in &nodes {
                wheres.push(node_sql(n, main)?);
            }
            match search {
                Some(SearchClause::Fts(fts, expr)) => {
                    let mut s = Sql::default();
                    s.push(&format!(
                        "{quoted_table}.`id` IN (SELECT `content_id` FROM {f} WHERE {f} MATCH ",
                        f = quoted(fts)
                    ))
                    .bind(SqlValue::Text(expr))
                    .push(" LIMIT 1000)");
                    wheres.push(s);
                }
                Some(SearchClause::Grammar(w)) => wheres.push(Sql {
                    text: w.sql,
                    params: w.params,
                }),
                None => {}
            }
            if q.accounts.is_some() {
                let mut s = Sql::default();
                accounts_sql(&mut s);
                wheres.push(s);
            }
            if let Some((Some(values), _)) = &cursor {
                wheres.push(after_cursor(&orders, values, main)?);
            }
            for (i, w) in wheres.into_iter().enumerate() {
                sql.push(if i == 0 { " WHERE " } else { " AND " }).append(w);
            }
            order_sql(&mut sql, main);
            limit_sql(&mut sql);
        }
    }

    Ok(Compiled {
        sql: sql.text,
        params: sql.params,
        output,
        class: class.to_owned(),
        includes: includes.into_iter().map(|(name, _)| name).collect(),
        cursor: cursor.map(|(_, limit)| (orders.len(), limit)),
    })
}

fn json_column(row: &Row<'_>, idx: usize) -> rusqlite::Result<Value> {
    Ok(match row.get_ref(idx)? {
        ValueRef::Text(b) | ValueRef::Blob(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

fn sql_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(b) | ValueRef::Blob(b) => Value::String(String::from_utf8_lossy(b).into()),
    }
}

impl ReadStore {
    /// Runs a `query` (see the module doc).
    pub fn model_query(&self, q: &ModelQuery) -> Result<QueryOutput, QueryError> {
        let c = compile(q)?;
        let mut stmt = self.conn().prepare(&c.sql)?;
        let mut rows = stmt.query(params_from_iter(c.params.iter()))?;
        match c.output {
            Output::Count => {
                let n = match rows.next()? {
                    Some(r) => r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                    None => 0,
                };
                Ok(QueryOutput::Count(n))
            }
            Output::Ids => {
                let mut ids = Vec::new();
                while let Some(r) = rows.next()? {
                    ids.push(r.get::<_, String>(0)?);
                }
                Ok(QueryOutput::Ids(ids))
            }
            Output::Rows => {
                let message = c.class == "Message";
                let mut out = Vec::new();
                let mut keys: Vec<Vec<Value>> = Vec::new();
                while let Some(r) = rows.next()? {
                    let mut model = json_column(r, 0)?;
                    if message {
                        model = with_hidden(model);
                    }
                    for (i, name) in c.includes.iter().enumerate() {
                        let v = r.get::<_, Option<String>>(1 + i)?;
                        if let Some(o) = model.as_object_mut() {
                            o.insert(name.clone(), v.map_or(Value::Null, Value::String));
                        }
                    }
                    if let Some((columns, _)) = c.cursor {
                        let start = 1 + c.includes.len();
                        keys.push(
                            (start..start + columns)
                                .map(|i| r.get_ref(i).map(sql_to_json))
                                .collect::<rusqlite::Result<_>>()?,
                        );
                    }
                    out.push(model);
                }
                let next_cursor = c.cursor.map(|(_, limit)| {
                    let more = out.len() as i64 > limit;
                    out.truncate(limit as usize);
                    keys.truncate(limit as usize);
                    more.then(|| keys.last().map(|k| Value::Array(k.clone()).to_string()))
                        .flatten()
                });
                Ok(QueryOutput::Rows {
                    rows: out,
                    next_cursor,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn query(v: Value) -> ModelQuery {
        serde_json::from_value(v).unwrap()
    }

    fn sql(v: Value) -> (String, Vec<SqlValue>) {
        let c = compile(&query(v)).unwrap();
        (c.sql, c.params)
    }

    fn error(v: Value) -> String {
        match compile(&query(v)) {
            Err(e) => e.to_string(),
            Ok(c) => panic!("compiled: {}", c.sql),
        }
    }

    #[test]
    fn allowlist_loads() {
        let t = allowlist();
        assert!(t.classes["Thread"].attributes.contains_key("categories"));
        assert!(matches!(
            t.classes["Message"].attributes["body"],
            AttrDef::JoinedData { .. }
        ));
    }

    #[test]
    fn thread_list_takes_the_subselect() {
        let (text, params) = sql(json!({
            "class": "Thread",
            "where": {"and": [
                {"attr": "categories", "op": "containsAny", "value": ["c1", "c2"]},
                {"attr": "inAllMail", "op": "!=", "value": 0},
            ]},
            "limit": 50, "offset": 100,
        }));
        assert_eq!(
            text,
            "SELECT `Thread`.`data` FROM `Thread` WHERE `Thread`.`id` IN (SELECT `id` FROM \
             `ThreadCategory` WHERE `ThreadCategory`.`value` IN (?,?) AND \
             `ThreadCategory`.`inAllMail` != ? ORDER BY \
             `ThreadCategory`.`lastMessageReceivedTimestamp` DESC LIMIT ? OFFSET ?) \
             ORDER BY `Thread`.`lastMessageReceivedTimestamp` DESC"
        );
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn other_matchers_join() {
        let (text, _) = sql(json!({
            "class": "Thread",
            "accounts": ["a1"],
            "where": [
                {"attr": "categories", "op": "contains", "value": "c1"},
                {"attr": "subject", "op": "startsWith", "value": "50%_off"},
            ],
            "limit": 5,
        }));
        assert!(
            text.contains("INNER JOIN `ThreadCategory` AS `M0`"),
            "{text}"
        );
        assert!(
            text.contains("`Thread`.`subject` LIKE ? ESCAPE '\\'"),
            "{text}"
        );
        assert!(text.contains("`Thread`.`accountId` IN (?)"), "{text}");
    }

    #[test]
    fn booleans_mean_non_zero() {
        let (text, params) = sql(json!({
            "class": "Thread",
            "where": {"attr": "unread", "op": "=", "value": true},
        }));
        assert!(text.contains("`Thread`.`unread` != ?"), "{text}");
        assert_eq!(params, vec![SqlValue::Integer(0)]);
        let (text, _) = sql(json!({
            "class": "Thread",
            "where": {"attr": "starred", "op": "!=", "value": true},
        }));
        assert!(text.contains("`Thread`.`starred` = ?"), "{text}");
    }

    #[test]
    fn null_comparisons_and_composites() {
        let (text, _) = sql(json!({
            "class": "Message",
            "where": {"or": [
                {"attr": "threadId", "op": "=", "value": null},
                {"not": [{"attr": "draft", "op": "!=", "value": null}]},
                {"and": []},
            ]},
        }));
        assert!(
            text.contains(
                "WHERE (`Message`.`threadId` IS NULL OR NOT ((`Message`.`draft` IS NOT NULL)) OR (1))"
            ),
            "{text}"
        );
        assert!(text.ends_with("ORDER BY `Message`.`date` ASC"), "{text}");
    }

    #[test]
    fn long_lists_go_through_json_each() {
        let ids: Vec<String> = (0..600).map(|i| format!("t{i}")).collect();
        let (text, params) = sql(json!({
            "class": "Thread",
            "where": {"attr": "id", "op": "in", "value": ids},
        }));
        assert!(
            text.contains("IN (SELECT value FROM json_each(?))"),
            "{text}"
        );
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn search_is_a_quoted_prefix() {
        assert_eq!(search_expression(" 'ann "), Some("\"ann\"*".into()));
        assert_eq!(search_expression("\"a\"b\""), Some("\"a\"\"b\"*".into()));
        assert_eq!(search_expression(" '' "), None);
        let (text, params) = sql(json!({
            "class": "Contact",
            "search": {"text": "ann", "grammar": false},
            "limit": 10,
        }));
        assert!(
            text.contains(
                "`Contact`.`id` IN (SELECT `content_id` FROM `ContactSearch` WHERE \
                 `ContactSearch` MATCH ? LIMIT 1000)"
            ),
            "{text}"
        );
        assert_eq!(params[0], SqlValue::Text("\"ann\"*".into()));
    }

    #[test]
    fn grammar_search_is_a_thread_condition() {
        let (text, params) = sql(json!({
            "class": "Thread", "accounts": ["a1"],
            "search": {"text": "from:ann after:yesterday", "grammar": true,
                       "tz": "Europe/Lisbon", "now": 1_791_990_000},
            "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "desc"}],
            "limit": 5,
        }));
        assert!(
            text.contains(
                "WHERE ((`Thread`.`id` IN (SELECT `content_id` FROM `ThreadSearch` WHERE \
                 `ThreadSearch` MATCH ?)) AND (`Thread`.`lastMessageReceivedTimestamp` >= ?)) \
                 AND `Thread`.`accountId` IN (?)"
            ),
            "{text}"
        );
        // 2026-10-13 00:00 in Lisbon (UTC+1).
        assert_eq!(
            params[..3],
            [
                SqlValue::Text("(from_ : \"ann\"*)".into()),
                SqlValue::Integer(1_791_846_000),
                SqlValue::Text("a1".into())
            ]
        );
    }

    #[test]
    fn refusals() {
        assert!(error(
            json!({"class": "Contact", "search": {"text": "from:ann", "grammar": true}})
        )
        .contains("finds threads"));
        assert!(error(
            json!({"class": "Thread", "search": {"text": "after:someday", "grammar": true}})
        )
        .contains("isn't a date"));
        // A bad zone fails the date terms that need it, not a search by words.
        assert!(error(json!({"class": "Thread", "search":
            {"text": "x after:2024-03-01", "grammar": true, "tz": "Nowhere"}}))
        .contains("IANA"));
        compile(&query(json!({"class": "Thread", "search":
            {"text": "x", "grammar": true, "tz": "UTC0"}})))
        .unwrap();
        assert!(error(json!({"class": "Account"})).contains("no class"));
        assert!(error(
            json!({"class": "Thread", "where": {"attr": "snippet", "op": "=", "value": "x"}})
        )
        .contains("no queryable attribute"));
        assert!(error(
            json!({"class": "Event", "where": {"attr": "status", "op": "=", "value": "x"}})
        )
        .contains("no queryable attribute"));
        assert!(
            error(json!({"class": "Event", "order": [{"attr": "status", "dir": "asc"}]}))
                .contains("can't be ordered")
        );
        assert!(error(
            json!({"class": "Thread", "where": {"attr": "categories", "op": "=", "value": "x"}})
        )
        .contains("collection"));
        assert!(error(
            json!({"class": "Thread", "where": {"attr": "subject", "op": "contains", "value": "x"}})
        )
        .contains("not a collection"));
        assert!(error(
            json!({"class": "Thread", "where": {"attr": "subject", "op": "~", "value": "x"}})
        )
        .contains("unknown op"));
        assert!(error(
            json!({"class": "Thread", "where": {"attr": "id", "op": "in", "value": "x"}})
        )
        .contains("list"));
        assert!(
            error(json!({"class": "Thread", "order": [{"attr": "snippet", "dir": "asc"}]}))
                .contains("ordered")
        );
        assert!(error(json!({"class": "Message", "include": ["files"]})).contains("joined data"));
        assert!(error(json!({"class": "Folder", "search": {"text": "x"}})).contains("search index"));
        assert!(error(json!({"class": "Thread", "cursor": ""})).contains("limit"));
        assert!(
            error(json!({"class": "Thread", "count": true, "idsOnly": true})).contains("exclude")
        );
        assert!(
            serde_json::from_value::<ModelQuery>(json!({"class": "Thread", "nope": 1})).is_err()
        );
    }

    #[test]
    fn natural_order_attribute_is_orderable() {
        // ContactGroup orders by `name`, which isn't queryable.
        let (text, _) =
            sql(json!({"class": "ContactGroup", "order": [{"attr": "name", "dir": "asc"}]}));
        assert!(
            text.ends_with("ORDER BY `ContactGroup`.`name` ASC"),
            "{text}"
        );
    }

    #[test]
    fn cursor_adds_the_tiebreak() {
        let (text, _) = sql(json!({
            "class": "Thread",
            "where": [{"attr": "categories", "op": "containsAny", "value": ["c1"]}],
            "limit": 2, "cursor": "[1700000000,\"t9\"]",
        }));
        assert!(
            text.contains(
                "ORDER BY `ThreadCategory`.`lastMessageReceivedTimestamp` DESC, \
                 `ThreadCategory`.`id` DESC LIMIT ?)"
            ),
            "{text}"
        );
        assert!(
            text.contains("`ThreadCategory`.`lastMessageReceivedTimestamp` IS ?"),
            "{text}"
        );
        assert!(error(json!({"class": "Thread", "limit": 2, "cursor": "[1]"})).contains("cursor"));
    }
}
