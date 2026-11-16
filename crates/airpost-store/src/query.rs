// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! A WHERE clause builder. Derived from Mailspring-Sync `MailSync/Query.*`, Copyright ©
//! 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Clauses are keyed by column, so a second clause on the same column replaces the first,
//! and they are emitted in column-name order (the C++ kept them in a sorted JSON object).

use std::collections::BTreeMap;

use rusqlite::types::Value as SqlValue;

/// A value a clause compares against: strings, integers, floats and bools.
pub trait QueryValue {
    fn into_sql(self) -> SqlValue;
}

impl QueryValue for &str {
    fn into_sql(self) -> SqlValue {
        SqlValue::Text(self.to_owned())
    }
}

impl QueryValue for &String {
    fn into_sql(self) -> SqlValue {
        SqlValue::Text(self.clone())
    }
}

impl QueryValue for String {
    fn into_sql(self) -> SqlValue {
        SqlValue::Text(self)
    }
}

impl QueryValue for f64 {
    fn into_sql(self) -> SqlValue {
        SqlValue::Real(self)
    }
}

impl QueryValue for bool {
    fn into_sql(self) -> SqlValue {
        SqlValue::Integer(self as i64)
    }
}

macro_rules! int_query_value {
    ($($t:ty),*) => {$(
        impl QueryValue for $t {
            fn into_sql(self) -> SqlValue {
                SqlValue::Integer(self as i64)
            }
        }
    )*};
}
int_query_value!(i32, i64, u32);

#[derive(Clone, Debug, PartialEq)]
enum Clause {
    Eq(SqlValue),
    In(Vec<SqlValue>),
    Op(&'static str, SqlValue),
    Between(SqlValue, SqlValue),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    clauses: BTreeMap<String, Clause>,
    limit: Option<usize>,
    order_by: Option<(String, &'static str)>,
}

impl Query {
    pub fn new() -> Self {
        Query::default()
    }

    fn set(mut self, col: &str, clause: Clause) -> Self {
        self.clauses.insert(col.to_owned(), clause);
        self
    }

    pub fn equal(self, col: &str, value: impl QueryValue) -> Self {
        self.set(col, Clause::Eq(value.into_sql()))
    }

    /// `col IN (…)`. An empty list matches nothing.
    pub fn equal_any<V: QueryValue>(self, col: &str, values: impl IntoIterator<Item = V>) -> Self {
        let values = values.into_iter().map(QueryValue::into_sql).collect();
        self.set(col, Clause::In(values))
    }

    pub fn gt(self, col: &str, value: impl QueryValue) -> Self {
        self.set(col, Clause::Op(">", value.into_sql()))
    }

    pub fn gte(self, col: &str, value: impl QueryValue) -> Self {
        self.set(col, Clause::Op(">=", value.into_sql()))
    }

    pub fn lt(self, col: &str, value: impl QueryValue) -> Self {
        self.set(col, Clause::Op("<", value.into_sql()))
    }

    pub fn lte(self, col: &str, value: impl QueryValue) -> Self {
        self.set(col, Clause::Op("<=", value.into_sql()))
    }

    /// Both ends inclusive.
    pub fn between_inclusive(self, col: &str, lo: impl QueryValue, hi: impl QueryValue) -> Self {
        self.set(col, Clause::Between(lo.into_sql(), hi.into_sql()))
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn order_by(mut self, col: &str, ascending: bool) -> Self {
        self.order_by = Some((col.to_owned(), if ascending { "ASC" } else { "DESC" }));
        self
    }

    pub fn get_limit(&self) -> Option<usize> {
        self.limit
    }

    /// `" WHERE … ORDER BY … LIMIT …"` (empty when there is nothing to add) and the
    /// positional parameters in order. Column names are not quoted, as in the C++.
    pub fn to_sql(&self) -> (String, Vec<SqlValue>) {
        let (mut sql, params) = self.where_sql();
        if let Some((col, dir)) = &self.order_by {
            sql.push_str(&format!(" ORDER BY {col} {dir}"));
        }
        if let Some(n) = self.limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        (sql, params)
    }

    /// `" WHERE …"` alone, for statements that take no ORDER BY or LIMIT (DELETE, COUNT,
    /// and `find`, which adds its own `LIMIT 1`).
    pub fn where_sql(&self) -> (String, Vec<SqlValue>) {
        let mut sql = String::new();
        let mut params = Vec::new();
        for (i, (col, clause)) in self.clauses.iter().enumerate() {
            sql.push_str(if i == 0 { " WHERE " } else { " AND " });
            match clause {
                Clause::Eq(v) => {
                    sql.push_str(&format!("{col} = ?"));
                    params.push(v.clone());
                }
                Clause::In(vs) if vs.is_empty() => sql.push_str("0 = 1"),
                Clause::In(vs) => {
                    let marks = vec!["?"; vs.len()].join(",");
                    sql.push_str(&format!("{col} IN ({marks})"));
                    params.extend(vs.iter().cloned());
                }
                Clause::Op(op, v) => {
                    sql.push_str(&format!("{col} {op} ?"));
                    params.push(v.clone());
                }
                Clause::Between(lo, hi) => {
                    sql.push_str(&format!("{col} BETWEEN ? AND ?"));
                    params.push(lo.clone());
                    params.push(hi.clone());
                }
            }
        }
        (sql, params)
    }

    /// `" ORDER BY …"` or "".
    pub fn order_sql(&self) -> String {
        match &self.order_by {
            Some((col, dir)) => format!(" ORDER BY {col} {dir}"),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sql() {
        let q = Query::new()
            .equal("accountId", "a1")
            .equal_any("id", ["x", "y"])
            .gt("date", 5)
            .equal("accountId", "a2")
            .order_by("date", false)
            .limit(10);
        let (sql, params) = q.to_sql();
        assert_eq!(
            sql,
            " WHERE accountId = ? AND date > ? AND id IN (?,?) ORDER BY date DESC LIMIT 10"
        );
        assert_eq!(params.len(), 4);
        assert_eq!(params[0], SqlValue::Text("a2".into()));
        assert_eq!(
            Query::new().equal_any::<i64>("id", []).to_sql().0,
            " WHERE 0 = 1"
        );
    }
}
