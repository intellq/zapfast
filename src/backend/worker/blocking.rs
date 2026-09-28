//! The account's blocklist. The server keeps it; this worker asks for it when
//! the link comes up and again now and then, since the protocol library tells
//! nothing when the phone changes it.

use super::*;
use std::collections::BTreeSet;
use whatsapp_rust::BlockingError;

/// How long a fetched blocklist stands before opening a chat asks again.
pub(super) const BLOCKLIST_REFRESH: Duration = Duration::from_secs(60);

/// Blocks or unblocks one contact. A block carries both the privacy id and
/// the number, so a number the library knows no privacy id for is looked up
/// once: the lookup teaches the library the pair.
async fn change_block(client: &Client, jid: &Jid, blocked: bool) -> Result<(), BlockingError> {
    let apply = || async {
        if blocked {
            client.blocking().block(jid).await
        } else {
            client.blocking().unblock(jid).await
        }
    };
    match apply().await {
        Err(BlockingError::InvalidJid(_)) if jid.is_pn() => {
            log::info!("blocklist: no privacy id known yet; looking the number up");
            let _ = client
                .contacts()
                .is_on_whatsapp(std::slice::from_ref(jid))
                .await;
            apply().await
        }
        result => result,
    }
}

/// What went wrong, without anything that names the contact.
fn failure_kind(error: &BlockingError) -> &'static str {
    match error {
        BlockingError::InvalidJid(_) => "no privacy id",
        BlockingError::Iq(_) => "server refused or timed out",
        _ => "internal",
    }
}

impl Worker {
    /// The chats the account has blocked, by the id the archive uses.
    pub(super) fn blocked_ids(&self) -> BTreeSet<ChatId> {
        self.blocklist
            .iter()
            .flatten()
            .map(|raw| self.canonical_str(raw))
            .collect()
    }

    pub(super) fn is_blocked(&self, chat: &str) -> bool {
        let id = self.canonical_str(chat);
        self.blocklist
            .iter()
            .flatten()
            .any(|raw| self.canonical_str(raw) == id)
    }

    /// Asks the server for the blocklist. Without `force`, a list fetched
    /// less than [`BLOCKLIST_REFRESH`] ago stands.
    pub(super) fn fetch_blocklist(&mut self, force: bool) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.status != LinkStatus::Connected {
            return;
        }
        if self.blocklist_fetching {
            self.blocklist_again |= force;
            return;
        }
        if !force
            && self
                .blocklist_asked
                .is_some_and(|asked| asked.elapsed() < BLOCKLIST_REFRESH)
        {
            return;
        }
        self.blocklist_fetching = true;
        self.blocklist_asked = Some(Instant::now());
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = match client.blocking().get_blocklist().await {
                Ok(entries) => {
                    let mut found = Vec::new();
                    let mut ids = Vec::new();
                    for entry in entries {
                        let jid = entry.jid.to_non_ad();
                        if jid.is_lid()
                            && let Ok(Some(pair)) = client.get_lid_pn_entry(&jid).await
                        {
                            found.push((pair.lid.to_string(), pair.phone_number.to_string()));
                        }
                        ids.push(jid.to_string());
                    }
                    // Before the list, so its privacy ids already name numbers.
                    if !found.is_empty() {
                        let _ = commands.send(Command::LidsFound(found));
                    }
                    Ok(ids)
                }
                Err(error) => Err(failure_kind(&error)),
            };
            let _ = commands.send(Command::BlocklistFetched(result));
        });
    }

    pub(super) fn blocklist_fetched(&mut self, result: Result<Vec<String>, &'static str>) {
        self.blocklist_fetching = false;
        match result {
            Ok(ids) => {
                log::info!("blocklist: {} contacts", ids.len());
                let before = self.blocked_ids();
                self.blocklist = Some(ids);
                self.blocklist_changed(&before);
            }
            Err(kind) => log::warn!("blocklist not fetched: {kind}"),
        }
        if std::mem::take(&mut self.blocklist_again) {
            self.fetch_blocklist(true);
        }
    }

    /// Tells the interface the whole list, and redraws the chats whose state
    /// changed since `before`.
    pub(super) fn blocklist_changed(&self, before: &BTreeSet<ChatId>) {
        if self.blocklist.is_none() {
            return;
        }
        let after = self.blocked_ids();
        self.emit(Event::Blocklist(after.iter().cloned().collect()));
        for chat in before.symmetric_difference(&after) {
            self.emit_chat(chat);
        }
    }

    pub(super) fn set_blocked(&mut self, chat: ChatId, blocked: bool) {
        let jid = Self::jid_of(&chat).filter(|jid| jid.is_pn() || jid.is_lid());
        let (Some(client), Some(jid)) = (
            self.client
                .clone()
                .filter(|_| self.status == LinkStatus::Connected),
            jid,
        ) else {
            self.emit(Event::BlockDone {
                chat,
                blocked,
                ok: false,
            });
            return;
        };
        if self.is_me(&chat) {
            self.emit(Event::BlockDone {
                chat,
                blocked,
                ok: false,
            });
            return;
        }
        log::info!(
            "blocklist: {} a contact; chat={}",
            if blocked { "blocking" } else { "unblocking" },
            jid_kind(&jid)
        );
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = change_block(&client, &jid, blocked).await;
            if let Err(error) = &result {
                log::warn!("blocklist change refused: {}", failure_kind(error));
            }
            let _ = commands.send(Command::BlockAnswered {
                chat,
                blocked,
                ok: result.is_ok(),
            });
        });
    }

    pub(super) fn block_answered(&mut self, chat: ChatId, blocked: bool, ok: bool) {
        if ok {
            log::info!("blocklist: the server took the change");
            let before = self.blocked_ids();
            let id = self.canonical_str(&chat);
            let mut list: Vec<String> = self
                .blocklist
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|raw| self.canonical_str(raw) != id)
                .collect();
            if blocked {
                list.push(id);
            }
            self.blocklist = Some(list);
            self.blocklist_changed(&before);
        }
        self.emit(Event::BlockDone { chat, blocked, ok });
        // What the server holds now, whichever way it answered.
        self.fetch_blocklist(true);
    }
}
