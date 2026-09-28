//! Transcripts of voice messages, made on this computer.
//!
//! They hold what was said in a message, so they live in the encrypted
//! archive beside it and last only while it is an audio message there:
//! deleting it, even for everyone, clearing its chat or unlinking drops them.

use super::{Archive, Result, params};
use crate::transcribe::Transcript;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS transcripts (
    chat TEXT NOT NULL,
    message TEXT NOT NULL,
    text TEXT NOT NULL,
    language TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (chat, message)
);
";

/// Drops transcripts whose audio is gone, as when a range of a chat was
/// removed on the phone or the message was deleted for everyone.
pub(super) fn prune(connection: &rusqlite::Connection) -> Result<()> {
    connection.execute(
        &format!("DELETE FROM transcripts WHERE NOT EXISTS ({AUDIO})"),
        [],
    )?;
    Ok(())
}

/// The transcript's message, while it is an audio message.
const AUDIO: &str = "SELECT 1 FROM messages m
     WHERE m.chat = transcripts.chat AND m.id = transcripts.message
         AND json_extract(m.content, '$.kind') = 'audio'";

impl Archive {
    /// Every stored transcript of an audio message, by chat and message id.
    pub fn transcripts(&self) -> Result<Vec<(String, String, Transcript)>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT chat, message, text, language, model, created_at FROM transcripts
             WHERE EXISTS ({AUDIO})"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                Transcript {
                    text: row.get(2)?,
                    language: row.get(3)?,
                    model: row.get(4)?,
                    at: row.get(5)?,
                },
            ))
        })?;
        rows.collect()
    }

    /// Stores a message's transcript, replacing an earlier one.
    pub fn set_transcript(&self, chat: &str, message: &str, transcript: &Transcript) -> Result<()> {
        self.connection.execute(
            "INSERT INTO transcripts (chat, message, text, language, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chat, message) DO UPDATE SET
                text = excluded.text,
                language = excluded.language,
                model = excluded.model,
                created_at = excluded.created_at",
            params![
                chat,
                message,
                transcript.text,
                transcript.language,
                transcript.model,
                transcript.at
            ],
        )?;
        Ok(())
    }

    /// Drops a message's transcript.
    pub fn clear_transcript(&self, chat: &str, message: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM transcripts WHERE chat = ?1 AND message = ?2",
            params![chat, message],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(text: &str) -> Transcript {
        Transcript {
            text: text.to_owned(),
            language: "pt".to_owned(),
            model: "ggml-small.bin".to_owned(),
            at: 10,
        }
    }

    #[test]
    fn transcripts_survive_a_restart_and_go_with_their_message() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [17; 32];
        let chat = "1@s.whatsapp.net";
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat(chat, "A").unwrap();
            archive
                .set_transcript(chat, "gone", &transcript("x"))
                .unwrap();
            archive
                .set_transcript(chat, "m1", &transcript("first"))
                .unwrap();
            archive
                .set_transcript(chat, "m1", &transcript("again"))
                .unwrap();
            // Kept only while its message is an audio in the archive.
            for (id, kind) in [("m1", "audio"), ("m2", "audio"), ("revoked", "revoked")] {
                archive
                    .connection
                    .execute(
                        "INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                         VALUES (?1, ?2, ?1, 0, 1, json_object('kind', ?3))",
                        params![chat, id, kind],
                    )
                    .unwrap();
            }
            archive
                .set_transcript(chat, "revoked", &transcript("y"))
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert_eq!(
            archive.transcripts().unwrap(),
            vec![(chat.to_owned(), "m1".to_owned(), transcript("again"))]
        );
        archive.delete_message(chat, "m1").unwrap();
        assert!(archive.transcripts().unwrap().is_empty());

        archive
            .set_transcript(chat, "m2", &transcript("b"))
            .unwrap();
        assert_eq!(archive.transcripts().unwrap().len(), 1);
        archive.clear_transcript(chat, "m2").unwrap();
        assert!(archive.transcripts().unwrap().is_empty());

        archive
            .set_transcript(chat, "m2", &transcript("b"))
            .unwrap();
        archive.clear_chat(chat).unwrap();
        assert!(archive.transcripts().unwrap().is_empty());
    }
}
