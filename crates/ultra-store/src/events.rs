//! Append-only job events. There is no update API, and a trigger refuses
//! updates at the database level; only retention deletes events, with their job.

use rusqlite::{Connection, OptionalExtension, params};
use ultra_core::{CallerId, Event, JobId, Result, ReviewError};

use crate::Store;
use crate::db::{from_json, is_constraint, ms, not_found_or_not_owned, store_err, to_json};

pub(crate) fn insert_event(conn: &Connection, event: &Event) -> Result<()> {
    conn.execute(
        "INSERT INTO events (event_id, job_id, at, body) VALUES (?1, ?2, ?3, ?4)",
        params![
            event.id.to_string(),
            event.job_id.to_string(),
            ms(event.at),
            to_json(event)?
        ],
    )
    .map_err(|e| {
        if is_constraint(&e) {
            ReviewError::invalid("event id is already recorded")
        } else {
            store_err(e)
        }
    })?;
    Ok(())
}

impl Store {
    /// Append `event` to `job`'s log. The event must name the same job.
    pub fn append_event(&self, job: JobId, event: Event) -> Result<()> {
        if event.job_id != job {
            return Err(ReviewError::invalid(format!(
                "event names job {} but was filed under {job}",
                event.job_id
            )));
        }
        let conn = self.conn()?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM jobs WHERE job_id = ?1",
                params![job.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(store_err)?
            .is_some();
        if !exists {
            return Err(ReviewError::invalid(format!("unknown job {job}")));
        }
        insert_event(&conn, &event)
    }

    /// The job's events in append order, for its owner only.
    pub fn list_events(&self, caller: &CallerId, job: JobId) -> Result<Vec<Event>> {
        let conn = self.conn()?;
        let owned = conn
            .query_row(
                "SELECT 1 FROM jobs WHERE job_id = ?1 AND owner = ?2",
                params![job.to_string(), caller.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(store_err)?
            .is_some();
        if !owned {
            return Err(not_found_or_not_owned("job"));
        }
        let mut stmt = conn
            .prepare("SELECT body FROM events WHERE job_id = ?1 ORDER BY seq")
            .map_err(store_err)?;
        let bodies: Vec<String> = stmt
            .query_map(params![job.to_string()], |r| r.get(0))
            .map_err(store_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(store_err)?;
        bodies.iter().map(|b| from_json(b)).collect()
    }
}
