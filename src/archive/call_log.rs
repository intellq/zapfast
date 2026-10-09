//! The call log: the call rows kept in the chats, read across every chat,
//! and the ones deleted here so a later sync from the phone does not bring
//! them back.

use super::{Archive, Result, params};
use crate::model::Content;
use rusqlite::OptionalExtension;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS removed_calls (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS messages_calls ON messages (timestamp)
    WHERE json_extract(content, '$.kind') = 'call';
";

/// One call row of the log, newest first in [`Archive::call_log`].
#[derive(Clone, Debug, PartialEq)]
pub struct LoggedCall {
    pub chat: String,
    pub id: String,
    pub timestamp: i64,
    pub content: Content,
}

impl Archive {
    /// The newest `limit` call rows across every chat.
    pub fn call_log(&self, limit: usize) -> Result<Vec<LoggedCall>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id, timestamp, content FROM messages
             WHERE json_extract(content, '$.kind') = 'call'
             ORDER BY timestamp DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            let content: String = row.get(3)?;
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, content))
        })?;
        let mut calls = Vec::new();
        for row in rows {
            let (chat, id, timestamp, content) = row?;
            if let Ok(content @ Content::Call { .. }) = serde_json::from_str(&content) {
                calls.push(LoggedCall {
                    chat,
                    id,
                    timestamp,
                    content,
                });
            }
        }
        Ok(calls)
    }

    /// A call row in `chat` in the same direction within `window` seconds of
    /// `timestamp`, other than `id`: the same call logged from another source.
    pub fn call_near(
        &self,
        chat: &str,
        id: &str,
        timestamp: i64,
        outgoing: bool,
        window: i64,
    ) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT id FROM messages
                 WHERE chat = ?1 AND id != ?2
                   AND json_extract(content, '$.kind') = 'call'
                   AND json_extract(content, '$.outgoing') = ?3
                   AND timestamp BETWEEN ?4 - ?5 AND ?4 + ?5
                 ORDER BY abs(timestamp - ?4) LIMIT 1",
                params![chat, id, outgoing, timestamp, window],
                |row| row.get(0),
            )
            .optional()
    }

    /// Deletes a call row here only, and remembers it so the phone's log
    /// does not file it again. Returns whether a row was deleted.
    pub fn remove_call(&self, chat: &str, id: &str) -> Result<bool> {
        self.connection.execute(
            "INSERT OR IGNORE INTO removed_calls (chat, id) VALUES (?1, ?2)",
            params![chat, id],
        )?;
        self.delete_message(chat, id)
    }

    /// Whether the call row `id` of `chat` was deleted here.
    pub fn call_removed(&self, chat: &str, id: &str) -> Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM removed_calls WHERE chat = ?1 AND id = ?2)",
            params![chat, id],
            |row| row.get(0),
        )
    }
}
