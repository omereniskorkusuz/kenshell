//! Folder tree repository (P1-T05). Folders nest via `parent_id` (NULL = root).

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use super::sessions::next_session_order;
use super::{Db, now_ms, validate};
use crate::error::{AppError, Result};
use crate::model::Folder;

fn row_to_folder(r: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        name: r.get(2)?,
        sort_order: r.get(3)?,
    })
}

fn folder_not_found(id: i64) -> AppError {
    AppError::not_found(format!("folder {id}"))
}

/// `Ok` when `id` is `None` (root) or an existing folder, `NotFound` otherwise.
pub(super) fn require_folder(conn: &Connection, id: Option<i64>) -> Result<()> {
    let Some(id) = id else { return Ok(()) };
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(folder_not_found(id))
    }
}

/// `sort_order` that appends a folder at the end of `parent`'s children.
fn next_folder_order(conn: &Connection, parent: Option<i64>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM folders WHERE parent_id IS ?1",
        [parent],
        |r| r.get(0),
    )?)
}

fn parent_of(conn: &Connection, id: i64) -> Result<Option<i64>> {
    conn.query_row("SELECT parent_id FROM folders WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()?
    .ok_or_else(|| folder_not_found(id))
}

/// True when `candidate` is `id` itself or one of its descendants.
fn is_self_or_descendant(conn: &Connection, id: i64, candidate: i64) -> Result<bool> {
    // Walk up from `candidate`; UNION (not UNION ALL) terminates even on a corrupt cycle.
    Ok(conn.query_row(
        "WITH RECURSIVE ancestors(id) AS (
             SELECT ?1
             UNION
             SELECT f.parent_id FROM folders f JOIN ancestors a ON f.id = a.id
             WHERE f.parent_id IS NOT NULL
         )
         SELECT EXISTS(SELECT 1 FROM ancestors WHERE id = ?2)",
        params![candidate, id],
        |r| r.get(0),
    )?)
}

impl Db {
    pub fn list_folders(&self) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, parent_id, name, sort_order FROM folders
             ORDER BY parent_id, sort_order, id",
        )?;
        let rows = stmt.query_map([], row_to_folder)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_folder(&self, name: &str, parent_id: Option<i64>) -> Result<Folder> {
        let name = validate::name(name, "folder")?;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_folder(&tx, parent_id)?;
        let sort_order = next_folder_order(&tx, parent_id)?;
        tx.execute(
            "INSERT INTO folders (parent_id, name, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![parent_id, name, sort_order, now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Folder {
            id,
            parent_id,
            name,
            sort_order,
        })
    }

    pub fn rename_folder(&self, id: i64, name: &str) -> Result<()> {
        let name = validate::name(name, "folder")?;
        let changed = self.conn().execute(
            "UPDATE folders SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        if changed == 0 {
            return Err(folder_not_found(id));
        }
        Ok(())
    }

    /// Moves a folder under `parent_id` (None = root). Rejects cycles with `Invalid`.
    /// The folder is appended at the end of its new siblings; moving to the current
    /// parent is a no-op that keeps its position.
    pub fn move_folder(&self, id: i64, parent_id: Option<i64>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_parent = parent_of(&tx, id)?;
        require_folder(&tx, parent_id)?;
        if current_parent == parent_id {
            return Ok(());
        }
        if let Some(target) = parent_id
            && is_self_or_descendant(&tx, id, target)?
        {
            return Err(AppError::invalid(
                "cannot move a folder into itself or its descendant",
            ));
        }
        let sort_order = next_folder_order(&tx, parent_id)?;
        tx.execute(
            "UPDATE folders SET parent_id = ?2, sort_order = ?3 WHERE id = ?1",
            params![id, parent_id, sort_order],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a folder under `parent_id` and places it right before sibling `before_id`
    /// (`None` = at the end). Sibling positions are renumbered 0..n. Rejects cycles.
    pub fn reorder_folder(
        &self,
        id: i64,
        parent_id: Option<i64>,
        before_id: Option<i64>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        parent_of(&tx, id)?;
        require_folder(&tx, parent_id)?;
        if let Some(target) = parent_id
            && is_self_or_descendant(&tx, id, target)?
        {
            return Err(AppError::invalid(
                "cannot move a folder into itself or its descendant",
            ));
        }
        let mut ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM folders WHERE parent_id IS ?1 AND id != ?2
                 ORDER BY sort_order, id",
            )?;
            stmt.query_map(params![parent_id, id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let at = before_id
            .and_then(|b| ids.iter().position(|x| *x == b))
            .unwrap_or(ids.len());
        ids.insert(at, id);
        {
            let mut upd =
                tx.prepare("UPDATE folders SET parent_id = ?2, sort_order = ?3 WHERE id = ?1")?;
            for (i, fid) in ids.iter().enumerate() {
                upd.execute(params![fid, parent_id, i as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a folder; its sessions and sub-folders move to the folder's parent.
    /// They are appended after the parent's existing children, keeping their relative
    /// order, so nothing is ever lost by deleting a folder.
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parent = parent_of(&tx, id)?;

        let min_session: Option<i64> = tx.query_row(
            "SELECT MIN(sort_order) FROM sessions WHERE folder_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        if let Some(min) = min_session {
            let offset = next_session_order(&tx, parent)? - min;
            tx.execute(
                "UPDATE sessions SET folder_id = ?2, sort_order = sort_order + ?3
                 WHERE folder_id = ?1",
                params![id, parent, offset],
            )?;
        }

        let min_child: Option<i64> = tx.query_row(
            "SELECT MIN(sort_order) FROM folders WHERE parent_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        if let Some(min) = min_child {
            let offset = next_folder_order(&tx, parent)? - min;
            tx.execute(
                "UPDATE folders SET parent_id = ?2, sort_order = sort_order + ?3
                 WHERE parent_id = ?1",
                params![id, parent, offset],
            )?;
        }

        tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::ErrorCode;
    use crate::model::{Protocol, SessionInput};
    use crate::storage::Db;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    fn ssh_in(db: &Db, folder_id: Option<i64>, name: &str) -> i64 {
        db.create_session(&SessionInput {
            folder_id,
            name: name.into(),
            protocol: Protocol::Ssh,
            host: Some("10.0.0.1".into()),
            port: None,
            username: None,
            serial: None,
            env_tag: None,
            tags: vec![],
            favorite: false,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
            identify: true,
        })
        .unwrap()
        .id
    }

    /// `(name, parent name)` pairs in list order.
    fn tree(db: &Db) -> Vec<(String, Option<String>)> {
        let folders = db.list_folders().unwrap();
        folders
            .iter()
            .map(|f| {
                let parent = f
                    .parent_id
                    .map(|p| folders.iter().find(|x| x.id == p).unwrap().name.clone());
                (f.name.clone(), parent)
            })
            .collect()
    }

    #[test]
    fn create_appends_and_trims() {
        let db = db();
        let a = db.create_folder("  Core  ", None).unwrap();
        let b = db.create_folder("Access", None).unwrap();
        let c = db.create_folder("Leaf", Some(a.id)).unwrap();
        assert_eq!(a.name, "Core");
        assert_eq!((a.sort_order, b.sort_order), (0, 1));
        assert_eq!(c.sort_order, 0, "sort order is per parent");
        assert_eq!(c.parent_id, Some(a.id));
        assert_eq!(db.list_folders().unwrap(), vec![a, b, c]);
    }

    #[test]
    fn create_validates_name_and_parent() {
        let db = db();
        assert_eq!(
            db.create_folder("   ", None).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(
            db.create_folder("x", Some(42)).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert!(db.list_folders().unwrap().is_empty());
    }

    #[test]
    fn rename() {
        let db = db();
        let f = db.create_folder("Old", None).unwrap();
        db.rename_folder(f.id, " New ").unwrap();
        assert_eq!(db.list_folders().unwrap()[0].name, "New");
        assert_eq!(
            db.rename_folder(f.id, "").unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(
            db.rename_folder(999, "x").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn move_appends_under_new_parent() {
        let db = db();
        let a = db.create_folder("A", None).unwrap();
        let b = db.create_folder("B", None).unwrap();
        db.create_folder("B1", Some(b.id)).unwrap();
        db.move_folder(a.id, Some(b.id)).unwrap();
        let moved = db
            .list_folders()
            .unwrap()
            .into_iter()
            .find(|f| f.id == a.id)
            .unwrap();
        assert_eq!(moved.parent_id, Some(b.id));
        assert_eq!(moved.sort_order, 1);
        // Back to root.
        db.move_folder(a.id, None).unwrap();
        assert_eq!(
            tree(&db),
            vec![
                ("B".into(), None),
                ("A".into(), None),
                ("B1".into(), Some("B".into())),
            ]
        );
    }

    #[test]
    fn move_to_same_parent_keeps_position() {
        let db = db();
        let a = db.create_folder("A", None).unwrap();
        db.create_folder("B", None).unwrap();
        db.move_folder(a.id, None).unwrap();
        assert_eq!(db.list_folders().unwrap()[0].id, a.id);
    }

    #[test]
    fn move_rejects_cycles() {
        let db = db();
        let a = db.create_folder("A", None).unwrap();
        let b = db.create_folder("B", Some(a.id)).unwrap();
        let c = db.create_folder("C", Some(b.id)).unwrap();
        for target in [a.id, b.id, c.id] {
            let err = db.move_folder(a.id, Some(target)).unwrap_err();
            assert_eq!(err.code, ErrorCode::Invalid, "target {target}");
        }
        assert_eq!(
            db.move_folder(b.id, Some(c.id)).unwrap_err().code,
            ErrorCode::Invalid
        );
        // Moving a descendant up / sideways is fine.
        db.move_folder(c.id, Some(a.id)).unwrap();
        db.move_folder(b.id, Some(c.id)).unwrap();
        assert_eq!(
            tree(&db),
            vec![
                ("A".into(), None),
                ("C".into(), Some("A".into())),
                ("B".into(), Some("C".into())),
            ]
        );
    }

    #[test]
    fn move_missing_folder_or_parent_is_not_found() {
        let db = db();
        let a = db.create_folder("A", None).unwrap();
        assert_eq!(
            db.move_folder(999, None).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            db.move_folder(a.id, Some(999)).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn delete_reparents_sessions_and_subfolders() {
        let db = db();
        let root = db.create_folder("Root", None).unwrap();
        let mid = db.create_folder("Mid", Some(root.id)).unwrap();
        let existing_sub = db.create_folder("Existing", Some(root.id)).unwrap();
        let leaf1 = db.create_folder("Leaf1", Some(mid.id)).unwrap();
        let leaf2 = db.create_folder("Leaf2", Some(mid.id)).unwrap();
        let in_root = ssh_in(&db, Some(root.id), "r1");
        let m1 = ssh_in(&db, Some(mid.id), "m1");
        let m2 = ssh_in(&db, Some(mid.id), "m2");
        let deep = ssh_in(&db, Some(leaf1.id), "deep");

        db.delete_folder(mid.id).unwrap();

        let folders = db.list_folders().unwrap();
        assert!(folders.iter().all(|f| f.id != mid.id));
        let children: Vec<i64> = folders
            .iter()
            .filter(|f| f.parent_id == Some(root.id))
            .map(|f| f.id)
            .collect();
        assert_eq!(children, vec![existing_sub.id, leaf1.id, leaf2.id]);

        let in_folder: Vec<i64> = db
            .list_sessions()
            .unwrap()
            .into_iter()
            .filter(|s| s.folder_id == Some(root.id))
            .map(|s| s.id)
            .collect();
        assert_eq!(in_folder, vec![in_root, m1, m2], "appended in order");
        assert_eq!(
            db.get_session(deep).unwrap().folder_id,
            Some(leaf1.id),
            "grandchildren stay with their folder"
        );
        assert_eq!(db.list_sessions().unwrap().len(), 4, "no data loss");
    }

    #[test]
    fn delete_top_level_moves_contents_to_root() {
        let db = db();
        let top = db.create_folder("Top", None).unwrap();
        let sub = db.create_folder("Sub", Some(top.id)).unwrap();
        let root_s = ssh_in(&db, None, "root-session");
        let s = ssh_in(&db, Some(top.id), "s");
        db.delete_folder(top.id).unwrap();
        assert_eq!(db.list_folders().unwrap()[0].parent_id, None);
        assert_eq!(db.list_folders().unwrap()[0].id, sub.id);
        let roots: Vec<i64> = db
            .list_sessions()
            .unwrap()
            .into_iter()
            .filter(|x| x.folder_id.is_none())
            .map(|x| x.id)
            .collect();
        assert_eq!(roots, vec![root_s, s]);
    }

    #[test]
    fn delete_missing_folder_is_not_found() {
        assert_eq!(db().delete_folder(7).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn folders_reorder_among_siblings_and_never_into_themselves() {
        let db = db();
        let a = db.create_folder("A", None).unwrap().id;
        let b = db.create_folder("B", None).unwrap().id;
        let c = db.create_folder("C", None).unwrap().id;
        let sub = db.create_folder("Sub", Some(a)).unwrap().id;
        let names = |db: &Db, parent: Option<i64>| -> Vec<String> {
            db.list_folders()
                .unwrap()
                .into_iter()
                .filter(|f| f.parent_id == parent)
                .map(|f| f.name)
                .collect()
        };
        db.reorder_folder(c, None, Some(a)).unwrap();
        assert_eq!(names(&db, None), ["C", "A", "B"]);
        db.reorder_folder(c, None, None).unwrap();
        assert_eq!(names(&db, None), ["A", "B", "C"]);
        // Into another parent, before an existing child.
        db.reorder_folder(b, Some(a), Some(sub)).unwrap();
        assert_eq!(names(&db, Some(a)), ["B", "Sub"]);
        assert_eq!(names(&db, None), ["A", "C"]);
        assert_eq!(
            db.reorder_folder(a, Some(sub), None).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(
            db.reorder_folder(999, None, None).unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}
