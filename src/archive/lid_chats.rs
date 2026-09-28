//! A direct chat that began under a contact's privacy id (`<lid>@lid`), before ZapFast knew their
//! number, joins the chat under that number once it is known. The phone shows one chat, named by
//! the number; without this, ZapFast showed an "Unknown" chat beside it with the older messages.
//!
//! The privacy id's own row stays, emptied, because early preference sync keeps writing to it
//! (see [`Archive::put_lid`]); the chat list leaves out such a row once it has no messages.

use super::{Archive, Result, params};

/// Tables whose rows belong to a chat, keyed by its id in `chat`.
const CHAT_ROWS: [&str; 8] = [
    "messages",
    "group_receipts",
    "polls",
    "poll_history",
    "poll_votes",
    "local_chat_labels",
    "drafts",
    "transcripts",
];

impl Archive {
    /// Moves what is filed under the privacy id's chat to the phone number's chat. Returns whether
    /// there was anything to move. A row both chats have, as the same message archived twice, keeps
    /// the phone number's copy.
    pub fn merge_lid_chat(&self, lid: &str, pn: &str) -> Result<bool> {
        let from = format!("{lid}@lid");
        let to = format!("{pn}@s.whatsapp.net");
        let messages: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM messages WHERE chat = ?1",
            params![from],
            |row| row.get(0),
        )?;
        if messages == 0 {
            return Ok(false);
        }
        let transaction = self.connection.unchecked_transaction()?;
        // The chat under the number takes the newer activity and the unread count, or is made from
        // the privacy id's row when there is none yet.
        transaction.execute(
            "UPDATE chats SET
                 last_activity = MAX(last_activity,
                     (SELECT last_activity FROM chats WHERE id = ?1)),
                 unread = unread + (SELECT unread FROM chats WHERE id = ?1),
                 name = CASE WHEN name = '' THEN (SELECT name FROM chats WHERE id = ?1)
                     ELSE name END
             WHERE id = ?2 AND EXISTS (SELECT 1 FROM chats WHERE id = ?1)",
            params![from, to],
        )?;
        let columns = {
            let mut statement =
                transaction.prepare("SELECT name FROM pragma_table_info('chats')")?;
            let names = statement.query_map([], |row| row.get::<_, String>(0))?;
            names
                .filter(|name| name.as_ref().map_or(true, |name| name != "id"))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        };
        transaction.execute(
            &format!(
                "INSERT OR IGNORE INTO chats (id, {columns})
                 SELECT ?2, {columns} FROM chats WHERE id = ?1"
            ),
            params![from, to],
        )?;
        transaction.execute(
            "UPDATE chats SET unread = 0, marked_unread = 0 WHERE id = ?1",
            params![from],
        )?;
        for table in CHAT_ROWS {
            transaction.execute(
                &format!("UPDATE OR IGNORE {table} SET chat = ?2 WHERE chat = ?1"),
                params![from, to],
            )?;
            transaction.execute(
                &format!("DELETE FROM {table} WHERE chat = ?1"),
                params![from],
            )?;
        }
        // What they sent was signed with the privacy id.
        transaction.execute(
            "UPDATE messages SET sender = ?2 WHERE chat = ?2 AND sender = ?1",
            params![from, to],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Merges every privacy-id chat whose number is already known, as those an archive kept from
    /// before chats were merged. Returns whether any was.
    pub fn merge_mapped_lid_chats(&self) -> Result<bool> {
        let pairs = {
            let mut statement = self.connection.prepare(
                "SELECT lid, pn FROM lids
                 WHERE EXISTS (SELECT 1 FROM messages WHERE chat = lids.lid || '@lid')",
            )?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<Result<Vec<(String, String)>>>()?
        };
        let mut merged = false;
        for (lid, pn) in pairs {
            merged |= self.merge_lid_chat(&lid, &pn)?;
        }
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(archive: &Archive, chat: &str, id: &str, sender: &str, timestamp: i64) {
        archive
            .connection
            .execute(
                "INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 VALUES (?1, ?2, ?3, 0, ?4, json_object('kind', 'text', 'text', ?2))",
                params![chat, id, sender, timestamp],
            )
            .unwrap();
    }

    fn ids(archive: &Archive, chat: &str) -> Vec<String> {
        let mut statement = archive
            .connection
            .prepare("SELECT id FROM messages WHERE chat = ?1 ORDER BY timestamp")
            .unwrap();
        statement
            .query_map(params![chat], |row| row.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_chat_begun_under_a_privacy_id_joins_the_numbers_chat() {
        for existing in [false, true] {
            let archive = Archive::in_memory().unwrap();
            let lid = "2@lid";
            let phone = "1@s.whatsapp.net";
            archive.ensure_chat(lid, "").unwrap();
            message(&archive, lid, "a", lid, 10);
            message(&archive, lid, "same", lid, 20);
            archive
                .connection
                .execute("UPDATE chats SET unread = 2 WHERE id = ?1", params![lid])
                .unwrap();
            if existing {
                archive.ensure_chat(phone, "").unwrap();
                message(&archive, phone, "same", phone, 20);
                message(&archive, phone, "b", phone, 30);
            }
            archive.put_lid("2", "1").unwrap();

            assert!(ids(&archive, lid).is_empty());
            let expected: &[&str] = if existing {
                &["a", "same", "b"]
            } else {
                &["a", "same"]
            };
            assert_eq!(ids(&archive, phone), expected);
            let senders: i64 = archive
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE sender = ?1",
                    params![lid],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(senders, 0, "the messages name the number");
            let chats = archive.chats().unwrap();
            let merged = chats.iter().find(|chat| chat.id == phone).unwrap();
            assert_eq!(merged.unread, 2);
            assert!(
                chats
                    .iter()
                    .all(|chat| chat.id != lid || chat.last.is_none())
            );
            assert_eq!(archive.chat(lid).unwrap().unwrap().unread, 0);
        }
    }

    #[test]
    fn chats_merge_on_opening_an_archive_that_knew_the_number_already() {
        let archive = Archive::in_memory().unwrap();
        let lid = "2@lid";
        archive.put_lid("2", "1").unwrap();
        archive.ensure_chat(lid, "").unwrap();
        message(&archive, lid, "a", lid, 10);
        assert!(archive.merge_mapped_lid_chats().unwrap());
        assert_eq!(ids(&archive, "1@s.whatsapp.net"), ["a"]);
        assert!(!archive.merge_mapped_lid_chats().unwrap(), "once");
    }
}
