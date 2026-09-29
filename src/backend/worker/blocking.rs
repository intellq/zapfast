//! The account's blocklist. The server keeps it; this worker asks for it when
//! the link comes up and again now and then, since the protocol library tells
//! nothing when the phone changes it.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use whatsapp_rust::BlockingError;
use whatsapp_rust::lid_pn_cache::LearningSource;
use whatsapp_rust::wacore::iq::blocklist::GetBlocklistSpec;
use whatsapp_rust::wacore::iq::spec::IqSpec;
use whatsapp_rust::wacore::request::InfoQuery;
use whatsapp_rust::wacore_binary::NodeRef;

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

/// One blocked contact as the server lists it: its id, a privacy id on
/// current accounts, and the number the server may send beside it.
#[derive(Debug, PartialEq)]
pub(super) struct Blocked {
    pub(super) jid: Jid,
    pub(super) pn: Option<Jid>,
}

/// The blocklist with the numbers the library's own query drops, and the
/// attribute names its items carried, for the log.
#[derive(Debug, Default)]
pub(super) struct Blocklist {
    pub(super) entries: Vec<Blocked>,
    pub(super) attributes: BTreeSet<String>,
}

/// The library's blocklist query, read in full.
struct FetchBlocklist;

impl IqSpec for FetchBlocklist {
    type Response = Blocklist;

    fn build_iq(&self) -> InfoQuery<'static> {
        GetBlocklistSpec.build_iq()
    }

    fn parse_response(&self, response: &NodeRef<'_>) -> anyhow::Result<Blocklist> {
        Ok(match response.get_optional_child("list") {
            Some(list) => parse_blocklist(list.get_children_by_tag("item")),
            None => parse_blocklist(response.get_children_by_tag("item")),
        })
    }
}

pub(super) fn parse_blocklist<'a, 'b: 'a>(
    items: impl Iterator<Item = &'a NodeRef<'b>>,
) -> Blocklist {
    let mut list = Blocklist::default();
    for item in items {
        list.attributes
            .extend(item.attrs_iter().map(|(name, _)| name.to_string()));
        let Some(jid) = item.get_attr("jid").and_then(|value| value.to_jid()) else {
            continue;
        };
        let pn = item
            .get_attr("pn_jid")
            .and_then(|value| value.to_jid())
            .map(|pn| pn.to_non_ad())
            .filter(Jid::is_pn);
        list.entries.push(Blocked {
            jid: jid.to_non_ad(),
            pn,
        });
    }
    list
}

/// Fetches the blocklist and learns the number behind each privacy id: from
/// the list itself, from what the library already knows, and last from the
/// server's contact lookup. Only counts reach the log.
async fn fetch_blocklist_numbers(
    client: &Client,
) -> Result<(Vec<String>, Vec<(String, String)>), &'static str> {
    let list = client
        .execute(FetchBlocklist)
        .await
        .map_err(|_| "server refused or timed out")?;
    let mut numbers: BTreeMap<String, String> = BTreeMap::new();
    let mut from_list = 0;
    let mut from_library = 0;
    let mut unresolved = Vec::new();
    for entry in &list.entries {
        if !entry.jid.is_lid() {
            continue;
        }
        let lid = entry.jid.user.to_string();
        if let Some(pn) = &entry.pn {
            from_list += 1;
            numbers.insert(lid.clone(), pn.user.to_string());
            let _ = client
                .add_lid_pn_mapping(&lid, &pn.user, LearningSource::BlocklistActive)
                .await;
        } else if let Ok(Some(pair)) = client.get_lid_pn_entry(&entry.jid).await {
            from_library += 1;
            numbers.insert(pair.lid.to_string(), pair.phone_number.to_string());
        } else {
            unresolved.push(entry.jid.clone());
        }
    }
    let mut from_lookup = 0;
    if !unresolved.is_empty()
        && let Ok(found) = client.contacts().is_on_whatsapp(&unresolved).await
    {
        for result in found {
            let lid = [Some(&result.jid), result.lid.as_ref()]
                .into_iter()
                .flatten()
                .find(|jid| jid.is_lid());
            let pn = [Some(&result.jid), result.pn_jid.as_ref()]
                .into_iter()
                .flatten()
                .find(|jid| jid.is_pn());
            if let (Some(lid), Some(pn)) = (lid, pn)
                && numbers
                    .insert(lid.user.to_string(), pn.user.to_string())
                    .is_none()
            {
                from_lookup += 1;
                let _ = client
                    .add_lid_pn_mapping(&lid.user, &pn.user, LearningSource::Usync)
                    .await;
            }
        }
    }
    let lids = list
        .entries
        .iter()
        .filter(|entry| entry.jid.is_lid())
        .count();
    log::info!(
        "blocklist: {} entries, {} privacy ids; numbers from the list {}, known {}, looked up {}, missing {}; item attributes {:?}",
        list.entries.len(),
        lids,
        from_list,
        from_library,
        from_lookup,
        lids - from_list - from_library - from_lookup,
        list.attributes
    );
    let ids = list
        .entries
        .iter()
        .map(|entry| entry.jid.to_string())
        .collect();
    Ok((ids, numbers.into_iter().collect()))
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
            let result = fetch_blocklist_numbers(&client)
                .await
                .map(|(ids, numbers)| {
                    // Before the list, so its privacy ids already name numbers.
                    if !numbers.is_empty() {
                        let _ = commands.send(Command::LidsFound(numbers));
                    }
                    ids
                });
            if let Err(kind) = result {
                log::warn!("blocklist not fetched: {kind}");
            }
            let _ = commands.send(Command::BlocklistFetched(result));
        });
    }

    pub(super) fn blocklist_fetched(&mut self, result: Result<Vec<String>, &'static str>) {
        self.blocklist_fetching = false;
        // A failed fetch was logged where it failed; the last list stands.
        if let Ok(ids) = result {
            let before = self.blocked_ids();
            self.blocklist = Some(ids);
            self.blocklist_changed(&before);
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

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::wacore_binary::builder::NodeBuilder;

    #[test]
    fn the_blocklist_keeps_the_number_the_server_sends_beside_a_privacy_id() {
        let lid: Jid = "100000012345678@lid".parse().unwrap();
        let bare: Jid = "100000099999999@lid".parse().unwrap();
        let pn: Jid = "12025550100@s.whatsapp.net".parse().unwrap();
        let node = NodeBuilder::new("iq")
            .children([NodeBuilder::new("list")
                .children([
                    NodeBuilder::new("item")
                        .attr("jid", lid.to_string())
                        .attr("pn_jid", pn.to_string())
                        .attr("t", "1704067200")
                        .build(),
                    NodeBuilder::new("item")
                        .attr("jid", bare.to_string())
                        .build(),
                ])
                .build()])
            .build();
        let list = FetchBlocklist.parse_response(&node.as_node_ref()).unwrap();
        assert_eq!(
            list.entries,
            [
                Blocked {
                    jid: lid,
                    pn: Some(pn)
                },
                Blocked {
                    jid: bare,
                    pn: None
                },
            ]
        );
        assert_eq!(
            list.attributes.into_iter().collect::<Vec<_>>(),
            ["jid", "pn_jid", "t"]
        );
    }
}
