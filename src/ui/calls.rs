//! Calls as the history shows them: the entry a call leaves among a chat's
//! messages, and the Calls page that lists every call of the account,
//! newest first.
//!
//! Both read the same entries, which come from WhatsApp's own records (the
//! phone's call log and the call-log messages in history) and from the calls
//! ZapFast took part in. Nothing here starts a call except the call-back
//! buttons.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense};

use crate::app::App;
use crate::i18n::{Locale, gettext};
use crate::model::{Action, CallStatus, Content, Message, Page};
use crate::theme::{self, Icon};
use crate::ui::widgets;

/// The height of one row on the Calls page.
const ROW: f32 = 62.0;

/// One call, as its entry reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Whether this account placed the call.
    pub from_me: bool,
    pub video: bool,
    pub status: CallStatus,
    /// Seconds the call was up.
    pub duration: u64,
    pub group: bool,
}

impl Entry {
    /// The entry of a call message, if it is one.
    pub fn of(message: &Message) -> Option<Self> {
        match message.content {
            Content::Call {
                video,
                status,
                duration,
                group,
            } => Some(Self {
                from_me: message.from_me,
                video,
                status,
                duration,
                group,
            }),
            _ => None,
        }
    }

    /// One glyph for the state: the arrow says which way an answered call
    /// went, the camera what it carried, and a call that never connected has
    /// the missed or the slashed handset.
    pub fn icon(&self) -> Icon {
        match self.status {
            CallStatus::Missed | CallStatus::NoAnswer => Icon::PhoneMissed,
            CallStatus::Declined
            | CallStatus::DeclinedElsewhere
            | CallStatus::Busy
            | CallStatus::Failed
            | CallStatus::ConnectionLost => Icon::PhoneOff,
            CallStatus::Answered | CallStatus::AnsweredElsewhere => {
                if self.video {
                    Icon::Video
                } else if self.from_me {
                    Icon::PhoneOutgoing
                } else {
                    Icon::PhoneIncoming
                }
            }
        }
    }

    /// What the call was: "Missed voice call", "Video call", "Group voice call".
    pub fn title(&self, locale: Locale) -> String {
        let missed = self.status == CallStatus::Missed && !self.from_me;
        let text = match (missed, self.group, self.video) {
            (true, false, false) => gettext(locale, "Missed voice call"),
            (true, false, true) => gettext(locale, "Missed video call"),
            (true, true, false) => gettext(locale, "Missed group voice call"),
            (true, true, true) => gettext(locale, "Missed group video call"),
            (false, false, false) => gettext(locale, "Voice call"),
            (false, false, true) => gettext(locale, "Video call"),
            (false, true, false) => gettext(locale, "Group voice call"),
            (false, true, true) => gettext(locale, "Group video call"),
        };
        text.into_owned()
    }

    /// How it went: its length when it was up, else what became of it. A
    /// missed call says it in its title.
    pub fn detail(&self, locale: Locale) -> Option<String> {
        let text = match self.status {
            CallStatus::Answered if self.duration > 0 => {
                return Some(crate::util::call_duration(self.duration));
            }
            CallStatus::Answered | CallStatus::Missed => return None,
            CallStatus::AnsweredElsewhere => gettext(locale, "Answered on another device"),
            CallStatus::Declined => gettext(locale, "Declined"),
            CallStatus::DeclinedElsewhere => gettext(locale, "Declined on another device"),
            CallStatus::Busy => gettext(locale, "Busy"),
            CallStatus::Failed => gettext(locale, "Could not connect"),
            CallStatus::NoAnswer => gettext(locale, "No answer"),
            CallStatus::ConnectionLost => gettext(locale, "Connection lost"),
        };
        Some(text.into_owned())
    }
}

/// The Calls page.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    header(app, ui);
    ui.add_space(2.0);
    // A locked chat's calls stay out while its folder is closed.
    let calls: Vec<Message> = app
        .call_log
        .iter()
        .filter(|message| {
            app.chat(&message.chat)
                .is_none_or(|chat| !chat.locked || app.locked_folder_open())
        })
        .cloned()
        .collect();
    if calls.is_empty() {
        let (title, body) = if app.call_log_loaded {
            (
                gettext(app.locale, "No calls yet"),
                gettext(
                    app.locale,
                    "Calls from your phone's call log and the ones you make here appear here.",
                ),
            )
        } else {
            (
                gettext(app.locale, "Loading…"),
                gettext(app.locale, "Reading this account's calls."),
            )
        };
        widgets::empty_state(ui, &palette, Icon::Phone, title.as_ref(), body.as_ref());
        return;
    }
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for message in &calls {
                row(app, ui, message, &mut actions);
            }
            ui.add_space(12.0);
        });
    app.actions.extend(actions);
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if theme::icon_button(
                    ui,
                    Icon::ArrowLeft,
                    18.0,
                    palette.secondary,
                    palette.text,
                    gettext(app.locale, "Back to the chats").as_ref(),
                )
                .clicked()
                {
                    app.actions.push(Action::Open(Page::Chats));
                }
                ui.add_space(2.0);
                theme::text(
                    ui,
                    gettext(app.locale, "Calls"),
                    theme::bold(20.0),
                    palette.text,
                );
            });
        });
}

/// One call: who, which way, how it went, when, and the way to call back.
fn row(app: &mut App, ui: &mut egui::Ui, message: &Message, actions: &mut Vec<Action>) {
    let Some(entry) = Entry::of(message) else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let name = app.display_name(&message.chat);
    let picture = app.avatar(&message.chat);
    // The row's own target goes first, so the call-back buttons inside keep
    // their clicks.
    let row = ui
        .interact(
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), ROW)),
            ui.id().with(("call-row", &message.chat, &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    Frame::new()
        .fill(if row.hovered() {
            palette.surface_hover
        } else {
            palette.panel
        })
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_height(ROW - 16.0);
            ui.horizontal(|ui| {
                widgets::avatar(ui, &palette, &name, &message.chat, 40.0, picture.as_deref());
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    theme::text(ui, &name, theme::semibold(14.5), palette.text);
                    ui.add_space(1.0);
                    ui.horizontal(|ui| {
                        theme::icon(
                            ui,
                            entry.icon(),
                            16.0,
                            if entry.status.is_bad() {
                                palette.danger
                            } else {
                                palette.accent
                            },
                        );
                        ui.add_space(6.0);
                        let line = match entry.detail(locale) {
                            Some(detail) => format!("{} · {detail}", entry.title(locale)),
                            None => entry.title(locale),
                        };
                        theme::text(ui, line, theme::medium(12.5), palette.secondary);
                    });
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !entry.group {
                        let video = gettext(locale, "Video call").into_owned();
                        if theme::icon_button(
                            ui,
                            Icon::Video,
                            16.0,
                            palette.secondary,
                            palette.text,
                            &video,
                        )
                        .clicked()
                        {
                            actions.push(Action::StartVideoCall(message.chat.clone()));
                        }
                        let voice = gettext(locale, "Voice call").into_owned();
                        if theme::icon_button(
                            ui,
                            Icon::Phone,
                            16.0,
                            palette.secondary,
                            palette.text,
                            &voice,
                        )
                        .clicked()
                        {
                            actions.push(Action::StartCall(message.chat.clone()));
                        }
                    }
                    ui.add_space(8.0);
                    theme::text(
                        ui,
                        crate::util::chat_stamp(locale, message.timestamp),
                        theme::medium(12.0),
                        palette.secondary,
                    );
                });
            });
        });
    let label = gettext(locale, "Open the chat with {name}").replace("{name}", &name);
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.as_str()));
    theme::reveal_focus(&row);
    if row.clicked() {
        actions.push(Action::Open(Page::Chats));
        actions.push(Action::OpenMessage {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(from_me: bool, video: bool, status: CallStatus) -> Entry {
        Entry {
            from_me,
            video,
            status,
            duration: 0,
            group: false,
        }
    }

    #[test]
    fn the_states_a_reader_tells_apart_have_different_glyphs() {
        let glyphs = [
            entry(false, false, CallStatus::Answered).icon(),
            entry(true, false, CallStatus::Answered).icon(),
            entry(false, false, CallStatus::Missed).icon(),
            entry(false, false, CallStatus::Declined).icon(),
            entry(true, true, CallStatus::Answered).icon(),
        ];
        for (index, first) in glyphs.iter().enumerate() {
            for second in &glyphs[index + 1..] {
                assert_ne!(first, second);
            }
        }
    }

    #[test]
    fn an_entry_says_what_the_call_was_and_how_it_went() {
        let locale = Locale::default();
        let mut answered = entry(true, false, CallStatus::Answered);
        answered.duration = 221;
        assert_eq!(answered.title(locale), "Voice call");
        assert_eq!(answered.detail(locale).as_deref(), Some("3:41"));
        let missed = entry(false, true, CallStatus::Missed);
        assert_eq!(missed.title(locale), "Missed video call");
        assert_eq!(missed.detail(locale), None, "the title already says it");
        let refused = entry(true, false, CallStatus::NoAnswer);
        assert_eq!(refused.detail(locale).as_deref(), Some("No answer"));
    }
}
