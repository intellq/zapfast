//! The call rows in the chats' history: the calls this computer saw, and the
//! ones the phone's call log brings.
//!
//! The phone reports its calls three ways: app state (`call_log`, one record
//! per call, also for calls placed or answered on the phone while ZapFast was
//! closed), the `callLogRecords` of the history sync at link time, and now and
//! then a `callLogMessage` in the chat. Each becomes the same local row, never
//! sent to WhatsApp. A call ZapFast logged itself and the phone's record of it
//! share the WhatsApp call id; a record without one is matched by chat,
//! direction and time.

use super::calls::CALL_ROW_PREFIX;
use super::*;
use crate::model::CallStatus;

/// How many call rows the call log screen holds.
const CALL_LOG_LIMIT: usize = 500;

/// Seconds apart two rows of the same chat and direction can be and still be
/// the same call seen from two sources.
const SAME_CALL_WINDOW: i64 = 90;

/// A missed call older than this, in seconds, is filed without notifying:
/// the phone's log can arrive long after the call.
const RECENT_CALL: i64 = 10 * 60;

/// A call to file in a chat's history.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CallEntry {
    /// The row id: [`CALL_ROW_PREFIX`] and the WhatsApp call id, or the id of
    /// the `callLogMessage` that carried it.
    pub id: String,
    pub chat: ChatId,
    /// When the call began ringing, in Unix seconds.
    pub began: i64,
    pub outgoing: bool,
    pub status: CallStatus,
    pub seconds: Option<u32>,
    pub video: bool,
    pub group: Option<u32>,
    /// Filed without counting as unread or notifying: from history, or too
    /// old to be news.
    pub quiet: bool,
}

impl CallEntry {
    fn content(&self) -> Content {
        Content::Call {
            outgoing: self.outgoing,
            status: self.status,
            seconds: self.seconds,
            video: self.video,
            group: self.group,
        }
    }
}

impl Worker {
    /// Hands the interface the newest call rows of every chat.
    pub(super) fn emit_call_log(&self) {
        match self.archive.call_log(CALL_LOG_LIMIT) {
            Ok(calls) => self.emit(Event::CallLog(calls)),
            Err(error) => log::warn!("could not read the call log: {error}"),
        }
    }

    /// Files a call in its chat's history, on this computer only, and says
    /// whether a row was added or changed.
    ///
    /// A call already filed, under its id or as the same call from another
    /// source, is completed rather than repeated. Only a missed call that
    /// rang counts as unread and notifies, as on the phone; the others are a
    /// record of something the user already saw.
    pub(super) fn log_call(&mut self, entry: CallEntry) -> bool {
        let chat = entry.chat.as_str();
        if self.archive.call_removed(chat, &entry.id).unwrap_or(false) {
            return false;
        }
        let existing = match self.archive.message(chat, &entry.id).ok().flatten() {
            Some(row) => Some(row),
            None => self
                .archive
                .call_near(
                    chat,
                    &entry.id,
                    entry.began,
                    entry.outgoing,
                    SAME_CALL_WINDOW,
                )
                .ok()
                .flatten()
                .and_then(|id| self.archive.message(chat, &id).ok().flatten()),
        };
        if let Some(mut row) = existing {
            let Some(content) = merged(&row.content, &entry) else {
                return false;
            };
            if let Err(error) = self
                .archive
                .set_content(chat, &row.id, &content, row.edited)
            {
                log::warn!("could not update a logged call: {error}");
                return false;
            }
            row.content = content;
            self.emit(Event::Messages {
                chat: chat.to_owned(),
                messages: vec![row],
                older: false,
                complete: false,
            });
            return true;
        }
        if self.predates_removal(chat, entry.began) {
            return false;
        }
        self.ensure_chat(chat, None);
        let row = Message {
            id: entry.id.clone(),
            chat: chat.to_owned(),
            sender: if entry.outgoing {
                self.me()
            } else {
                chat.to_owned()
            },
            sender_name: None,
            from_me: entry.outgoing,
            timestamp: entry.began,
            history_order: None,
            content: entry.content(),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        if let Err(error) = self.archive.insert_message(&row, None) {
            log::warn!("could not log a call: {error}");
            return false;
        }
        let news = entry.status == CallStatus::Missed && !entry.quiet;
        if news {
            let _ = self.archive.bump_unread(chat);
            if !self.keep_chats_archived
                && let Err(error) = self
                    .archive
                    .unarchive_for_message(chat, entry.began.saturating_mul(1000))
            {
                log::warn!("could not unarchive a chat: {error}");
            }
        }
        self.emit(Event::Messages {
            chat: chat.to_owned(),
            messages: vec![row.clone()],
            older: false,
            complete: false,
        });
        self.emit_chat(chat);
        if news && !self.syncing {
            self.emit(Event::Incoming {
                chat: chat.to_owned(),
                message: Box::new(row),
            });
        }
        true
    }

    /// A call the phone's log synced through app state.
    pub(super) fn call_log_synced(&mut self, sync: &wa_events::CallLogSync) {
        let creator = self.canonical(&sync.call_creator_jid);
        let Some(mut entry) = self.entry_from_record(&sync.record, Some(&creator), sync.from_me)
        else {
            return;
        };
        entry.id = format!("{CALL_ROW_PREFIX}{}", sync.call_id);
        entry.quiet |= sync.from_full_sync;
        if self.log_call(entry) {
            self.emit_call_log();
        }
    }

    /// Files the call records a history sync carried, quietly, and refreshes
    /// the call log once.
    pub(super) fn file_history_calls(&mut self, records: &[wa::CallLogRecord]) {
        let mut changed = false;
        for record in records {
            let creator = record
                .call_creator_jid
                .as_deref()
                .and_then(Self::jid_of)
                .map(|jid| self.canonical(&jid));
            // Without a creator, the phone's own records say the direction
            // literally (see whatsapp-rust's call_log module).
            let from_me = match &creator {
                Some(creator) => self.is_me(creator),
                None => record.is_incoming == Some(false),
            };
            let Some(call_id) = record.call_id.clone() else {
                continue;
            };
            let Some(mut entry) = self.entry_from_record(record, creator.as_deref(), from_me)
            else {
                continue;
            };
            entry.id = format!("{CALL_ROW_PREFIX}{call_id}");
            entry.quiet = true;
            changed |= self.log_call(entry);
        }
        if changed {
            self.emit_call_log();
        }
    }

    /// The row for a record of the phone's call log, without its id; `None`
    /// for a call not to show (a scheduled one still to come, or one whose
    /// other side cannot be told).
    fn entry_from_record(
        &self,
        record: &wa::CallLogRecord,
        creator: Option<&str>,
        from_me: bool,
    ) -> Option<CallEntry> {
        use wa::call_log_record::{CallResult, SilenceReason};
        let result = record.call_result.unwrap_or(CallResult::CONNECTED);
        let mut status = match result {
            CallResult::UPCOMING => return None,
            CallResult::CONNECTED => CallStatus::Answered,
            CallResult::REJECTED => CallStatus::Declined,
            CallResult::ACCEPTEDELSEWHERE => CallStatus::AnsweredElsewhere,
            CallResult::ONGOING => CallStatus::Ongoing,
            CallResult::FAILED | CallResult::INVALID => CallStatus::Failed,
            CallResult::CANCELLED
            | CallResult::MISSED
            | CallResult::UNAVAILABLE
            | CallResult::ABANDONED => {
                if from_me {
                    CallStatus::NoAnswer
                } else {
                    CallStatus::Missed
                }
            }
        };
        if status == CallStatus::Missed {
            if record.is_dnd_mode == Some(true) {
                status = CallStatus::SilencedDnd;
            } else if record.silence_reason == Some(SilenceReason::PRIVACY) {
                status = CallStatus::SilencedUnknown;
            }
        }
        let others: Vec<String> = record
            .participants
            .iter()
            .filter_map(|participant| participant.user_jid.as_deref().and_then(Self::jid_of))
            .map(|jid| self.canonical(&jid))
            .filter(|id| !self.is_me(id))
            .collect();
        let group_chat = record
            .group_jid
            .as_deref()
            .and_then(Self::jid_of)
            .map(|jid| self.canonical(&jid));
        let group = (group_chat.is_some() || others.len() > 1)
            .then(|| u32::try_from(others.len() + 1).unwrap_or(u32::MAX));
        let chat = match (group_chat, creator) {
            (Some(group), _) => group,
            (None, Some(creator)) if !from_me => creator.to_owned(),
            _ => others.first()?.clone(),
        };
        let seconds = record
            .duration
            .filter(|_| status == CallStatus::Answered)
            .map(|duration| u32::try_from(duration.max(0)).unwrap_or(u32::MAX));
        let began = record
            .start_time
            .map(seconds_of)
            .unwrap_or_else(crate::util::now);
        Some(CallEntry {
            id: String::new(),
            chat,
            began,
            outgoing: from_me,
            status,
            seconds,
            video: record.is_video == Some(true),
            group,
            quiet: crate::util::now() - began > RECENT_CALL,
        })
    }

    /// Removes a call row here only, and keeps it from coming back.
    pub(super) fn remove_call(&mut self, chat: &str, id: &str) {
        match self.archive.remove_call(chat, id) {
            Ok(_) => {
                self.emit(Event::MessageDeleted {
                    chat: chat.to_owned(),
                    id: id.to_owned(),
                });
                self.emit_chat(chat);
                self.emit_call_log();
            }
            Err(error) => {
                log::warn!("could not remove a logged call: {error}");
                self.emit(Event::Error(tr("Could not remove the call").to_owned()));
            }
        }
    }
}

/// The row a `callLogMessage` in a chat stands for, as filed by
/// [`Worker::log_call`].
pub(super) fn call_log_message_entry(
    log: &wa::message::CallLogMessage,
    id: &str,
    chat: &str,
    timestamp: i64,
    from_me: bool,
) -> CallEntry {
    let status = call_log_message_status(log, from_me);
    let people = log.participants.len();
    CallEntry {
        id: id.to_owned(),
        chat: chat.to_owned(),
        began: timestamp,
        outgoing: from_me,
        status,
        seconds: log
            .duration_secs
            .filter(|_| status == CallStatus::Answered)
            .map(|seconds| u32::try_from(seconds.max(0)).unwrap_or(u32::MAX)),
        video: log.is_video == Some(true),
        group: (people > 1).then(|| u32::try_from(people + 1).unwrap_or(u32::MAX)),
        quiet: crate::util::now() - timestamp > RECENT_CALL,
    }
}

/// How the call of a `callLogMessage` turned out, from its sender's side.
pub(super) fn call_log_message_status(
    log: &wa::message::CallLogMessage,
    from_me: bool,
) -> CallStatus {
    use wa::message::call_log_message::CallOutcome;
    match log.call_outcome.unwrap_or(CallOutcome::CONNECTED) {
        CallOutcome::CONNECTED => CallStatus::Answered,
        CallOutcome::MISSED if from_me => CallStatus::NoAnswer,
        CallOutcome::MISSED => CallStatus::Missed,
        CallOutcome::FAILED => CallStatus::Failed,
        CallOutcome::REJECTED => CallStatus::Declined,
        CallOutcome::ACCEPTED_ELSEWHERE => CallStatus::AnsweredElsewhere,
        CallOutcome::ONGOING => CallStatus::Ongoing,
        CallOutcome::SILENCED_BY_DND => CallStatus::SilencedDnd,
        CallOutcome::SILENCED_UNKNOWN_CALLER => CallStatus::SilencedUnknown,
    }
}

/// What a logged call becomes once `entry` reports the same call, or `None`
/// when it adds nothing: a call still going takes its outcome, and a missing
/// length, kind or group size is filled in. A call's own outcome, seen here,
/// is kept over the phone's.
fn merged(current: &Content, entry: &CallEntry) -> Option<Content> {
    let Content::Call {
        outgoing,
        status,
        seconds,
        video,
        group,
    } = current
    else {
        return None;
    };
    let status = if *status == CallStatus::Ongoing {
        entry.status
    } else {
        *status
    };
    let content = Content::Call {
        outgoing: *outgoing,
        status,
        seconds: seconds.or(entry.seconds.filter(|_| status == CallStatus::Answered)),
        video: *video || entry.video,
        group: group.or(entry.group),
    };
    (content != *current).then_some(content)
}

/// Unix seconds from a time the phone may give in seconds or milliseconds.
fn seconds_of(value: i64) -> i64 {
    if value > 100_000_000_000 {
        value / 1000
    } else {
        value.max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;
    use wa::call_log_record::{CallResult, ParticipantInfo};

    const ME: &str = "15550001111@s.whatsapp.net";

    fn record(result: CallResult, start: i64) -> wa::CallLogRecord {
        wa::CallLogRecord {
            call_result: Some(result),
            start_time: Some(start),
            ..Default::default()
        }
    }

    fn synced(
        call_id: &str,
        creator: &str,
        from_me: bool,
        record: wa::CallLogRecord,
    ) -> wa_events::CallLogSync {
        wa_events::CallLogSync::builder()
            .call_creator_jid(creator.parse().unwrap())
            .call_id(call_id.to_owned())
            .from_me(from_me)
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .record(Box::new(record))
            .from_full_sync(false)
            .build()
    }

    fn row(worker: &Worker, chat: &str, call_id: &str) -> Option<Message> {
        worker
            .archive
            .message(chat, &format!("{CALL_ROW_PREFIX}{call_id}"))
            .unwrap()
    }

    fn calls_in(worker: &Worker, chat: &str) -> usize {
        worker
            .archive
            .call_log(100)
            .unwrap()
            .into_iter()
            .filter(|call| call.chat == chat)
            .count()
    }

    #[test]
    fn a_call_missed_on_the_phone_is_filed_unread_and_notifies() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let now = crate::util::now();
        let mut missed = record(CallResult::MISSED, now * 1000);
        missed.is_video = Some(true);
        worker.call_log_synced(&synced("phone-1", PEER, false, missed));
        let row = row(&worker, PEER, "phone-1").expect("the call is filed");
        assert_eq!(
            row.content,
            Content::Call {
                outgoing: false,
                status: CallStatus::Missed,
                seconds: None,
                video: true,
                group: None,
            }
        );
        assert_eq!(row.timestamp, now, "milliseconds become seconds");
        assert_eq!(worker.archive.chat(PEER).unwrap().unwrap().unread, 1);
        let events: Vec<_> = events.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Incoming { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::CallLog(calls) if calls.len() == 1))
        );
    }

    #[test]
    fn an_old_call_from_the_phone_is_filed_quietly() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let long_ago = crate::util::now() - 3 * 3600;
        let mut silenced = record(CallResult::MISSED, long_ago);
        silenced.is_dnd_mode = Some(true);
        worker.call_log_synced(&synced("old", PEER, false, silenced));
        let row = row(&worker, PEER, "old").expect("the call is filed");
        assert!(matches!(
            row.content,
            Content::Call {
                status: CallStatus::SilencedDnd,
                ..
            }
        ));
        assert_eq!(worker.archive.chat(PEER).unwrap().unwrap().unread, 0);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Incoming { .. }))
        );
    }

    #[test]
    fn a_call_placed_on_the_phone_is_filed_under_the_person_called() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let mut placed = record(CallResult::CONNECTED, crate::util::now() - 600);
        placed.duration = Some(95);
        placed.participants = vec![ParticipantInfo {
            user_jid: Some(PEER.into()),
            ..Default::default()
        }];
        worker.call_log_synced(&synced("out", ME, true, placed));
        let row = row(&worker, PEER, "out").expect("the call is in the peer's chat");
        assert!(row.from_me);
        assert_eq!(
            row.content,
            Content::Call {
                outgoing: true,
                status: CallStatus::Answered,
                seconds: Some(95),
                video: false,
                group: None,
            }
        );
    }

    #[test]
    fn the_phones_record_of_a_call_seen_here_completes_it_once() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let began = crate::util::now() - 120;
        // Logged here while still going, under the call's id.
        worker.log_call(CallEntry {
            id: format!("{CALL_ROW_PREFIX}same"),
            chat: PEER.into(),
            began,
            outgoing: false,
            status: CallStatus::Ongoing,
            seconds: None,
            video: false,
            group: None,
            quiet: false,
        });
        let mut done = record(CallResult::CONNECTED, began);
        done.duration = Some(40);
        worker.call_log_synced(&synced("same", PEER, false, done.clone()));
        // The same call again, as a record with another id a few seconds off.
        worker.call_log_synced(&synced("other-id", PEER, false, {
            done.start_time = Some(began + 5);
            done
        }));
        assert_eq!(calls_in(&worker, PEER), 1);
        assert!(matches!(
            row(&worker, PEER, "same").unwrap().content,
            Content::Call {
                status: CallStatus::Answered,
                seconds: Some(40),
                ..
            }
        ));
    }

    #[test]
    fn a_call_deleted_here_does_not_come_back() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let call = record(CallResult::REJECTED, crate::util::now() - 4000);
        worker.call_log_synced(&synced("gone", PEER, false, call.clone()));
        let id = format!("{CALL_ROW_PREFIX}gone");
        worker.remove_call(PEER, &id);
        assert!(row(&worker, PEER, "gone").is_none());
        assert!(events.try_iter().any(
            |event| matches!(event, Event::MessageDeleted { id: deleted, .. } if deleted == id)
        ));
        worker.call_log_synced(&synced("gone", PEER, false, call));
        assert!(row(&worker, PEER, "gone").is_none());
    }

    #[test]
    fn history_brings_the_phones_call_log_quietly() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.me_pn = Some(ME.into());
        let group = "120363000000000001@g.us";
        let mut in_group = record(CallResult::MISSED, crate::util::now() - 60);
        in_group.call_id = Some("group-call".into());
        in_group.call_creator_jid = Some(PEER.into());
        in_group.group_jid = Some(group.into());
        in_group.participants = vec![
            ParticipantInfo {
                user_jid: Some(PEER.into()),
                ..Default::default()
            },
            ParticipantInfo {
                user_jid: Some("4917600000002@s.whatsapp.net".into()),
                ..Default::default()
            },
        ];
        let mut upcoming = record(CallResult::UPCOMING, crate::util::now() + 3600);
        upcoming.call_id = Some("later".into());
        upcoming.call_creator_jid = Some(PEER.into());
        worker.file_history_calls(&[in_group, upcoming]);
        let row = row(&worker, group, "group-call").expect("filed in the group");
        assert!(matches!(
            row.content,
            Content::Call {
                status: CallStatus::Missed,
                group: Some(3),
                ..
            }
        ));
        assert!(self::row(&worker, PEER, "later").is_none());
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Incoming { .. }))
        );
    }

    #[test]
    fn a_call_log_message_takes_its_direction_from_the_envelope() {
        use wa::message::call_log_message::CallOutcome;
        let log = wa::message::CallLogMessage {
            call_outcome: Some(CallOutcome::MISSED),
            ..Default::default()
        };
        assert_eq!(call_log_message_status(&log, false), CallStatus::Missed);
        assert_eq!(call_log_message_status(&log, true), CallStatus::NoAnswer);
        let mut content = classify_base(&wa::Message {
            call_log_messsage: MessageField::some(log),
            ..Default::default()
        })
        .unwrap();
        content.set_call_direction(true);
        assert!(matches!(
            content,
            Content::Call {
                outgoing: true,
                status: CallStatus::NoAnswer,
                ..
            }
        ));
    }
}
