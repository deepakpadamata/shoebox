//! The service's records in the Durable Object's SQLite database. A row holds at most 1 MiB (SQLite in a Durable
//! Object stores at most 2 MB a row), so a large record (a share listing 100,000 blobs) is kept in parts.
//! Lines appended to a record (`index.log`) are rows of their own until the record is next written whole.

use shoebox_cloud::meta::Meta;
use std::collections::BTreeSet;
use std::io;
use worker::{SqlStorage, SqlStorageValue};

const PART: usize = 1 << 20;

pub struct SqlRecords {
    sql: SqlStorage,
}

fn io_err(e: worker::Error) -> io::Error {
    io::Error::other(e.to_string())
}

impl SqlRecords {
    pub fn new(sql: SqlStorage) -> worker::Result<Self> {
        sql.exec(
            "CREATE TABLE IF NOT EXISTS records (key TEXT NOT NULL, part INTEGER NOT NULL, value BLOB NOT NULL, \
             PRIMARY KEY (key, part))",
            None,
        )?;
        sql.exec(
            "CREATE TABLE IF NOT EXISTS lines (seq INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL, line TEXT NOT NULL)",
            None,
        )?;
        sql.exec(
            "CREATE INDEX IF NOT EXISTS lines_key ON lines (key, seq)",
            None,
        )?;
        Ok(Self { sql })
    }

    fn rows(
        &self,
        query: &str,
        args: Vec<SqlStorageValue>,
    ) -> io::Result<Vec<Vec<SqlStorageValue>>> {
        self.sql
            .exec(query, args)
            .map_err(io_err)?
            .raw()
            .collect::<worker::Result<Vec<_>>>()
            .map_err(io_err)
    }
}

impl Meta for SqlRecords {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        let parts = self.rows(
            "SELECT value FROM records WHERE key = ? ORDER BY part",
            vec![key.into()],
        )?;
        let lines = self.rows(
            "SELECT line FROM lines WHERE key = ? ORDER BY seq",
            vec![key.into()],
        )?;
        if parts.is_empty() && lines.is_empty() {
            return Ok(None);
        }
        let mut out = vec![];
        for row in parts {
            if let Some(SqlStorageValue::Blob(b)) = row.into_iter().next() {
                out.extend_from_slice(&b);
            }
        }
        for row in lines {
            if let Some(SqlStorageValue::String(l)) = row.into_iter().next() {
                out.extend_from_slice(l.as_bytes());
                out.push(b'\n');
            }
        }
        Ok(Some(out))
    }

    fn put(&self, key: &str, value: &[u8]) -> io::Result<()> {
        self.delete(key)?;
        let mut parts: Vec<&[u8]> = value.chunks(PART).collect();
        if parts.is_empty() {
            parts.push(&[]);
        }
        for (i, part) in parts.into_iter().enumerate() {
            self.sql
                .exec(
                    "INSERT INTO records (key, part, value) VALUES (?, ?, ?)",
                    vec![key.into(), (i as i64).into(), part.to_vec().into()],
                )
                .map_err(io_err)?;
        }
        Ok(())
    }

    fn append(&self, key: &str, line: &str) -> io::Result<()> {
        self.sql
            .exec(
                "INSERT INTO lines (key, line) VALUES (?, ?)",
                vec![key.into(), line.into()],
            )
            .map_err(io_err)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        self.sql
            .exec("DELETE FROM records WHERE key = ?", vec![key.into()])
            .map_err(io_err)?;
        self.sql
            .exec("DELETE FROM lines WHERE key = ?", vec![key.into()])
            .map_err(io_err)?;
        Ok(())
    }

    fn children(&self, dir: &str) -> io::Result<Vec<String>> {
        // Keys from `dir/` up to (not including) `dir0`: '0' is the character after '/'.
        let (lo, hi) = (format!("{dir}/"), format!("{dir}0"));
        let rows = self.rows(
            "SELECT DISTINCT key FROM records WHERE key >= ? AND key < ? \
             UNION SELECT DISTINCT key FROM lines WHERE key >= ? AND key < ?",
            vec![
                lo.as_str().into(),
                hi.as_str().into(),
                lo.as_str().into(),
                hi.as_str().into(),
            ],
        )?;
        let mut out = BTreeSet::new();
        for row in rows {
            if let Some(SqlStorageValue::String(k)) = row.into_iter().next() {
                if let Some(rest) = k.strip_prefix(&lo) {
                    out.insert(rest.split('/').next().unwrap_or(rest).to_string());
                }
            }
        }
        Ok(out.into_iter().collect())
    }
}
