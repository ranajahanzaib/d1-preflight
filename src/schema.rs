use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Column {
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub default: Option<String>,
    pub pk: i64,
    pub hidden: i64,
}
pub type IndexTerm = (i64, Option<String>, i64, Option<String>, i64);
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Index {
    pub unique: bool,
    pub origin: String,
    pub partial: bool,
    pub terms: Vec<IndexTerm>,
    pub sql: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForeignKey {
    pub id: i64,
    pub seq: i64,
    pub parent: String,
    pub from: String,
    pub to: Option<String>,
    pub on_update: String,
    pub on_delete: String,
    pub match_kind: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Table {
    pub columns: Vec<Column>,
    pub indexes: BTreeMap<String, Index>,
    pub foreign_keys: Vec<ForeignKey>,
    pub sql: String,
    pub strict: bool,
    pub without_rowid: bool,
    pub rows: i64,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
    pub objects: BTreeMap<String, (String, String)>,
}
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn snapshot(c: &Connection, tracking: &str) -> Result<Schema> {
    let mut schema = Schema::default();
    let mut stmt=c.prepare("SELECT type,name,coalesce(sql,'') FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let objects = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (kind, name, sql) in objects {
        if name == tracking || name.starts_with("_cf_") {
            continue;
        }
        if kind == "view" || kind == "trigger" {
            schema.objects.insert(name, (kind, sql));
            continue;
        }
        if kind != "table" {
            continue;
        }
        let columns = c
            .prepare(
                "SELECT name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?1)",
            )?
            .query_map([&name], |r| {
                Ok(Column {
                    name: r.get(0)?,
                    declared_type: r.get(1)?,
                    not_null: r.get(2)?,
                    default: r.get(3)?,
                    pk: r.get(4)?,
                    hidden: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut indexes = BTreeMap::new();
        let ix = c
            .prepare(
                "SELECT name,\"unique\",origin,partial FROM pragma_index_list(?1) ORDER BY name",
            )?
            .query_map([&name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (n, unique, origin, partial) in ix {
            let terms = c
                .prepare(
                    "SELECT cid,name,desc,coll,\"key\" FROM pragma_index_xinfo(?1) ORDER BY seqno",
                )?
                .query_map([&n], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let sql = c
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='index' AND name=?1",
                    [&n],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            indexes.insert(
                n,
                Index {
                    unique,
                    origin,
                    partial,
                    terms,
                    sql,
                },
            );
        }
        let foreign_keys=c.prepare("SELECT id,seq,\"table\",\"from\",\"to\",on_update,on_delete,\"match\" FROM pragma_foreign_key_list(?1) ORDER BY id,seq")?.query_map([&name],|r|Ok(ForeignKey{id:r.get(0)?,seq:r.get(1)?,parent:r.get(2)?,from:r.get(3)?,to:r.get(4)?,on_update:r.get(5)?,on_delete:r.get(6)?,match_kind:r.get(7)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let (without_rowid, strict) = c.query_row(
            "SELECT wr,strict FROM pragma_table_list WHERE schema='main' AND name=?1",
            [&name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let rows = c.query_row(&format!("SELECT count(*) FROM {}", quote(&name)), [], |r| {
            r.get(0)
        })?;
        schema.tables.insert(
            name,
            Table {
                columns,
                indexes,
                foreign_keys,
                sql,
                strict,
                without_rowid,
                rows,
            },
        );
    }
    Ok(schema)
}
/// Token-based normalization preserves quoted values and expression punctuation.
pub fn normalized(sql: &str) -> String {
    use sqlparser::{dialect::SQLiteDialect, tokenizer::Tokenizer};
    match Tokenizer::new(&SQLiteDialect {}, sql).tokenize() {
        Ok(t) => t
            .into_iter()
            .filter(|t| !matches!(t, sqlparser::tokenizer::Token::Whitespace(_)))
            .map(|mut t| {
                if let sqlparser::tokenizer::Token::Word(w) = &mut t
                    && w.quote_style.is_none()
                {
                    w.value.make_ascii_uppercase();
                }
                t.to_string()
            })
            .collect::<Vec<_>>()
            .join(" "),
        Err(_) => sql.into(),
    }
}
