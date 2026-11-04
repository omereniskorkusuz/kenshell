//! Key/value settings with JSON values (P1-T04). Keys are app-defined identifiers
//! such as `ui.theme`; values are any JSON. Storing `null` removes the key so the
//! frontend falls back to its default.

use rusqlite::{OptionalExtension, params};

use super::{Db, validate};
use crate::error::{AppError, ErrorCode, Result};

fn corrupt(key: &str, e: &serde_json::Error) -> AppError {
    AppError::with(
        ErrorCode::Db,
        format!("setting `{key}` is not valid JSON: {e}"),
    )
}

impl Db {
    #[allow(dead_code, reason = "contract API; no backend consumer yet")]
    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let key = validate::setting_key(key)?;
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|text| serde_json::from_str(&text).map_err(|e| corrupt(key, &e)))
            .transpose()
    }

    /// Inserts or replaces `key`. A JSON `null` deletes the key instead.
    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let key = validate::setting_key(key)?;
        let conn = self.conn();
        if value.is_null() {
            conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
            return Ok(());
        }
        let text = serde_json::to_string(value)
            .map_err(|e| AppError::internal(format!("encode setting: {e}")))?;
        conn.execute(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, text],
        )?;
        Ok(())
    }

    /// All settings. A value that no longer parses is skipped (and logged by key only)
    /// so one damaged entry cannot block app start-up.
    pub fn all_settings(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT key, value_json FROM settings ORDER BY key")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = serde_json::Map::new();
        for row in rows {
            let (key, text) = row?;
            match serde_json::from_str(&text) {
                Ok(value) => {
                    out.insert(key, value);
                }
                Err(e) => tracing::warn!(key = %key, "skipping corrupt setting: {e}"),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::error::ErrorCode;
    use crate::storage::Db;

    #[test]
    fn get_missing_is_none() {
        assert_eq!(
            Db::open_in_memory().unwrap().get_setting("x").unwrap(),
            None
        );
    }

    #[test]
    fn json_values_round_trip_and_overwrite() {
        let db = Db::open_in_memory().unwrap();
        let values = [
            json!("dark"),
            json!(14),
            json!(1.5),
            json!(true),
            json!([1, "a", null]),
            json!({"font": {"family": "JetBrains Mono", "size": 13}, "ligatures": false}),
        ];
        for v in &values {
            db.set_setting("k", v).unwrap();
            assert_eq!(db.get_setting("k").unwrap().as_ref(), Some(v));
        }
        assert_eq!(db.all_settings().unwrap().len(), 1, "upsert, not insert");
    }

    #[test]
    fn all_settings_returns_map() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("ui.theme", &json!("light")).unwrap();
        db.set_setting("ui.locale", &json!("tr")).unwrap();
        let all = db.all_settings().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["ui.theme"], json!("light"));
        assert_eq!(all["ui.locale"], json!("tr"));
    }

    #[test]
    fn null_deletes_key() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("k", &json!(1)).unwrap();
        db.set_setting("k", &serde_json::Value::Null).unwrap();
        assert_eq!(db.get_setting("k").unwrap(), None);
        assert!(db.all_settings().unwrap().is_empty());
        // Deleting a missing key is fine.
        db.set_setting("never", &serde_json::Value::Null).unwrap();
    }

    #[test]
    fn invalid_keys_are_rejected() {
        let db = Db::open_in_memory().unwrap();
        for key in ["", "  ", " padded"] {
            assert_eq!(
                db.set_setting(key, &json!(1)).unwrap_err().code,
                ErrorCode::Invalid
            );
            assert_eq!(db.get_setting(key).unwrap_err().code, ErrorCode::Invalid);
        }
    }

    #[test]
    fn corrupt_value_is_db_error_on_get_and_skipped_in_all() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("good", &json!(1)).unwrap();
        db.conn()
            .execute(
                "INSERT INTO settings (key, value_json) VALUES ('bad', '{oops')",
                [],
            )
            .unwrap();
        assert_eq!(db.get_setting("bad").unwrap_err().code, ErrorCode::Db);
        let all = db.all_settings().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all.contains_key("good"));
    }
}
