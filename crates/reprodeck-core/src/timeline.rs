
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub meta: Option<String>,
    pub state: String,
    pub created_at: i64,
}

pub fn create_action(conn: &Connection, a: &Action) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO actions (id, session_id, parent_id, kind, meta, state, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![a.id, a.session_id, a.parent_id, a.kind, a.meta, a.state, a.created_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::db::init_db;

    #[test]
    fn create_and_read_action() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        let conn = init_db(path).expect("init db");

        let a = Action {
            id: "act-1".to_string(),
            session_id: "s-1".to_string(),
            parent_id: None,
            kind: "test".to_string(),
            meta: Some("{}".to_string()),
            state: "Created".to_string(),
            created_at: 1,
        };

        // session must exist per FK; create minimal session
        conn.execute("INSERT INTO sessions(id, repo_id, started_at) VALUES (?1,?2,?3)", rusqlite::params!["s-1","repo-x",1]).unwrap();

        create_action(&conn, &a).expect("insert");

        let mut stmt = conn.prepare("SELECT id, session_id, kind FROM actions WHERE id = ?1").unwrap();
        let mut rows = stmt.query(rusqlite::params!["act-1"]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let id: String = row.get(0).unwrap();
        assert_eq!(id, "act-1");
    }
}
