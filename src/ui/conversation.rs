//! The open chat: its header, the messages, and the composer.

use crate::i18n::tr;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers,
    Rect, Sense, Stroke, Vec2, pos2, vec2,
};

use crate::animation;
use crate::app::{App, Conversation, JumpHighlight, KeyScroll, RowHeight};
use crate::markup;
use crate::model::{
    Action, Chat, ChatId, ComposerTextCommand, Content, Delivery, Dialog, LinkPreview, Media,
    MediaState, Message, PickerTab, Reaction, Scroll,
};
use crate::theme::{self, Icon, Palette};
use crate::wallpaper;

use super::focus::{Stop, TabStop};
use super::widgets;

/// Group-message avatar size.
const SENDER_AVATAR: f32 = 28.0;
const BODY_SIZE: f32 = 14.5;
/// Extra space above the first message of a run from one side.
const RUN_GAP: f32 = 5.0;
/// Footer label on an outgoing message that failed to send.
const NOT_SENT: &str = crate::i18n::n_("Not sent");
const NOT_SENT_HINT: &str = crate::i18n::n_(
    "This message could not be sent, and ZapFast will not retry it. Send it again yourself.",
);

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(chat) = app.current_chat().cloned() else {
        if theme::macos_chrome(ui.ctx()) {
            super::banner(app, ui);
        }
        empty(app, ui);
        return;
    };
    wallpaper::paint(ui, &app.wallpaper());
    let header = header(app, ui, &chat);
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    composer(app, ui, &chat);
    // Attachments waiting to be sent take the history's place, as on the
    // phone, with the composer below for their caption.
    if app.pending.is_empty() {
        messages(app, ui, &chat);
    } else {
        pending_preview(app, ui);
    }
    // Over the messages, which scroll under the header.
    widgets::paint_shadow_below(
        ui,
        &app.palette,
        header.left(),
        header.right(),
        header.bottom(),
    );
    // The chat list, or its rail of avatars, stands at the header's level
    // beside the conversation: it casts the same shadow across it, from
    // under the header down.
    widgets::paint_shadow_beside(
        ui,
        &app.palette,
        header.left(),
        header.bottom(),
        ui.max_rect().bottom(),
    );
}

fn empty(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let rect = ui.max_rect();
    let center = rect.center() - vec2(0.0, 30.0);
    theme::logo(
        ui,
        center - vec2(0.0, 60.0),
        72.0,
        palette.surface,
        palette.dim,
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "ZapFast",
        theme::bold(24.0),
        palette.text,
    );
    ui.painter().text(
        center + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        if app.chats.is_empty() {
            tr("Your chats appear on the left as they load.")
        } else {
            tr("Select a chat on the left.")
        },
        theme::regular(14.0),
        palette.secondary,
    );
    if app.settings.show_shortcut_hints {
        ui.painter().text(
            center + vec2(0.0, 56.0),
            Align2::CENTER_CENTER,
            super::keys::label(
                crate::i18n::gettext(app.locale, "Ctrl+K to search · ? for keyboard shortcuts")
                    .as_ref(),
            ),
            theme::regular(12.5),
            palette.dim,
        );
    }
}

fn header(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> Rect {
    let palette = app.palette;
    let title = app.chat_title(chat);
    egui::Panel::top("chat-header")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            // The chat list, expanded or collapsed, clears the traffic lights.
            if theme::macos_chrome(ui.ctx()) {
                super::titlebar_drag(ui, ui.max_rect());
            }
            ui.horizontal(|ui| {
                // Give both rows a fixed height so their contents align.
                ui.set_min_height(HEADER_ROW);
                let picture = app.avatar(&chat.id);
                let (subtitle, color) = subtitle(app, chat);
                // The call buttons reflect the backend's own state: this chat's call is the one
                // the worker owns, and nothing about it is inferred here.
                let call_here = app.call.as_ref().is_some_and(|call| call.chat == chat.id);
                let right_controls = 108.0;
                // Treat the avatar, name, and subtitle as one info button.
                let block = ui
                    .scope(|ui| {
                        // Fix the child height before centering its contents.
                        ui.allocate_ui_with_layout(
                            vec2(
                                (ui.available_width() - right_controls).max(80.0),
                                HEADER_ROW,
                            ),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                let avatar_response = widgets::avatar(
                                    ui,
                                    &palette,
                                    &title,
                                    &chat.id,
                                    40.0,
                                    picture.as_deref(),
                                );
                                if chat.ephemeral_expiration.is_some() {
                                    widgets::paint_disappearing_badge(
                                        ui,
                                        &palette,
                                        avatar_response.rect,
                                    );
                                }
                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    let width = (ui.available_width() - right_controls).max(80.0);
                                    ui.set_max_width(width);
                                    if subtitle.is_empty() {
                                        // Center the name on the avatar.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(17.0),
                                                    palette.text,
                                                );
                                            },
                                        );
                                    } else {
                                        // Align the name and subtitle with the avatar edges.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::top_down(Align::Min),
                                            |ui| {
                                                widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(15.0),
                                                    palette.text,
                                                );
                                                ui.with_layout(
                                                    Layout::bottom_up(Align::Min),
                                                    |ui| {
                                                        widgets::rich_text(
                                                            ui,
                                                            &subtitle,
                                                            theme::regular(12.5),
                                                            color,
                                                        );
                                                    },
                                                );
                                            },
                                        );
                                    }
                                });
                            },
                        );
                    })
                    .response;
                let block = ui
                    .interact(block.rect, ui.id().with("chat-header-info"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if block.clicked() {
                    app.actions
                        .push(Action::ShowDialog(Dialog::ChatInfo(chat.id.clone())));
                }
                // The item and the width that has to hold it are measured from
                // the same localized label: a translation wider than the
                // English one would otherwise be clipped.
                let leave_label = if chat.is_channel() {
                    crate::i18n::gettext(app.locale, "Leave channel")
                } else {
                    crate::i18n::gettext(app.locale, "Leave group")
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let more = theme::icon_button(
                        ui,
                        Icon::Ellipsis,
                        18.0,
                        palette.secondary,
                        palette.text,
                        tr("More"),
                    );
                    let width = widgets::menu_width(
                        ui,
                        &[
                            tr("Info"),
                            tr("Pin to top"),
                            tr("Unarchive"),
                            tr("Clear chat"),
                            tr("Block…"),
                            tr("Unblock"),
                            leave_label.as_ref(),
                            tr("Copy number"),
                            tr("Close chat"),
                        ],
                        true,
                    );
                    // Demo/test: hold this menu open for a screenshot.
                    #[cfg(any(test, feature = "demo"))]
                    if app.open_header_menu.as_deref() == Some(chat.id.as_str()) {
                        egui::Popup::open_id(ui.ctx(), more.id.with("popup"));
                    }
                    egui::Popup::menu(&more)
                        .width(width)
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            if widgets::menu_item(ui, &palette, Some(Icon::Info), tr("Info")) {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ChatInfo(chat.id.clone())));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(if chat.pinned { Icon::PinOff } else { Icon::Pin }),
                                if chat.pinned {
                                    tr("Unpin")
                                } else {
                                    tr("Pin to top")
                                },
                            ) {
                                app.actions
                                    .push(Action::SetPinned(chat.id.clone(), !chat.pinned));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Archive),
                                if chat.archived {
                                    tr("Unarchive")
                                } else {
                                    tr("Archive")
                                },
                            ) {
                                app.actions
                                    .push(Action::SetArchived(chat.id.clone(), !chat.archived));
                            }
                            // Clearing reaches the phone, so it waits for a
                            // connection, as deleting does in the chat list.
                            if widgets::menu_item_enabled(
                                ui,
                                &palette,
                                Some(Icon::Eraser),
                                tr("Clear chat"),
                                app.is_connected(),
                            ) {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmClearChat(
                                        chat.id.clone(),
                                    )));
                            }
                            super::chats::block_menu_item(app, ui, &palette, chat);
                            widgets::menu_separator(ui, &palette);
                            if chat.can_leave(&app.our_ids())
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::LogOut),
                                    leave_label.as_ref(),
                                )
                            {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(
                                        chat.id.clone(),
                                    )));
                            }
                            if let Some(phone) = chat.phone()
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Copy),
                                    tr("Copy number"),
                                )
                            {
                                app.actions.push(Action::CopyText(format!("+{phone}")));
                            }
                            if widgets::menu_item(ui, &palette, Some(Icon::X), tr("Close chat")) {
                                app.actions.push(Action::CloseChat);
                            }
                        });
                    // A call is one to one, and it needs the platform's media backend: a group, a
                    // channel or a broadcast list has no phone button here, and neither has any
                    // chat on a platform whose backend cannot open a microphone, where a call
                    // would fail on its first frame. The worker refuses those JIDs whatever this
                    // header offers, so nothing can be started behind the interface's back either.
                    // A live call still offers its hang-up button, which can only exist where the
                    // backend does.
                    let calls_here = crate::calls::capabilities();
                    // A blocked contact cannot be called, as on the phone.
                    if chat.kind == crate::model::ChatKind::Direct
                        && ((calls_here.voice && !chat.blocked) || call_here)
                    {
                        // While this chat is the one on a call, the phone button ends it;
                        // otherwise it starts a voice call. A call in another chat is refused by
                        // the worker rather than hidden here.
                        let (call_tooltip, call_icon, call_fill, call) = if call_here {
                            (
                                crate::i18n::gettext(app.locale, "Hang up").into_owned(),
                                Icon::Phone,
                                palette.danger,
                                Action::HangupCall,
                            )
                        } else {
                            (
                                crate::i18n::gettext(app.locale, "Voice call").into_owned(),
                                Icon::Phone,
                                palette.secondary,
                                Action::StartCall(chat.id.clone()),
                            )
                        };
                        if theme::icon_button(
                            ui,
                            call_icon,
                            18.0,
                            call_fill,
                            palette.text,
                            &call_tooltip,
                        )
                        .clicked()
                        {
                            app.actions.push(call);
                        }
                    }
                    let searching = app.chat_search_open;
                    let tip = format!(
                        "{} ({})",
                        crate::i18n::gettext(app.locale, "Search messages"),
                        super::keys::label("Ctrl+F")
                    );
                    if theme::icon_button(
                        ui,
                        Icon::Search,
                        18.0,
                        if searching {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        &tip,
                    )
                    .tab_stop(Stop::ChatSearch)
                    .clicked()
                    {
                        app.actions.push(if searching {
                            Action::CloseChatSearch
                        } else {
                            Action::OpenChatSearch
                        });
                    }
                });
            });
        })
        .response
        .rect
}

/// Chat-header subtitle.
fn subtitle(app: &App, chat: &Chat) -> (String, Color32) {
    let palette = app.palette;
    let typing = app.typing_in(&chat.id);
    if !typing.is_empty() {
        let text = if chat.is_group() {
            let names: Vec<&str> = typing.iter().map(|(_, name)| name.as_str()).collect();
            match names.as_slice() {
                [] => String::new(),
                [one] => tr("{name} is typing…").replace("{name}", one),
                [rest @ .., last] => tr("{names} and {last} are typing…")
                    .replace("{names}", &rest.join(", "))
                    .replace("{last}", last),
            }
        } else {
            tr("typing…").to_owned()
        };
        return (text, palette.accent);
    }
    if chat.is_group() {
        let names = app.participant_names(chat);
        return (
            if names.is_empty() {
                tr("Group").to_owned()
            } else {
                names
            },
            palette.secondary,
        );
    }
    if let Some(presence) = app.presence.get(&chat.id) {
        if presence.online {
            return (tr("online").to_owned(), palette.accent);
        }
        if let Some(seen) = presence.last_seen {
            return (crate::util::last_seen(app.locale, seen), palette.secondary);
        }
    }
    match chat.phone() {
        Some(phone) if !app.is_saved_contact(&chat.id) => {
            (crate::util::phone(phone), palette.secondary)
        }
        _ => (String::new(), palette.secondary),
    }
}

/// Byte position of a freshly typed standalone trigger immediately before
/// the text cursor. Colons inside times and URLs, and `@` inside addresses,
/// remain ordinary text.
/// Linux's primary selection in the composer: finished selections are
/// offered to other applications, and a middle click pastes the selection
/// from any of them where it was pressed.
fn primary_selection(
    app: &mut App,
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    response: &egui::Response,
    id: egui::Id,
) {
    let frame = ui.ctx().cumulative_frame_nr();
    if response.clicked_by(egui::PointerButton::Middle) && app.primary_paste_frame != frame {
        app.primary_paste_frame = frame;
        if let Some(text) = crate::app::primary_selection() {
            app.primary_paste = Some(text);
            ui.memory_mut(|memory| memory.request_focus(id));
            ui.ctx().request_repaint();
        }
        return;
    }
    // Offer a selection once it is made, not at every step of a drag.
    if ui.input(|input| input.pointer.any_down()) || !response.has_focus() {
        return;
    }
    let selected = output
        .cursor_range
        .map(|range| {
            let (start, end) = (range.primary.index.0, range.secondary.index.0);
            let (start, end) = (start.min(end), start.max(end));
            app.composer
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
        })
        .unwrap_or_default();
    if selected.is_empty() {
        // Selecting the same text again takes the selection back.
        app.primary_offered.clear();
    } else if selected != app.primary_offered {
        crate::app::offer_primary_selection(&selected);
        app.primary_offered = selected;
    }
}

fn standalone_trigger(text: &str, cursor: usize, trigger: char) -> Option<usize> {
    let cursor = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let (at, found) = text[..cursor].char_indices().next_back()?;
    if found != trigger {
        return None;
    }
    (at == 0
        || text[..at]
            .chars()
            .next_back()
            .is_some_and(|character| !character.is_alphanumeric()))
    .then_some(at)
}

/// Active mention query from its `@` through the current text cursor.
fn active_mention(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(start.checked_add(1)?..end)?;
    (!query.contains(['@', '\n'])).then_some((end, query))
}

/// Active emoji query from its `:` through the current text cursor. Spaces
/// and punctuation end autocomplete without changing what the user typed.
fn active_emoji(text: &str, start: Option<usize>, cursor: usize) -> Option<(usize, &str)> {
    let start = start?;
    let after = start.checked_add(1)?;
    if text.get(start..after) != Some(":") {
        return None;
    }
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let query = text.get(after..end)?;
    query
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '+'))
        .then_some((end, query))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EmojiSuggestion {
    emoji: &'static str,
    shortcode: String,
    name: &'static str,
}

fn emoji_match_score(emoji: &emojis::Emoji, query: &str) -> Option<u8> {
    let name = emoji.name();
    let shortcodes = emoji.shortcodes();
    if shortcodes.clone().any(|code| code == query) {
        Some(0)
    } else if shortcodes.clone().any(|code| code.starts_with(query)) {
        Some(1)
    } else if name == query {
        Some(2)
    } else if name.starts_with(query)
        || name
            .split([' ', '-', '_'])
            .any(|word| word.starts_with(query))
    {
        Some(3)
    } else if shortcodes.clone().any(|code| code.contains(query)) {
        Some(4)
    } else if name.contains(query) {
        Some(5)
    } else {
        None
    }
}

fn emoji_suggestion(emoji: &'static emojis::Emoji) -> EmojiSuggestion {
    let shortcode = emoji
        .shortcode()
        .map_or_else(|| emoji.name().replace([' ', '-'], "_"), str::to_owned);
    EmojiSuggestion {
        emoji: emoji.as_str(),
        shortcode: format!(":{shortcode}:"),
        name: emoji.name(),
    }
}

/// Completions for `:query`. `emojis::iter` yields each emoji once, and a
/// skin-tone-capable one such as 👍 reports `Some(SkinTone::Default)`.
fn emoji_candidates(app: &App, query: &str) -> Vec<EmojiSuggestion> {
    const LIMIT: usize = 6;
    let query = query.to_lowercase();
    let mut seen = HashSet::new();
    if query.is_empty() {
        let recent = app
            .settings
            .recent_emoji
            .iter()
            .filter_map(|emoji| emojis::get(emoji))
            .chain(emojis::iter())
            .filter(|emoji| seen.insert(emoji.as_str()))
            .take(LIMIT)
            .map(emoji_suggestion)
            .collect();
        return recent;
    }

    let mut found: Vec<_> = emojis::iter()
        .enumerate()
        .filter_map(|(order, emoji)| {
            emoji_match_score(emoji, &query).map(|score| (score, order, emoji))
        })
        .collect();
    found.sort_by_key(|(score, order, _)| (*score, *order));
    found
        .into_iter()
        .filter(|(_, _, emoji)| seen.insert(emoji.as_str()))
        .take(LIMIT)
        .map(|(_, _, emoji)| emoji_suggestion(emoji))
        .collect()
}

fn take_plain_key(ui: &mut egui::Ui, key: Key) -> bool {
    ui.input_mut(|input| {
        let mut taken = false;
        input.events.retain(|event| {
            if taken {
                return true;
            }
            let matches = matches!(
                event,
                egui::Event::Key {
                    key: found,
                    pressed: true,
                    modifiers,
                    ..
                } if *found == key && *modifiers == Modifiers::NONE
            );
            taken |= matches;
            !matches
        });
        taken
    })
}

/// Slack-style emoji suggestions above the composer. The composer keeps
/// focus, so ordinary typing continues refining the query.
fn emoji_suggestions(app: &mut App, ui: &mut egui::Ui, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_emoji(&app.composer, app.emoji_start, cursor) else {
        app.emoji_start = None;
        return;
    };
    let start = app.emoji_start.expect("checked above");
    let candidates = emoji_candidates(app, query);
    if candidates.is_empty() {
        return;
    }

    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.emoji_selected = (app.emoji_selected + 1) % candidates.len();
    }
    if up {
        app.emoji_selected = (app.emoji_selected + candidates.len() - 1) % candidates.len();
    }
    app.emoji_selected = app.emoji_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.emoji_selected].clone());
    let palette = app.palette;

    widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
        let row_height = 36.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, candidate) in candidates.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
            if index == app.emoji_selected {
                ui.painter().rect_filled(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    palette.accent.gamma_multiply(0.18),
                );
                ui.painter().rect_stroke(
                    rect,
                    SUGGESTION_ROW_RADIUS,
                    Stroke::new(1.0, palette.accent),
                    egui::StrokeKind::Inside,
                );
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(rect, SUGGESTION_ROW_RADIUS, palette.surface_hover);
            }

            let emoji = widgets::line(
                ui,
                candidate.emoji,
                theme::regular(22.0),
                palette.text,
                30.0,
                1,
            );
            emoji.paint(
                ui,
                pos2(rect.left() + 6.0, rect.center().y - emoji.size().y / 2.0),
                palette.text,
            );
            let shortcode = widgets::line(
                ui,
                &candidate.shortcode,
                theme::medium(13.0),
                palette.text,
                (rect.width() * 0.4).max(100.0),
                1,
            );
            let text_x = rect.left() + 42.0;
            shortcode.paint(
                ui,
                pos2(text_x, rect.center().y - shortcode.size().y / 2.0),
                palette.text,
            );
            let name_x = text_x + shortcode.size().x + 12.0;
            let name = widgets::line(
                ui,
                candidate.name,
                theme::regular(12.5),
                palette.secondary,
                (rect.right() - name_x - 8.0).max(0.0),
                1,
            );
            name.paint(
                ui,
                pos2(name_x, rect.center().y - name.size().y / 2.0),
                palette.secondary,
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.emoji_selected = index;
                picked = Some(candidate.clone());
            }
        }
    });
    strip_gap(ui);
    if let Some(candidate) = picked {
        app.actions.push(Action::InsertEmojiCompletion {
            emoji: candidate.emoji.to_owned(),
            start,
            end,
        });
    }
}

/// Group-member suggestions above the composer.
fn mention_picker(app: &mut App, ui: &mut egui::Ui, chat: &Chat, field: egui::Id) {
    let cursor = egui::TextEdit::load_state(ui.ctx(), field)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0)
        .unwrap_or_else(|| app.composer.chars().count());
    let Some((end, query)) = active_mention(&app.composer, app.mention_start, cursor) else {
        app.mention_start = None;
        return;
    };
    let start = app.mention_start.expect("checked above");
    let candidates = app.mention_candidates(chat, query);
    if candidates.is_empty() {
        return;
    }
    let down = take_plain_key(ui, Key::ArrowDown);
    let up = take_plain_key(ui, Key::ArrowUp);
    if down {
        app.mention_selected = (app.mention_selected + 1) % candidates.len();
    }
    if up {
        app.mention_selected = (app.mention_selected + candidates.len() - 1) % candidates.len();
    }
    app.mention_selected = app.mention_selected.min(candidates.len() - 1);
    let submit = take_plain_key(ui, Key::Enter) || take_plain_key(ui, Key::Tab);
    let mut picked = submit.then(|| candidates[app.mention_selected].clone());
    let palette = app.palette;

    widgets::raised(ui, &palette, suggestion_frame(&palette), |ui| {
        let row_height = 38.0;
        egui::ScrollArea::vertical()
            .id_salt("mention-members")
            .max_height(row_height * candidates.len().min(5) as f32)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, candidates.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let (id, label) = &candidates[index];
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), row_height),
                        Sense::click(),
                    );
                    if index == app.mention_selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            SUGGESTION_ROW_RADIUS,
                            palette.surface_hover,
                        );
                    }
                    let avatar = Rect::from_center_size(
                        pos2(rect.left() + 19.0, rect.center().y),
                        Vec2::splat(28.0),
                    );
                    let picture = app.avatar(id);
                    widgets::paint_avatar(
                        ui,
                        &palette,
                        avatar,
                        label.trim_start_matches('~'),
                        id,
                        picture.as_deref(),
                    );
                    let detail = crate::model::phone_of(id)
                        .map(crate::util::phone)
                        .unwrap_or_default();
                    let detail = widgets::line(
                        ui,
                        &detail,
                        theme::regular(11.5),
                        palette.secondary,
                        (rect.width() * 0.36).min(150.0),
                        1,
                    );
                    let name = widgets::line(
                        ui,
                        label,
                        theme::medium(13.5),
                        palette.text,
                        rect.width() - detail.size().x - 62.0,
                        1,
                    );
                    name.paint(
                        ui,
                        pos2(rect.left() + 40.0, rect.center().y - name.size().y / 2.0),
                        palette.text,
                    );
                    detail.paint(
                        ui,
                        pos2(
                            rect.right() - detail.size().x - 8.0,
                            rect.center().y - detail.size().y / 2.0,
                        ),
                        palette.secondary,
                    );
                    if response.hovered() {
                        app.mention_selected = index;
                    }
                    if (down || up) && index == app.mention_selected {
                        response.scroll_to_me(Some(Align::Center));
                    }
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        picked = Some((id.clone(), label.clone()));
                    }
                }
            });
    });
    strip_gap(ui);
    if let Some((id, name)) = picked {
        app.actions.push(Action::InsertMention {
            id,
            name,
            start,
            end,
        });
    }
}

fn composer(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    // The composer floats over the wallpaper; the bar of selected messages is
    // a solid strip, so it reads as a bar and not as text on the wallpaper.
    let selecting = app
        .selection
        .as_ref()
        .is_some_and(|(selected_chat, _)| *selected_chat == chat.id);
    let shown = egui::Panel::bottom("composer")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(if selecting {
                    palette.panel
                } else {
                    Color32::TRANSPARENT
                })
                .inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: if selecting { 8 } else { 0 },
                    bottom: 8,
                }),
        )
        .show(ui, |ui| {
            if let Some((selected_chat, selected)) = app.selection.clone()
                && selected_chat == chat.id
            {
                selection_bar(app, ui, &chat.id, &selected);
                return;
            }
            if !chat.can_send() {
                if chat.kind == crate::model::ChatKind::Broadcast {
                    // A channel we left says so; the rest are only read-only.
                    ui.vertical_centered(|ui| {
                        theme::text(
                            ui,
                            if chat.left {
                                crate::i18n::gettext(app.locale, "You left this channel")
                            } else {
                                crate::i18n::gettext(
                                    app.locale,
                                    "Channels are read-only in ZapFast",
                                )
                            }
                            .as_ref(),
                            theme::regular(13.5),
                            palette.secondary,
                        );
                    });
                    return;
                }
                if chat.blocked {
                    blocked_strip(app, ui, chat);
                    return;
                }
                if chat.locked {
                    ui.vertical_centered(|ui| {
                        theme::text(ui, tr("Locked chats are read-only in ZapFast"), theme::regular(13.5), palette.secondary);
                    });
                    return;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    // A group we left says so instead of blaming the admins:
                    // either we left it here, or the phone says we are no
                    // longer a member.
                    let ours = app.our_ids();
                    let left = chat.left
                        || (!ours.is_empty()
                            && !chat.participants.is_empty()
                            && !chat.lists_any(&ours));
                    if left {
                        theme::text(
                            ui,
                            crate::i18n::gettext(app.locale, "You left this group"),
                            theme::regular(13.5),
                            palette.secondary,
                        );
                    } else {
                        ui.horizontal(|ui| {
                            let width = 230.0;
                            ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                            theme::text(ui, tr("Only"), theme::regular(13.5), palette.secondary);
                            theme::text(ui, tr("admins"), theme::semibold(13.5), palette.accent);
                            theme::text(
                                ui,
                                tr("can send messages"),
                                theme::regular(13.5),
                                palette.secondary,
                            );
                        });
                    }
                    ui.add_space(8.0);
                });
                return;
            }
            // A refused voice message waits in its own chat only.
            let unsent_voice = app
                .unsent_voice
                .as_ref()
                .filter(|(unsent, _)| *unsent == chat.id)
                .map(|(_, samples)| samples.len());
            if let Some(samples) = unsent_voice
                && app.editing.is_none()
            {
                unsent_voice_strip(app, ui, samples);
            }
            if app.editing.is_some() {
                edit_strip(app, ui);
            } else if let Some(reply_id) = app.reply_to.clone() {
                let quoted = app
                    .conversations
                    .get(&chat.id)
                    .and_then(|conversation| conversation.message(&reply_id))
                    .cloned();
                match quoted {
                    Some(quoted) => reply_strip(app, ui, &quoted),
                    None => app.reply_to = None,
                }
            }
            // The first link in the text gets a preview, as on the phone, but
            // not in an edit or a caption.
            let link_previews =
                app.settings.link_previews && app.editing.is_none() && app.pending.is_empty();
            if link_previews
                && app.recording.is_none()
                && let Some(link) = app
                    .composer_link
                    .as_ref()
                    .filter(|link| link.chat == chat.id && !link.dismissed)
            {
                link_strip(ui, &palette, link, &mut app.actions);
            }
            let id = egui::Id::new("composer-text");
            let has_focus = ui.memory(|memory| memory.has_focus(id));
            let enter_sends = app.settings.enter_sends;
            let (typed_colon, typed_at) = ui.input(|input| {
                let typed = |needle: &str| {
                    input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(text) if text == needle))
                };
                (has_focus && typed(":"), has_focus && typed("@"))
            });
            if app.recording.is_some() {
                widgets::raised(ui, &palette, composer_pill(&palette), |ui| {
                    recording_strip(app, ui)
                });
                return;
            }
            emoji_suggestions(app, ui, id);
            mention_picker(app, ui, chat, id);
            // `consume_key(NONE, Enter)` also matches Shift+Enter. Check the
            // event modifiers directly. An active suggestion list consumes
            // plain Enter first when it has a selection.
            let send_key = has_focus
                && ui.input_mut(|input| {
                    let mut sent = false;
                    input.events.retain(|event| {
                        if sent {
                            return true;
                        }
                        let is_send = matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                modifiers,
                                ..
                            } if !modifiers.shift && !modifiers.alt
                                && (if enter_sends { !modifiers.command && !modifiers.ctrl } else { modifiers.command })
                        );
                        sent |= is_send;
                        !is_send
                    });
                    sent
                });
            let mut send_click = false;
            let line_height = ui
                .painter()
                .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
                .size()
                .y;
            // Every control sits in a band as tall as a one-line field at the
            // bottom of the row, centred on it. The field grows to six lines
            // above that band, so the buttons stay beside its last line.
            // A one-line field, like the composer before the rounded field:
            // the text line with padding, the send button as tall as that.
            let line = (line_height + COMPOSER_PADDING)
                .round()
                .max(COMPOSER_CONTROL);
            let button_width = line;
            let field_margin = ((line - line_height) / 2.0).round().max(0.0);
            // Measure this frame's draft at the text column's width, so the
            // field and the panel holding it grow on the keystroke that wraps
            // a line rather than a frame later, which made them jump.
            let wrap_id = id.with("wrap");
            let text_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(wrap_id))
                .map(|wrap| {
                    let format =
                        egui::TextFormat::simple(theme::regular(BODY_SIZE), palette.text);
                    crate::bidi::layout_editor(ui, &app.composer, &format, wrap, true)
                        .0
                        .size()
                        .y
                })
                .or_else(|| ui.ctx().read_response(id).map(|previous| previous.rect.height()))
                .unwrap_or(line_height)
                .clamp(line_height, line_height * 6.0);
            let row_height = (text_height + 2.0 * field_margin).max(line);
            let pill = widgets::raised(ui, &palette, composer_pill(&palette), |ui| {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), row_height),
                Layout::left_to_right(Align::Max),
                |ui| {
                // The plus sits in the field's rounded left end, centred as
                // the send button is in the right one.
                ui.add_space((line / 2.0 - PLUS_EDGE / 2.0).max(0.0));
                if app.editing.is_none() {
                    let tools = last_line(ui, line, |ui| theme::icon_button(
                        ui,
                        Icon::Plus,
                        22.0,
                        if app.composer_tools_open {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        &crate::i18n::gettext(app.locale, "Attach"),
                    ))
                    .tab_stop(Stop::Attach);
                    composer_tools_menu(app, chat, &tools);
                    // Plus and emoji sit close together, as a pair.
                    ui.add_space(COMPOSER_PAIR_GAP - ui.spacing().item_spacing.x);
                    let smile = last_line(ui, line, |ui| theme::icon_button(
                        ui,
                        Icon::Smile,
                        22.0,
                        if app.picker.is_some() {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                        palette.text,
                        tr("Emoji, GIFs, and stickers"),
                    )).tab_stop(Stop::Emoji);
                    app.picker_anchor = Some(smile.rect);
                    if smile.clicked() {
                        if app.composer_tools_open {
                            app.actions.push(Action::SetComposerTools(false));
                        }
                        app.actions.push(Action::TogglePicker(PickerTab::Emoji));
                    }
                    // The text follows the pair as closely as the emoji
                    // follows the plus (the field's own left margin included).
                    ui.add_space(COMPOSER_TEXT_GAP - ui.spacing().item_spacing.x - 8.0);
                }
                // The send button closes the row, flush with the field's end.
                let field_width =
                    (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(0.0);
                // The ink is centred on the controls, not the line box: the
                // span from a capital's top to a descender's bottom sits as far
                // from the field's top as from its bottom. Inter's box leaves
                // more room above the capitals than below the descenders, and
                // at fractional scales the glyphs round to the pixel grid off
                // centre in it. Measured at this scale, snapped to a pixel.
                let ink_middle = ink_middle(ui, line_height);
                let top = fastframe_text::snap_to_pixels(
                    row_height - line / 2.0 - (text_height - line_height) - ink_middle,
                    ui.ctx().pixels_per_point(),
                )
                .max(0.0);
                let bottom = (row_height - text_height - top).max(0.0);
                Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.set_width((field_width - 16.0).max(0.0));
                        ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(top);
                        // Grow from one to six lines, then scroll. The height
                        // is this frame's draft, measured above, rather than
                        // the scroll area's memory of the last frame.
                        egui::ScrollArea::vertical()
                            .id_salt("composer-scroll")
                            .max_height(text_height)
                            .min_scrolled_height(text_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                // Keep emoji in the buffer so character offsets match, then
                                // paint their color bitmaps over the transparent glyphs.
                                let mut clusters: Vec<(usize, usize, String)> = Vec::new();
                                let format = egui::TextFormat::simple(
                                    theme::regular(BODY_SIZE),
                                    palette.text,
                                );
                                let composer_rtl = crate::bidi::base_rtl(&app.composer);
                                let mut layouter = |ui: &egui::Ui,
                                                    text: &dyn egui::TextBuffer,
                                                    wrap: f32| {
                                    let (galley, found) = crate::bidi::layout_editor(
                                        ui,
                                        text.as_str(),
                                        &format,
                                        wrap,
                                        true,
                                    );
                                    clusters = found;
                                    galley
                                };
                                let wrap = ui.available_width();
                                ui.ctx().data_mut(|data| data.insert_temp(wrap_id, wrap));
                                let selection_before = egui::TextEdit::load_state(ui.ctx(), id)
                                    .and_then(|state| state.cursor.char_range());
                                // A middle click last frame put the caret where it
                                // was pressed; the field pastes as it does Ctrl+V.
                                if let Some(text) = app.primary_paste.take() {
                                    ui.input_mut(|input| {
                                        input.events.push(egui::Event::Paste(text));
                                    });
                                }
                                let output = egui::TextEdit::multiline(&mut app.composer)
                                    .id(id)
                                    .frame(Frame::NONE)
                                    .margin(Margin::ZERO)
                                    .hint_text(
                                        egui::RichText::new(if app.pending.is_empty() {
                                            crate::i18n::gettext(app.locale, "Type a message")
                                                .into_owned()
                                        } else {
                                            crate::i18n::gettext(app.locale, "Add a caption")
                                                .into_owned()
                                        })
                                        .color(palette.dim)
                                        .font(theme::regular(BODY_SIZE)),
                                    )
                                    .font(theme::regular(BODY_SIZE))
                                    .text_color(palette.text)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    .horizontal_align(if composer_rtl {
                                        Align::RIGHT
                                    } else {
                                        Align::LEFT
                                    })
                                    .return_key(if enter_sends {
                                        Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                                    } else {
                                        Some(KeyboardShortcut::new(Modifiers::NONE, Key::Enter))
                                    })
                                    .layouter(&mut layouter)
                                    .show(ui);
                                for (start, length, cluster) in &clusters {
                                    let Some(bounds) = crate::bidi::char_bounds(
                                        &output.galley,
                                        *start,
                                        start + length,
                                    ) else {
                                        continue;
                                    };
                                    // Skip emoji clusters split across rows.
                                    if bounds.height() > line_height * 1.5 {
                                        continue;
                                    }
                                    let rect = bounds.translate(output.galley_pos.to_vec2());
                                    crate::emoji::paint_cluster(ui, cluster, rect);
                                }
                                // Misspelled words get a wavy underline, except the one
                                // still being typed.
                                let speller = app
                                    .settings
                                    .spell_check
                                    .then(|| app.spelling.ready())
                                    .flatten();
                                if let Some(speller) = &speller {
                                    let caret = output
                                        .response
                                        .response
                                        .has_focus()
                                        .then(|| output.cursor_range.map(|range| range.primary.index.0))
                                        .flatten();
                                    for (start, end) in speller.misspelled(&app.composer) {
                                        if caret == Some(end) {
                                            continue;
                                        }
                                        paint_misspelling(
                                            ui,
                                            &output.galley,
                                            output.galley_pos,
                                            start,
                                            end,
                                            palette.danger,
                                        );
                                    }
                                }
                                // The row was sized from last frame's text. When a
                                // keystroke wraps or unwraps a line, lay the frame
                                // out again instead of showing the field a frame
                                // late, which made it jump while typing.
                                // egui sizes the box before applying the keystroke,
                                // and text typed into an empty field reaches its
                                // galley a frame later still, so measure the
                                // edited draft itself.
                                let painted = Rect::from_min_size(
                                    output.galley_pos,
                                    output.galley.size(),
                                );
                                let measured = crate::bidi::layout_editor(
                                    ui,
                                    &app.composer,
                                    &format,
                                    wrap,
                                    true,
                                )
                                .0
                                .size()
                                .y
                                .clamp(line_height, line_height * 6.0);
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(composer_text_id(), painted);
                                });
                                if (measured - text_height).abs() > 0.5 {
                                    ui.ctx().request_discard("composer height changed");
                                }
                                let response = output.response.response.clone().tab_stop(Stop::Composer);
                                primary_selection(app, ui, &output, &response, id);
                                ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(tr("Message")));
                                // egui moves the caret on secondary *press*, before it
                                // reports the completed click. Save the selection on press.
                                let secondary_press = ui.input(|input| {
                                    input.pointer.button_pressed(egui::PointerButton::Secondary)
                                        && input.pointer.latest_pos().is_some_and(|pointer| {
                                            response.rect.contains(pointer)
                                        })
                                });
                                if secondary_press {
                                    app.composer_menu_selection =
                                        selection_before.filter(|range| !range.is_empty());
                                } else if response.secondary_clicked()
                                    && app.composer_menu_selection.is_none()
                                {
                                    // Some platforms deliver press and release together.
                                    app.composer_menu_selection =
                                        selection_before.filter(|range| !range.is_empty());
                                }
                                // The menu offers corrections for the misspelled word it
                                // opened on.
                                if secondary_press
                                    || (response.secondary_clicked() && app.spell_menu.is_none())
                                {
                                    let pointer = ui.input(|input| input.pointer.latest_pos());
                                    app.spell_menu = speller.as_ref().zip(pointer).and_then(
                                        |(speller, pointer)| {
                                            let at = output
                                                .galley
                                                .cursor_from_pos(pointer - output.galley_pos)
                                                .index
                                                .0;
                                            let (start, end) = speller
                                                .misspelled(&app.composer)
                                                .into_iter()
                                                .find(|&(start, end)| start <= at && at <= end)?;
                                            let word: String = app
                                                .composer
                                                .chars()
                                                .skip(start)
                                                .take(end - start)
                                                .collect();
                                            let suggestions = speller.suggest(&word);
                                            Some((start, end, word, suggestions))
                                        },
                                    );
                                }
                                if (secondary_press || response.secondary_clicked())
                                    && app.composer_menu_selection.is_some()
                                {
                                    let mut state = output.state.clone();
                                    state.cursor.set_char_range(app.composer_menu_selection);
                                    state.store(ui.ctx(), id);
                                }
                                let selected = app.composer_menu_selection.is_some();
                                let cut = crate::i18n::gettext(app.locale, "Cut");
                                let copy = crate::i18n::gettext(app.locale, "Copy");
                                let paste = crate::i18n::gettext(app.locale, "Paste");
                                let select_all = crate::i18n::gettext(app.locale, "Select all");
                                let learn = crate::i18n::gettext(app.locale, "Add to dictionary");
                                let ignore = crate::i18n::gettext(app.locale, "Ignore");
                                let none = crate::i18n::gettext(app.locale, "No suggestions");
                                let spell_menu = app.spell_menu.clone();
                                let mut labels: Vec<&str> = vec![&cut, &copy, &paste, &select_all];
                                if let Some((_, _, _, suggestions)) = &spell_menu {
                                    labels.extend([learn.as_ref(), ignore.as_ref(), none.as_ref()]);
                                    labels.extend(suggestions.iter().map(String::as_str));
                                }
                                // "Add to dictionary" and "Ignore" carry icons.
                                let menu_width =
                                    widgets::menu_width(ui, &labels, spell_menu.is_some());
                                egui::Popup::context_menu(&response)
                                    .width(menu_width)
                                    .frame(widgets::menu_frame(&palette))
                                    .show(|ui| {
                                        if let Some((start, end, word, suggestions)) = &spell_menu {
                                            for suggestion in suggestions {
                                                if widgets::menu_item(ui, &palette, None, suggestion) {
                                                    app.actions.push(Action::ReplaceComposerWord {
                                                        start: *start,
                                                        end: *end,
                                                        with: suggestion.clone(),
                                                    });
                                                }
                                            }
                                            if suggestions.is_empty() {
                                                widgets::menu_item_enabled(ui, &palette, None, &none, false);
                                            }
                                            widgets::menu_separator(ui, &palette);
                                            if widgets::menu_item(ui, &palette, Some(Icon::Plus), &learn) {
                                                app.actions.push(Action::LearnWord(word.clone()));
                                            }
                                            if widgets::menu_item(ui, &palette, Some(Icon::EyeOff), &ignore) {
                                                app.actions.push(Action::IgnoreWord(word.clone()));
                                            }
                                            widgets::menu_separator(ui, &palette);
                                        }
                                        for (label, command, enabled) in [
                                            (&cut, ComposerTextCommand::Cut, selected),
                                            (&copy, ComposerTextCommand::Copy, selected),
                                            (&paste, ComposerTextCommand::Paste, true),
                                        ] {
                                            if widgets::menu_item_enabled(
                                                ui, &palette, None, label, enabled,
                                            ) {
                                                app.actions.push(Action::ComposerTextCommand(command));
                                            }
                                        }
                                        widgets::menu_separator(ui, &palette);
                                        if widgets::menu_item_enabled(
                                            ui, &palette, None, &select_all, !app.composer.is_empty(),
                                        ) {
                                            app.actions.push(Action::ComposerTextCommand(
                                                ComposerTextCommand::SelectAll,
                                            ));
                                        }
                                    });
                                if response.changed() {
                                    app.actions.push(Action::Composing {
                                        chat: chat.id.clone(),
                                        composing: true,
                                    });
                                    let cursor = output
                                        .cursor_range
                                        .map(|range| range.primary.index.0)
                                        .unwrap_or_else(|| app.composer.chars().count());
                                    if typed_colon
                                        && app.settings.emoji_shortcuts
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            ':',
                                        )
                                    {
                                        app.picker = None;
                                        app.emoji_start = Some(at);
                                        app.emoji_selected = 0;
                                        app.mention_start = None;
                                    } else if typed_at
                                        && chat.is_group()
                                        && !chat.participants.is_empty()
                                        && let Some(at) = standalone_trigger(
                                            &app.composer,
                                            cursor,
                                            '@',
                                        )
                                    {
                                        app.emoji_start = None;
                                        app.mention_start = Some(at);
                                        app.mention_selected = 0;
                                    } else if app.emoji_start.is_some() {
                                        app.emoji_selected = 0;
                                    }
                                }
                                let cursor = output
                                    .cursor_range
                                    .map(|range| range.primary.index.0)
                                    .unwrap_or_else(|| app.composer.chars().count());
                                if app.emoji_start.is_some()
                                    && active_emoji(&app.composer, app.emoji_start, cursor).is_none()
                                {
                                    app.emoji_start = None;
                                }
                                if app.mention_start.is_some()
                                    && active_mention(&app.composer, app.mention_start, cursor)
                                        .is_none()
                                {
                                    app.mention_start = None;
                                }
                                // The composer waits for the image preview to
                                // close before taking focus back.
                                if app.focus_composer && app.image_preview.is_none() {
                                    app.focus_composer = false;
                                    response.request_focus();
                                }
                            });
                        ui.add_space(bottom);
                        });
                    });
                let ready = !app.composer.trim().is_empty()
                    || !app.pending.is_empty()
                    || (unsent_voice.is_some() && app.editing.is_none());
                let (fill, hover, icon) = if ready {
                    (palette.accent, palette.accent_hover, palette.on_accent)
                } else {
                    (palette.surface, palette.surface_hover, palette.dim)
                };
                last_line(ui, line, |ui| if !ready && app.editing.is_none() {
                    // An empty composer changes the send button to record.
                    if theme::circle_button(
                        ui,
                        Icon::Mic,
                        button_width,
                        fill,
                        hover,
                        palette.secondary,
                        tr("Record a voice message"),
                    )
                    .tab_stop(Stop::Send)
                    .clicked()
                    {
                        app.actions.push(Action::StartRecording);
                    }
                } else {
                    let icon_kind = if app.editing.is_some() {
                        Icon::Check
                    } else {
                        Icon::Send
                    };
                    if theme::circle_button(ui, icon_kind, button_width, fill, hover, icon, tr("Send"))
                        .tab_stop(Stop::Send)
                        .clicked()
                    {
                        send_click = true;
                    }
                });
            },
                    );
                });
            // While typing, a border a little darker than the field keeps it
            // discreet; Tab still rings it in the accent.
            theme::quiet_focus_outline(
                ui,
                id,
                pill.response.rect,
                f32::from(COMPOSER_RADIUS),
                palette.surface.lerp_to_gamma(Color32::BLACK, 0.3),
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(composer_pill_id(), pill.response.rect));
            // Read after the field took this frame's keys.
            let link = link_previews
                .then(|| markup::first_web_link(&app.composer))
                .flatten();
            let asked = app
                .composer_link
                .as_ref()
                .filter(|asked| asked.chat == chat.id)
                .map(|asked| asked.link.as_str());
            if link.as_deref() != asked {
                app.actions.push(Action::ComposerLink {
                    chat: chat.id.clone(),
                    link,
                });
            }
            if (send_key || send_click)
                && (!app.composer.trim().is_empty() || !app.pending.is_empty())
            {
                let text = std::mem::take(&mut app.composer);
                if app.pending.is_empty() {
                    app.actions.push(Action::SendText {
                        chat: chat.id.clone(),
                        text,
                        quoting: app.reply_to.clone(),
                    });
                } else {
                    app.actions.push(Action::SendPending {
                        chat: chat.id.clone(),
                        caption: text,
                    });
                }
                app.focus_composer = true;
            } else if (send_key || send_click) && unsent_voice.is_some() && app.editing.is_none() {
                // An empty composer with a refused voice message: Send tries
                // that message again.
                app.actions.push(Action::SendRecording);
                app.focus_composer = true;
            }
            if app.settings.show_shortcut_hints {
                let hint_text = if enter_sends {
                    crate::i18n::gettext(app.locale, "Enter sends · Shift+Enter for a new line · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                } else {
                    crate::i18n::gettext(app.locale, "Ctrl+Enter sends · *bold* _italic_ ~strike~ · Ctrl+V pastes a picture")
                };
                let hint = super::keys::label(hint_text.as_ref());
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        13.0,
                        palette.dim,
                        palette.secondary,
                        crate::i18n::gettext(app.locale, "Hide shortcut hints (bring them back from Keyboard shortcuts)").as_ref(),
                    )
                    .clicked()
                    {
                        app.actions.push(Action::SetShortcutHints(false));
                    }
                    theme::text(ui, &hint, theme::regular(11.0), palette.dim);
                    // Open the shortcut list without consuming typed `?`.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::Keyboard,
                            13.0,
                            palette.dim,
                            palette.secondary,
                            &crate::i18n::gettext(app.locale, "All shortcuts ({})")
                                .replace("{}", &super::keys::label("Ctrl+/")),
                        )
                        .clicked()
                        {
                            app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                        }
                    });
                });
            }
        });
    // A bottom panel is placed from last frame's height. When the composer
    // grows or shrinks, lay the frame out again so it never shows the field
    // hanging below the window or a gap above it for a frame.
    let height = shown.response.rect.height();
    let height_id = egui::Id::new("composer-panel-height");
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<f32>(height_id);
        data.insert_temp(height_id, height);
        previous
    });
    if previous.is_some_and(|previous| (previous - height).abs() > 0.5) {
        ui.ctx().request_discard("composer height changed");
    }
    // Toasts sit above the composer so they never cover its buttons.
    ui.ctx()
        .data_mut(|data| data.insert_temp(super::composer_rect_id(), shown.response.rect));
}

/// Corner radius of the composer's rounded field.
const COMPOSER_RADIUS: u8 = 24;
/// Padding around the text of a one-line composer row. The row, and the send
/// and record button, are one text line plus this; the controls are centred
/// on it.
const COMPOSER_PADDING: f32 = 14.0;
/// Height of the plus and emoji buttons: a row is never shorter, or they
/// would stretch it and pull the text off its centre.
const COMPOSER_CONTROL: f32 = 36.0;
/// Space between the composer's rounded field and the buttons at its ends.
const COMPOSER_INSET: i8 = 2;
/// Space between the plus and emoji buttons' hit areas. Each area pads its
/// 22-point icon by six points a side, so this leaves eight between the
/// icons, a pair.
const COMPOSER_PAIR_GAP: f32 = -4.0;
/// From the emoji button's edge to the text. The plus's glyph leaves more of
/// its box empty than the round emoji's, so this is wider than the pair's
/// gap: at it, the text starts as far from the emoji's ink as the emoji
/// starts from the plus's.
pub(crate) const COMPOSER_TEXT_GAP: f32 = 6.0;
/// Width of the plus button: its 22-point icon and `icon_button`'s padding.
const PLUS_EDGE: f32 = 34.0;

/// Where the composer's text was painted in the frame's last pass, for
/// layout tests.
pub(crate) fn composer_text_id() -> egui::Id {
    egui::Id::new("composer-text-rect")
}

/// Where the composer's rounded field was drawn, for layout tests.
pub(crate) fn composer_pill_id() -> egui::Id {
    egui::Id::new("composer-pill")
}

/// The open chat's message list scroll offset and viewport height, as
/// `(offset.y, height)`, as of its last frame: for tests that need them
/// precisely. `App::at_bottom` covers ordinary UI checks.
pub(crate) fn scroll_metrics_id(chat: &ChatId) -> egui::Id {
    egui::Id::new(("message-scroll-metrics", chat))
}

/// How far below the top of a composer text row the middle of its ink sits,
/// at this scale: halfway from a capital's top to a descender's bottom.
fn ink_middle(ui: &egui::Ui, line_height: f32) -> f32 {
    let format = egui::TextFormat::simple(theme::regular(BODY_SIZE), Color32::WHITE);
    let (galley, _) = crate::bidi::layout_editor(ui, "Hy", &format, f32::INFINITY, true);
    galley
        .rows
        .first()
        .and_then(|row| {
            let [capital, descender] = [row.glyphs.first()?, row.glyphs.get(1)?];
            let top = capital.pos.y + capital.uv_rect.offset.y;
            let bottom = descender.pos.y + descender.uv_rect.offset.y + descender.uv_rect.size.y;
            (!capital.uv_rect.is_nothing() && !descender.uv_rect.is_nothing())
                .then(|| row.pos.y + (top + bottom) / 2.0)
        })
        .unwrap_or(line_height / 2.0)
}

/// Lays out a control centred in the composer's last-line band, however
/// tall the draft has grown, so every control shares one vertical centre.
fn last_line<R>(ui: &mut egui::Ui, line: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(0.0, line), Layout::left_to_right(Align::Center), add)
        .inner
}

/// The rounded field that holds the composer's controls, or the recorder.
fn composer_pill(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(COMPOSER_RADIUS))
        // The end buttons are inset by as much at the sides as above and
        // below, so they sit evenly in the rounded ends.
        .inner_margin(Margin {
            left: COMPOSER_INSET,
            right: COMPOSER_INSET,
            top: COMPOSER_INSET,
            bottom: COMPOSER_INSET,
        })
}

/// The plus menu beside the composer: send files or create a poll.
fn composer_tools_menu(app: &mut App, chat: &Chat, plus: &egui::Response) {
    let id = plus.id.with("composer-tools");
    let mut open = app.composer_tools_open;
    let picker_open = app.picker.is_some();
    if plus.clicked() {
        open = !open;
        if open && picker_open {
            app.actions.push(Action::ClosePicker);
        }
    }
    let mut draw_open = open;
    if draw_open && !picker_open {
        egui::Popup::menu(plus)
            .id(id)
            .open_bool(&mut draw_open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .width(190.0)
            .frame(widgets::menu_frame(&app.palette))
            .show(|ui| {
                let send_files = crate::i18n::gettext(app.locale, "Send files");
                if widgets::menu_item(ui, &app.palette, Some(Icon::Paperclip), &send_files) {
                    app.actions.push(Action::Attach);
                }
                let create_poll = crate::i18n::gettext(app.locale, "Create poll");
                if widgets::menu_item(ui, &app.palette, Some(Icon::ListChecks), &create_poll) {
                    app.actions
                        .push(Action::ShowDialog(Dialog::CreatePoll(chat.id.clone())));
                }
            });
    }
    if draw_open != app.composer_tools_open {
        app.actions.push(Action::SetComposerTools(draw_open));
    }
}

/// Offers a refused voice message for another try, or to discard it.
/// From a strip above the composer (reply, edit, unsent voice) to the field
/// it belongs to: close enough to read as one piece.
pub(crate) const STRIP_GAP: f32 = 3.0;

/// Leaves [`STRIP_GAP`] below a strip, the layout's own spacing included.
fn strip_gap(ui: &mut egui::Ui) {
    ui.add_space(STRIP_GAP - ui.spacing().item_spacing.y);
}

/// Corner radius of the strips above the composer and of its suggestion
/// lists: rounder than a bubble, closer to the composer's own ends.
const STRIP_RADIUS: u8 = 16;

/// The frame of a strip above the composer (reply, edit, unsent voice).
/// Like the composer it takes the incoming bubble's colour, which in the
/// light theme is white on the chat rather than the grey interface surface.
/// Its content starts 14 points in, so a reply's accent bar lines up with
/// the composer's plus below it.
fn strip_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 6,
            bottom: 6,
        })
}

/// The emoji and @mention suggestion lists, raised above the composer like
/// the strips, with the same fill and corners.
fn suggestion_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin::same(SUGGESTION_INSET))
}

/// Space between a suggestion list's edge and its rows.
const SUGGESTION_INSET: i8 = 4;
/// A suggestion row's corners, concentric with the list's.
const SUGGESTION_ROW_RADIUS: f32 = (STRIP_RADIUS as i8 - SUGGESTION_INSET) as f32;

fn unsent_voice_strip(app: &mut App, ui: &mut egui::Ui, samples: usize) {
    let palette = app.palette;
    let seconds = (samples as f64 / f64::from(crate::voice::RATE))
        .round()
        .max(1.0) as u32;
    let label = crate::i18n::gettext(app.locale, "Voice message ({duration}) not sent")
        .replace("{duration}", &crate::util::duration(seconds));
    let discard = crate::i18n::gettext(app.locale, "Discard voice message");
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Mic, 16.0, palette.danger);
            theme::text(ui, &label, theme::semibold(12.5), palette.danger);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    discard.as_ref(),
                );
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("unsent-voice-discard"), button.rect)
                });
                if button.clicked() {
                    app.actions.push(Action::DiscardUnsentVoice);
                }
            });
        });
    });
    strip_gap(ui);
}

fn edit_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Pencil, 16.0, palette.accent);
            theme::text(
                ui,
                tr("Editing message"),
                theme::semibold(12.5),
                palette.accent,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    tr("Stop editing (Esc)"),
                )
                .clicked()
                {
                    app.actions.push(Action::CancelEdit);
                }
            });
        });
    });
    strip_gap(ui);
}

fn reply_strip(app: &mut App, ui: &mut egui::Ui, quoted: &Message) {
    let palette = app.palette;
    let who = if quoted.from_me {
        tr("You").to_owned()
    } else {
        app.display_name_or(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary(), &app.mention_list(quoted));
    let picture = quote_picture(quoted);
    let strip = widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width().max(0.0));
        ui.horizontal(|ui| {
            let (bar, _) = ui.allocate_exact_size(vec2(3.0, 34.0), Sense::hover());
            ui.painter().rect_filled(bar, 2.0, palette.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let reserved = if picture.is_some() { 88.0 } else { 40.0 };
                ui.set_max_width((ui.available_width() - reserved).max(0.0));
                widgets::rich_text(
                    ui,
                    &tr("Replying to {who}").replace("{who}", &who),
                    theme::semibold(12.5),
                    palette.accent,
                );
                widgets::rich_text(ui, &summary, theme::regular(12.5), palette.secondary);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    tr("Cancel reply (Esc)"),
                )
                .clicked()
                {
                    app.actions.push(Action::CancelReply);
                }
                if let Some(picture) = &picture {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    paint_quote_picture(
                        ui,
                        picture,
                        rect,
                        CornerRadius::same(4),
                        palette.surface_hover,
                    );
                }
            });
        });
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(reply_strip_id(), strip.response.rect));
    strip_gap(ui);
}

/// Where the reply strip was drawn, for layout tests.
pub(crate) fn reply_strip_id() -> egui::Id {
    egui::Id::new("reply-strip")
}

/// The preview of the composer's first link, with a button that sends the
/// text without it. Nothing shows once the page turned out to say nothing.
fn link_strip(
    ui: &mut egui::Ui,
    palette: &Palette,
    link: &crate::model::ComposerLink,
    actions: &mut Vec<Action>,
) {
    let card = match &link.card {
        Some(None) => return,
        Some(Some(card)) => Some(card),
        None => None,
    };
    widgets::raised(
        ui,
        palette,
        Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(10, 6)),
        |ui| {
            ui.set_width(ui.available_width().max(0.0));
            ui.horizontal(|ui| {
                match card {
                    None => {
                        theme::spinner(ui, 16.0, palette.accent);
                        theme::text(
                            ui,
                            tr("Loading link preview…"),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    }
                    Some(card) => {
                        if let Some(bytes) = card.thumbnail.as_deref() {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                            let mut hasher = std::hash::DefaultHasher::new();
                            std::hash::Hash::hash(&card.link, &mut hasher);
                            let key = format!("link-{:x}", std::hash::Hasher::finish(&hasher));
                            let uri = thumbnail_uri(ui.ctx(), &link.chat, &key, bytes);
                            egui::Image::new(uri)
                                .fit_to_exact_size(rect.size())
                                .corner_radius(4.0)
                                .paint_at(ui, rect);
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.set_max_width((ui.available_width() - 40.0).max(0.0));
                            let rows = [
                                (
                                    card.title.as_deref(),
                                    theme::semibold(12.5),
                                    palette.text,
                                    1,
                                ),
                                (
                                    card.description.as_deref(),
                                    theme::regular(12.5),
                                    palette.secondary,
                                    2,
                                ),
                            ];
                            for (text, font, color, lines) in rows {
                                if let Some(text) = text {
                                    let line = widgets::line(
                                        ui,
                                        text,
                                        font,
                                        color,
                                        ui.available_width(),
                                        lines,
                                    );
                                    let (rect, _) =
                                        ui.allocate_exact_size(line.size(), Sense::hover());
                                    line.paint(ui, rect.min, color);
                                }
                            }
                            let url = crate::safety::preview_url(&card.link).unwrap_or_default();
                            theme::text(ui, domain_of(&url), theme::regular(12.0), palette.dim);
                        });
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        16.0,
                        palette.secondary,
                        palette.text,
                        tr("Remove link preview"),
                    )
                    .clicked()
                    {
                        actions.push(Action::DismissLinkPreview);
                    }
                });
            });
        },
    );
    strip_gap(ui);
}

/// App data needed while drawing a checked-out conversation.
struct View<'a> {
    palette: Palette,
    locale: crate::i18n::Locale,
    chat: &'a Chat,
    me: Option<&'a str>,
    download_settings: &'a crate::settings::Settings,
    connected: bool,
    poll_voting: &'a HashSet<(ChatId, String)>,
    interactive_pending: &'a HashSet<(ChatId, String)>,
    /// Own voice messages and attachments being prepared, uploaded and sent.
    media_sending: &'a HashSet<(ChatId, String)>,
    /// Share of each sending attachment uploaded so far, in percent.
    media_progress: &'a HashMap<(ChatId, String), u8>,
    /// Pictures of the loaded messages that others quote, by message id.
    quote_pictures: &'a HashMap<String, QuotePicture>,
    anchor: Option<&'a str>,
    /// The anchor returns to the top of the view, not to its middle.
    anchor_at_top: bool,
    /// Demo/test: keep this message's context menu open.
    open_menu: Option<&'a str>,
    reaction: Option<&'a str>,
    /// The reaction picker was opened from the message's context menu.
    reaction_menu: bool,
    reaction_emoji: &'a [(String, u32)],
    keyboard_navigation: &'a std::cell::Cell<bool>,
    /// Resolves a name with the message's stored name as fallback.
    names_or: &'a dyn Fn(&str, Option<&str>) -> String,
    /// Resolves mention names without replacing our name with "You".
    mention_names: &'a dyn Fn(&str) -> String,
    avatars: &'a HashMap<String, Option<PathBuf>>,
    contacts: &'a HashMap<String, crate::model::Contact>,
    now: i64,
    /// Animate media only while this window is active.
    animate: bool,
    player: &'a crate::audio::Player,
    transcriber: &'a crate::transcribe::Transcriber,
    transcripts: &'a HashMap<crate::transcribe::Key, crate::transcribe::Transcript>,
    transcripts_folded: &'a HashSet<crate::transcribe::Key>,
    video: &'a crate::video::Player,
    /// The video shown expanded over the window, whose message keeps its
    /// poster meanwhile.
    video_expanded: Option<&'a str>,
    copy_rows: &'a std::sync::Mutex<Vec<crate::transcript::Row>>,
    /// The messages selected in this chat, in chat order.
    selection: &'a [String],
    /// Whether every selected message can be deleted for everyone.
    selection_revocable: bool,
}

/// How far a message bubble may extend across the transcript.
fn bubble_width_limit(available: f32, own: bool, carousel: bool, with_avatar: bool) -> f32 {
    let width = if carousel {
        (available * 0.95).min(920.0)
    } else if own {
        (available * 0.72).min(560.0)
    } else {
        // Leave room for the frame's 20-point horizontal padding and an
        // eight-point gap from the outgoing bubbles' right edge.
        available - 28.0
    };
    (width
        - if with_avatar {
            SENDER_AVATAR + 8.0
        } else {
            0.0
        })
    .max(0.0)
}

/// A row height to assume for a message that has not been laid out yet. Rows
/// near the viewport are always measured, and a change in the height of a row
/// above the viewport moves the scroll offset with it, so this only shapes the
/// scrollbar until the reader scrolls near the row.
fn estimated_height(message: &Message, width: f32, new_day: bool, sender_pictures: bool) -> f32 {
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    let bubble = (bubble_width_limit(
        width,
        message.from_me,
        carousel,
        !message.from_me && sender_pictures,
    ) - 20.0)
        .max(40.0);
    let text_rows = |text: &str| {
        let per_row = bubble / 7.5;
        (text.chars().count() as f32 / per_row).ceil().max(1.0)
    };
    let caption_rows = |caption: &Option<String>| {
        caption
            .as_deref()
            .map_or(0.0, |caption| text_rows(caption) * 19.0)
    };
    let body = match &message.content {
        Content::Text { text, preview } => {
            text_rows(text) * 19.0 + if preview.is_some() { 60.0 } else { 0.0 }
        }
        Content::Interactive { text, .. } => text_rows(text) * 19.0 + 60.0,
        Content::Image { caption, .. } | Content::Video { caption, .. } => {
            200.0 + caption_rows(caption)
        }
        Content::Document { caption, .. } => 70.0 + caption_rows(caption),
        Content::Sticker { .. } => 140.0,
        Content::Audio { .. } => 60.0,
        _ => 40.0,
    };
    // Bubble padding, the sender line, and the row spacing, plus the date
    // chip above the first message of a day.
    40.0 + body + if new_day { 36.0 } else { 0.0 }
}

/// Incoming messages carry their sender's picture and name in groups only,
/// as on WhatsApp.
fn shows_sender_pictures(chat: &Chat) -> bool {
    chat.is_group()
}

fn messages(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    // Taken up front: `names_or` below borrows the rest of `app` for the
    // whole function, so a pending scroll must come out before that.
    let pending_scroll = app.scroll_page.take();
    // A scroll gesture that began over the messages stays with them when
    // the pointer drifts off (#274), taken here for the same reason.
    let carried = app.scroll_route.take(crate::app::ScrollPane::Messages);
    // An explicit jump (Ctrl+End, or the return-to-bottom button) must reach
    // the bottom even while a message bubble retains keyboard focus.
    let scroll_forced = std::mem::take(&mut app.scroll_to_bottom_forced);
    // Check out the conversation while drawing rows and collecting actions.
    let mut conversation = app.conversations.remove(&chat.id).unwrap_or_default();
    let typing = app.typing_in(&chat.id);
    let mut avatars = HashMap::new();
    if shows_sender_pictures(chat) {
        let mut senders: HashSet<String> = conversation
            .messages
            .iter()
            .filter(|message| !message.from_me)
            .map(|message| message.sender.clone())
            .collect();
        // A typing participant may not have a visible message.
        senders.extend(typing.iter().map(|(id, _)| id.clone()));
        for sender in senders {
            let picture = app.avatar(&sender);
            avatars.insert(sender, picture);
        }
    }
    let quote_pictures = quote_pictures(&conversation.messages);
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let keyboard_navigation = std::cell::Cell::new(false);
    let selected: Vec<String> = app
        .selection
        .as_ref()
        .filter(|(selected_chat, _)| *selected_chat == chat.id)
        .map(|(_, ids)| ids.clone())
        .unwrap_or_default();
    let now = crate::util::now();
    let selection_revocable = all_revocable(&conversation, &selected, now);
    let view = View {
        palette,
        locale: app.locale,
        chat,
        me: app.me.as_deref(),
        download_settings: &app.settings,
        connected: app.link.is_connected(),
        poll_voting: &app.poll_voting,
        interactive_pending: &app.interactive_sending,
        media_sending: &app.media_sending,
        media_progress: &app.media_progress,
        quote_pictures: &quote_pictures,
        anchor: if conversation.loading_older || conversation.fetching_phone {
            None
        } else {
            app.scroll_anchor.as_deref()
        },
        anchor_at_top: app.scroll_anchor_at_top,
        open_menu: app.open_message_menu.as_deref(),
        reaction: app
            .reaction_target
            .as_ref()
            .filter(|(id, _)| id == &chat.id)
            .map(|(_, message)| message.as_str()),
        reaction_menu: app.reaction_beside_menu,
        reaction_emoji: &app.settings.reaction_emoji,
        keyboard_navigation: &keyboard_navigation,
        names_or: &names_or,
        mention_names: &mention_names,
        avatars: &avatars,
        contacts: &app.contacts,
        now,
        animate: app.window_focused,
        player: &app.player,
        transcriber: &app.transcriber,
        transcripts: &app.transcripts,
        transcripts_folded: &app.transcripts_folded,
        video: &app.video,
        video_expanded: app.video_expanded.as_deref(),
        copy_rows: app.copy_rows.as_ref(),
        selection: &selected,
        selection_revocable,
    };
    let mut actions = Vec::new();
    let mut anchored = false;
    // The first unread message is the count-th incoming one from the end.
    let divider = app
        .unread_divider
        .as_ref()
        .filter(|divider| divider.chat == chat.id)
        .and_then(|divider| {
            conversation
                .messages
                .iter()
                .rev()
                .filter(|message| !message.from_me)
                .nth(divider.count.saturating_sub(1) as usize)
                .map(|message| (message.id.clone(), divider.count, divider.placed))
        });
    let mut divider_placed = false;
    let selection: Option<Vec<String>> = app
        .selection
        .as_ref()
        .filter(|(selected_chat, _)| *selected_chat == chat.id)
        .map(|(_, ids)| ids.clone());
    let scroll_to_bottom =
        app.scroll_to_bottom && divider.as_ref().is_none_or(|(.., placed)| *placed);
    // The message a quote or search result jumped to flashes once in view.
    let jump = app
        .jump_highlight
        .clone()
        .filter(|jump| jump.chat == chat.id);
    let jump_since = std::cell::Cell::new(jump.as_ref().and_then(|jump| jump.since));
    let time = ui.input(|input| input.time);
    // Do not animate programmatic scrolling. Pending animations can delay a
    // later request to reach the end. Keyboard page, Home, and End scrolls
    // ease by applying small instant steps each frame instead (`KeyScroll`).
    let mut edge_scrolled_up = false;
    // Rows far from the viewport are skipped rather than laid out, keeping the
    // height they last took (or an estimate), so a long history costs the rows
    // near the screen instead of every loaded one. A jump to a message, the
    // unread divider's first placement, and a text selection lay out every
    // row: a jump needs exact positions, and egui drops a selection whose
    // ends it does not see in a frame. Rows near the viewport are measured
    // again each frame, so a width change or an image that loads corrects
    // them before they come into view.
    let layout_width = ui.available_width();
    let lay_out_all = view.anchor.is_some()
        || divider.as_ref().is_some_and(|(.., placed)| !placed)
        || ui
            .ctx()
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection());
    let pass = ui.ctx().cumulative_pass_nr();
    let redo = ui.ctx().current_pass_index() > 0;
    let mut rows = std::mem::take(&mut conversation.rows);
    // Forget rows that left the conversation, such as deleted messages.
    if rows.len() > conversation.messages.len() * 2 + 64 {
        let ids: HashSet<&str> = conversation
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        rows.retain(|id, _| ids.contains(id.as_str()));
    }
    // How far rows entirely above the viewport grew this frame. The offset
    // follows, so what the reader looks at stays put while rows scrolled past
    // are measured for the first time.
    let mut grew_above = 0.0;
    // The offset the message list is about to load, the distance a Home
    // scroll still has to cover: the same id the `ScrollArea` below loads.
    // `.id_salt()` hashes its salt into an `IdSalt` before combining it with
    // the `Ui`'s id, so the salt must go through the same `IdSalt::new` here
    // to land on the same id.
    let scroll_id = ui.make_persistent_id(egui::IdSalt::new(("messages", &chat.id)));
    let offset =
        egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
    // A keyboard scroll under way stops for a jump to a message, for the
    // reaction bar, which holds the view still, and for a wheel or trackpad
    // turn over the message list, before it takes another step.
    let mut key_scroll = conversation.key_scroll.take();
    let list = ui.available_rect_before_wrap();
    let wheel_over_list = ui.input(|input| {
        (input.smooth_scroll_delta.y != 0.0
            || input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::MouseWheel { .. })))
            && input
                .pointer
                .hover_pos()
                .is_some_and(|pos| list.contains(pos))
    });
    // The reaction bar holds the view still.
    let carried = if view.reaction.is_some() {
        0.0
    } else {
        carried
    };
    if view.anchor.is_some() || view.reaction.is_some() || wheel_over_list || carried != 0.0 {
        key_scroll = None;
    }
    let key_duration = ui.style().scroll_animation.duration.max;
    // A key scroll starts one frame back, so it moves on its first frame and
    // a held key's repeats, which restart it, never stall it.
    let key_start = time - f64::from(ui.input(|input| input.stable_dt.min(0.1)));
    let mut pinned = false;
    // The newest day at the top of the view, for the floating date.
    let mut top_day: Option<i64> = None;
    let output = egui::ScrollArea::vertical()
        .id_salt(("messages", &chat.id))
        .auto_shrink([false, false])
        .stick_to_bottom(view.reaction.is_none())
        .scroll_source(if view.reaction.is_some() {
            egui::scroll_area::ScrollSource::NONE
        } else {
            Default::default()
        })
        .animated(false)
        .show(ui, |ui| {
            // Scroll while selecting near an edge. `scroll_with_delta` also
            // releases stick-to-bottom; setting the offset directly does not.
            let viewport = ui.clip_rect();
            *app.selection_view.lock().unwrap_or_else(|p| p.into_inner()) = Some(viewport);
            if carried != 0.0 {
                ui.scroll_with_delta_animation(
                    vec2(0.0, carried),
                    egui::style::ScrollAnimation::none(),
                );
            }
            // Keep ordinary conversation-space clicks useful: after reading,
            // the next keystroke should go straight to the composer. Register
            // this before the message controls so text, links, media, and
            // selection interactions remain in front of the background.
            let background = ui.interact(
                viewport,
                ui.id().with(("message-background", &chat.id)),
                Sense::click(),
            );
            // Only a drag that has moved past a click, such as selecting
            // text, scrolls; a click near an edge does not.
            let held_inside = ui.input(|input| {
                input.pointer.primary_down()
                    && input.pointer.is_decidedly_dragging()
                    && input.pointer.press_origin().is_some_and(|origin| {
                        viewport.contains(origin) && origin.x < viewport.right() - 16.0
                    })
            });
            if view.reaction.is_none()
                && held_inside
                && let Some(pointer) = ui.input(|input| input.pointer.latest_pos())
            {
                let delta = edge_scroll(pointer.y, viewport.top(), viewport.bottom());
                if delta != 0.0 {
                    if delta < 0.0 {
                        edge_scrolled_up = true;
                    }
                    key_scroll = None;
                    ui.scroll_with_delta_animation(
                        vec2(0.0, -delta),
                        egui::style::ScrollAnimation::none(),
                    );
                    ui.ctx().request_repaint();
                }
            }
            Frame::new()
                .inner_margin(Margin::symmetric(18, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    top_of_history(ui, &palette, &conversation, chat, &mut actions);
                    let mut previous: Option<&Message> = None;
                    // Rows within a few viewports of the screen are laid out
                    // and their height remembered, so scrolling finds them
                    // measured before they show.
                    let margin = (viewport.height() * 3.0).max(600.0);
                    for message in &conversation.messages {
                        let before = ui.cursor().top();
                        let new_day = previous.is_none_or(|previous| {
                            crate::util::day_key(previous.timestamp)
                                != crate::util::day_key(message.timestamp)
                        });
                        let known = rows.get(&message.id).copied();
                        let height = known.map_or_else(
                            || {
                                estimated_height(
                                    message,
                                    layout_width,
                                    new_day,
                                    shows_sender_pictures(view.chat),
                                )
                            },
                            |row| row.height,
                        );
                        // A pass redone after the offset followed rows that
                        // grew keeps to the rows it laid out before (and any
                        // now on screen): measuring rows the shift brought
                        // into range would move the view once more.
                        let reach = if redo
                            && known
                                .and_then(|row| row.pass)
                                .is_none_or(|last| last + 1 != pass)
                        {
                            0.0
                        } else {
                            margin
                        };
                        let near = before + height >= viewport.top() - reach
                            && before <= viewport.bottom() + reach;
                        if !lay_out_all && !near {
                            // The body rect of a row that just left the
                            // layout marks where it last was, not where it is.
                            if known
                                .is_some_and(|row| row.pass.is_some_and(|last| last + 1 == pass))
                            {
                                let id = bubble_id(&chat.id, &message.id).with("body");
                                ui.ctx().data_mut(|data| data.remove::<Rect>(id));
                            }
                            ui.add_space(height);
                            if known.is_none() {
                                rows.insert(message.id.clone(), RowHeight { height, pass: None });
                            }
                            previous = Some(message);
                            continue;
                        }
                        // A row laid out again after a skip still has the
                        // rect it last had on screen, where other rows are
                        // now. The bubble registers its click targets from
                        // it, so drop it rather than let it take their clicks.
                        // So does every row when older messages were just
                        // added above them, or a jump lays them all out: the
                        // bubble's response merges that rect with its new
                        // one, which would aim the scroll at the middle of
                        // everything in between.
                        if view.anchor.is_some()
                            || known.is_none_or(|row| row.pass.is_none_or(|last| last + 1 < pass))
                        {
                            let id = bubble_id(&chat.id, &message.id).with("rect");
                            ui.ctx().data_mut(|data| data.remove::<Rect>(id));
                        }
                        if new_day {
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                widgets::chip(
                                    ui,
                                    &palette,
                                    &crate::util::day_label(app.locale, message.timestamp),
                                );
                            });
                            ui.add_space(4.0);
                        }
                        if let Some((id, count, placed)) = &divider
                            && *id == message.id
                        {
                            ui.add_space(6.0);
                            let label = crate::i18n::ngettext(
                                app.locale,
                                "{} unread message",
                                "{} unread messages",
                                *count,
                            )
                            .replace("{}", &count.to_string());
                            let response = ui
                                .vertical_centered(|ui| widgets::chip(ui, &palette, &label))
                                .inner;
                            if !placed && view.anchor.is_none() {
                                response.scroll_to_me(Some(Align::Min));
                                divider_placed = true;
                            }
                            ui.add_space(4.0);
                        }
                        let show_sender = shows_sender_pictures(chat)
                            && !message.from_me
                            && (new_day
                                || previous.is_none_or(|previous| {
                                    previous.sender != message.sender || previous.from_me
                                }));
                        // The first message of a run from one side, as the
                        // phone draws it: a little apart, with a tail.
                        let first_in_run = new_day
                            || previous.is_none_or(|previous| {
                                previous.from_me != message.from_me
                                    || (!message.from_me && previous.sender != message.sender)
                            });
                        if first_in_run && !new_day && previous.is_some() {
                            ui.add_space(RUN_GAP);
                        }
                        let flash = jump
                            .as_ref()
                            .filter(|jump| jump.message == message.id)
                            .map(|_| (ui.painter().add(egui::Shape::Noop), ui.cursor().top()));
                        // A press can start text selection, causing earlier virtualized
                        // rows to be laid out on the next frame. Keep this row's
                        // widget IDs stable so the release still clicks its link.
                        let response = ui
                            .scope_builder(
                                egui::UiBuilder::new()
                                    .id(bubble_id(&chat.id, &message.id).with("row-ui")),
                                |ui| {
                                    bubble(
                                        ui,
                                        &view,
                                        message,
                                        show_sender,
                                        first_in_run,
                                        &mut actions,
                                    )
                                },
                            )
                            .inner;
                        if let Some((slot, top)) = flash {
                            if view.anchor == Some(message.id.as_str()) && response.is_some() {
                                jump_since.set(Some(time));
                            }
                            let strength = jump_since
                                .get()
                                .map_or(0.0, |since| JumpHighlight::strength(time - since));
                            if strength > 0.0 {
                                // Behind the row, across the whole message
                                // view, like WhatsApp's.
                                let band = Rect::from_x_y_ranges(
                                    viewport.x_range(),
                                    top - 3.0..=ui.min_rect().bottom() + 3.0,
                                );
                                ui.painter().set(
                                    slot,
                                    egui::Shape::rect_filled(
                                        band,
                                        0.0,
                                        palette.accent.gamma_multiply(0.22 * strength),
                                    ),
                                );
                            }
                            if jump_since.get().is_some() {
                                ui.ctx().request_repaint();
                            }
                        }
                        if let (Some(selected), Some(response)) = (&selection, &response) {
                            if selected.contains(&message.id) {
                                ui.painter().rect(
                                    response.rect.expand(2.0),
                                    10.0,
                                    palette.accent.gamma_multiply(0.18),
                                    Stroke::new(2.0, palette.accent),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            // While selecting, a click anywhere on the row picks
                            // the message: its text, links and media, and the
                            // strip beside it, not only the bubble's padding
                            // (#241). Registered after the row, so it takes
                            // those clicks; a drag still selects text.
                            let row = Rect::from_x_y_ranges(
                                ui.max_rect().x_range(),
                                response.rect.y_range(),
                            );
                            let pick = ui.interact(
                                row,
                                bubble_id(&chat.id, &message.id).with("pick"),
                                Sense::CLICK,
                            );
                            if response.clicked() || pick.clicked() {
                                let shift = ui.input(|input| input.modifiers.shift);
                                actions.push(if shift {
                                    Action::SelectRange(message.id.clone())
                                } else {
                                    Action::ToggleSelected(message.id.clone())
                                });
                            }
                        } else if let Some(response) = &response
                            && response.clicked()
                            && ui.input(|input| input.modifiers.command)
                        {
                            // Ctrl-click (Command-click on macOS) starts a selection.
                            actions.push(Action::SelectMessage(message.id.clone()));
                        }
                        if let Some(response) = response
                            && view.anchor == Some(message.id.as_str())
                        {
                            response.scroll_to_me_animation(
                                Some(if view.anchor_at_top {
                                    Align::Min
                                } else {
                                    Align::Center
                                }),
                                egui::style::ScrollAnimation::none(),
                            );
                            anchored = true;
                        }
                        let measured = ui.cursor().top() - before;
                        if before + height <= viewport.top() {
                            grew_above += measured - height;
                        }
                        if top_day.is_none() && before + measured > viewport.top() {
                            top_day = Some(message.timestamp);
                        }
                        rows.insert(
                            message.id.clone(),
                            RowHeight {
                                height: measured,
                                pass: Some(pass),
                            },
                        );
                        previous = Some(message);
                    }
                    if !typing.is_empty() {
                        typing_bubble(ui, &view, &typing);
                    }
                    ui.add_space(4.0);
                    // An explicit jump (Ctrl+End, or the return-to-bottom
                    // button) wins over a message bubble's retained keyboard
                    // focus; other triggers still defer to it, so an
                    // automatic pin does not pull the view away from what a
                    // keyboard-navigating reader is looking at.
                    if scroll_to_bottom
                        && (scroll_forced || !keyboard_navigation.get())
                        && view.reaction.is_none()
                    {
                        ui.scroll_to_rect_animation(
                            Rect::from_min_size(ui.cursor().min, Vec2::ZERO),
                            None,
                            egui::style::ScrollAnimation::none(),
                        );
                        pinned = true;
                    }
                });
            if background.clicked() {
                actions.push(Action::FocusComposer);
            }
            // A keyboard PgUp/PgDn/Home/End scroll moves by the next slice of
            // its eased distance each frame, as an instant scroll: relative
            // steps compose with the row-height compensation below, and
            // `scroll_with_delta` releases stick-to-bottom like edge-scroll
            // above. An instant jump to the end this frame wins over it.
            if pinned {
                key_scroll = None;
            } else if let Some(kind) = pending_scroll
                && view.reaction.is_none()
            {
                // A repeated page key adds a page to the distance left, so a
                // held key keeps moving; anything else starts over.
                let page = match kind {
                    Scroll::PageUp => viewport.height() * 0.9,
                    Scroll::PageDown => -(viewport.height() * 0.9),
                    Scroll::LineUp => KEY_LINE_SCROLL,
                    Scroll::LineDown => -KEY_LINE_SCROLL,
                    Scroll::Top | Scroll::Bottom => 0.0,
                };
                let left = key_scroll
                    .filter(|scroll| scroll.kind == kind)
                    .map_or(0.0, |scroll| scroll.remaining);
                key_scroll = Some(KeyScroll::new(kind, left + page, key_start, key_duration));
            }
            if let Some(scroll) = &mut key_scroll {
                let fraction = scroll.advance(time);
                let step = match scroll.kind {
                    Scroll::PageUp | Scroll::PageDown | Scroll::LineUp | Scroll::LineDown => {
                        let step = scroll.remaining * fraction;
                        scroll.remaining -= step;
                        step
                    }
                    Scroll::Top => offset * fraction,
                    // Measured again every frame, so messages that arrive
                    // on the way are included.
                    Scroll::Bottom => {
                        -(ui.min_rect().bottom() - viewport.bottom()).max(0.0) * fraction
                    }
                };
                ui.scroll_with_delta_animation(
                    vec2(0.0, step),
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
        });
    app.scroll_route
        .place(crate::app::ScrollPane::Messages, output.inner_rect);
    let at_bottom =
        output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 24.0;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            scroll_metrics_id(&chat.id),
            (output.state.offset.y, output.inner_rect.height()),
        );
    });
    // Keep the view at the end while initial content expands, until the user
    // scrolls with the wheel, trackpad, or scrollbar.
    let bar = Rect::from_min_max(
        pos2(output.inner_rect.right() - 16.0, output.inner_rect.top()),
        output.inner_rect.right_bottom(),
    );
    let reader_scrolled = ui.input(|input| {
        input.smooth_scroll_delta.y != 0.0
            || input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::MouseWheel { .. }))
            || (input.pointer.primary_down()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| bar.contains(pos)))
    });
    floating_day(
        ui,
        app,
        &chat.id,
        output.inner_rect,
        top_day.filter(|_| !at_bottom),
        reader_scrolled || key_scroll.is_some(),
    );
    let complete = conversation.complete;
    let loading = conversation.loading_older;
    let fetching = conversation.fetching_phone;
    let exhausted = conversation.phone_exhausted;
    conversation.rows = rows;
    // Keep the rows on screen where they were when rows above them changed
    // height, unless this frame scrolled on purpose (to the end, a jump, or
    // the divider) or sticks to the end, where egui keeps the offset anyway.
    // Home targets an absolute offset (0), so it is skipped while Home
    // scrolls, rather than pushing the offset away from the top; PgUp/PgDn
    // steps are relative and still get it. The pass is redone at the new
    // offset, so the shift never shows.
    if grew_above.abs() >= 0.5
        && !lay_out_all
        && !scroll_to_bottom
        && !at_bottom
        && key_scroll.is_none_or(|scroll| scroll.kind != Scroll::Top)
    {
        let mut state = output.state;
        state.offset.y = (state.offset.y + grew_above).max(0.0);
        state.store(ui.ctx(), output.id);
        ui.ctx()
            .request_discard("transcript rows above the viewport changed height");
    }
    if reader_scrolled {
        key_scroll = None;
    }
    if let Some(scroll) = key_scroll
        && scroll.done()
    {
        key_scroll = None;
        if scroll.kind == Scroll::Bottom {
            // Keep later messages pinned, as Ctrl+End already does.
            app.scroll_to_bottom = true;
        }
    }
    conversation.key_scroll = key_scroll;
    app.conversations
        .insert(chat.id.clone(), std::mem::take(&mut conversation));
    app.at_bottom = at_bottom;
    if app.scroll_to_bottom && (reader_scrolled || (keyboard_navigation.get() && !scroll_forced)) {
        app.scroll_to_bottom = false;
    }
    if divider_placed {
        if let Some(divider) = app
            .unread_divider
            .as_mut()
            .filter(|divider| divider.chat == chat.id)
        {
            divider.placed = true;
        }
        app.scroll_to_bottom = false;
    }
    if let Some(jump) = &mut app.jump_highlight
        && jump.chat == chat.id
    {
        jump.since = jump_since.get();
        if jump
            .since
            .is_some_and(|since| time - since > JumpHighlight::DURATION)
        {
            app.jump_highlight = None;
        }
    }
    if anchored {
        app.scroll_anchor = None;
        // The message the reader jumped to wins over the unread divider,
        // which would otherwise scroll away from it on the next frame.
        if let Some(divider) = app
            .unread_divider
            .as_mut()
            .filter(|divider| divider.chat == chat.id)
        {
            divider.placed = true;
        }
    } else if let Some(anchor) = app.scroll_anchor.clone()
        && !loading
        && !fetching
        && !app
            .conversations
            .get(&chat.id)
            .is_some_and(|c| c.message(&anchor).is_some())
        && app
            .conversations
            .get(&chat.id)
            .is_none_or(|c| c.complete || !c.messages.is_empty())
    {
        // Keep the anchor when the first page is still loading. Event::Messages
        // will request it again.
        app.scroll_anchor = None;
    }
    // At the top, load more from the archive and then the phone. Short chats
    // request more immediately.
    let fits = output.content_size.y <= output.inner_rect.height() + 1.0;
    let near_top = output.state.offset.y < 80.0;
    if (near_top || fits) && ((!complete && !loading) || (complete && !fetching && !exhausted)) {
        actions.push(Action::LoadOlder(chat.id.clone()));
    }
    app.actions.extend(actions);
    if edge_scrolled_up {
        // Scrolling up releases stick-to-bottom.
        app.scroll_to_bottom = false;
    }
    // Show a return-to-bottom button while reading older messages.
    if !at_bottom {
        let rect = output.inner_rect;
        let center = pos2(rect.right() - 34.0, rect.bottom() - 34.0);
        let button = Rect::from_center_size(center, Vec2::splat(40.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        let unread = app.chat(&chat.id).map_or(0, |chat| chat.unread);
        // It floats over the messages: a soft shadow lifts it off them.
        child.painter().add(
            egui::epaint::Shadow {
                offset: [0, 3],
                blur: 10,
                spread: 0,
                color: palette.lift_shadow(),
            }
            .as_shape(button, CornerRadius::same(20)),
        );
        if theme::circle_button(
            &mut child,
            Icon::ArrowDown,
            40.0,
            palette.overlay,
            palette.surface_hover,
            palette.text,
            tr("Newest message"),
        )
        .clicked()
        {
            app.actions.push(Action::ScrollToBottom);
        }
        if unread > 0 {
            widgets::badge(
                ui,
                &palette,
                pos2(center.x + 14.0, center.y - 16.0),
                unread,
                false,
            );
        }
    }
}

/// The disc behind a message's react or reply button: opaque, so the chat's
/// wallpaper does not show through, and lifted like a bubble.
fn floating_circle(ui: &egui::Ui, palette: &Palette, rect: Rect, hovered: bool) {
    let radius = CornerRadius::from(rect.width() / 2.0);
    ui.painter()
        .add(palette.bubble_shadow().as_shape(rect, radius));
    ui.painter().circle_filled(
        rect.center(),
        rect.width() / 2.0,
        if hovered {
            palette.surface_hover
        } else {
            palette.overlay
        },
    );
}

/// Loading state above the oldest visible message, and what the phone did
/// when it was asked for older ones.
fn top_of_history(
    ui: &mut egui::Ui,
    palette: &Palette,
    conversation: &Conversation,
    chat: &Chat,
    actions: &mut Vec<Action>,
) {
    ui.vertical_centered(|ui| {
        if !conversation.complete {
            if conversation.loading_older {
                theme::spinner(ui, 18.0, palette.accent);
            } else {
                ui.add_space(18.0);
            }
        } else if conversation.messages.is_empty() {
            ui.add_space(24.0);
            widgets::chip(ui, palette, tr("No messages here yet"));
            // History sync names old chats without their messages, and the phone
            // answers a request only from a message it can start at.
            ui.add_space(8.0);
            centered_note(ui, |ui| {
                theme::paragraph(
                    ui,
                    tr("Older messages from this chat, if any, stay on your phone: WhatsApp does not send them to linked devices."),
                    theme::regular(12.5),
                    palette.secondary,
                );
            });
        } else if conversation.fetching_phone {
            ui.horizontal(|ui| {
                let width = 260.0;
                ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                theme::spinner(ui, 16.0, palette.accent);
                theme::text(
                    ui,
                    tr("Loading older messages from your phone…"),
                    theme::regular(12.5),
                    palette.secondary,
                );
            });
        } else if conversation.phone_silent {
            ui.add_space(6.0);
            phone_silence(ui, palette, chat, actions);
        } else {
            ui.add_space(6.0);
        }
    });
}

/// Says the phone was asked and sent nothing, why that happens with it online,
/// and offers to ask again.
fn phone_silence(ui: &mut egui::Ui, palette: &Palette, chat: &Chat, actions: &mut Vec<Action>) {
    centered_note(ui, |ui| {
        theme::paragraph(
            ui,
            format!(
                "{} {}",
                tr("Your phone did not send older messages."),
                tr(
                    "WhatsApp gives linked devices only part of the history; the rest stays on the phone."
                )
            ),
            theme::regular(12.5),
            palette.secondary,
        );
        ui.add_space(4.0);
        if theme::link(ui, tr("Try again"), theme::medium(12.5), palette.accent).clicked() {
            actions.push(Action::FetchOlder(chat.id.clone()));
        }
    });
}

/// What stands where the composer would in a chat with a contact the
/// account blocked: nothing can be sent until it is unblocked.
fn blocked_strip(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let palette = app.palette;
    let enabled = app.is_connected() && !app.blocking.contains(&chat.id);
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        theme::text(
            ui,
            tr("You blocked this contact."),
            theme::regular(13.5),
            palette.secondary,
        );
        ui.add_space(2.0);
        ui.add_enabled_ui(enabled, |ui| {
            if theme::link(ui, tr("Unblock"), theme::medium(13.5), palette.accent).clicked() {
                app.actions.push(Action::SetBlocked(chat.id.clone(), false));
            }
        });
        ui.add_space(8.0);
    });
}

/// A column of at most 380 points, centred in the transcript, for lines that
/// wrap.
fn centered_note(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width().min(380.0);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Center), |ui| {
            ui.set_width(width);
            add_contents(ui);
        });
    });
}

/// Typing indicator with stacked avatars and animated dots.
fn typing_bubble(ui: &mut egui::Ui, view: &View<'_>, typers: &[(String, String)]) {
    let palette = view.palette;
    ui.horizontal(|ui| {
        if shows_sender_pictures(view.chat) {
            let count = typers.len().min(3);
            let step = SENDER_AVATAR * 0.6;
            let width = SENDER_AVATAR + step * (count.saturating_sub(1)) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(width, SENDER_AVATAR), Sense::hover());
            if ui.is_rect_visible(rect) {
                for (index, (id, name)) in typers.iter().take(count).enumerate() {
                    let avatar = Rect::from_min_size(
                        rect.min + vec2(step * index as f32, 0.0),
                        Vec2::splat(SENDER_AVATAR),
                    );
                    if index > 0 {
                        // Outline overlapping avatars with the chat background.
                        ui.painter().circle_filled(
                            avatar.center(),
                            SENDER_AVATAR / 2.0 + 1.5,
                            palette.chat,
                        );
                    }
                    widgets::paint_avatar(
                        ui,
                        &palette,
                        avatar,
                        name.trim_start_matches('~'),
                        id,
                        view.avatars.get(id).and_then(|picture| picture.as_deref()),
                    );
                }
            }
            ui.add_space(2.0);
        }
        let backdrop = ui.painter().add(egui::Shape::Noop);
        let rect = Frame::new()
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| typing_dots(ui, &palette))
            .response
            .rect;
        ui.painter().set(
            backdrop,
            widgets::bubble_shape(&palette, rect, palette.bubble_in, Some(widgets::Side::Left)),
        );
    });
}

fn typing_dots(ui: &mut egui::Ui, palette: &Palette) {
    let radius = 3.0;
    let gap = 5.0;
    let lift = 3.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(radius * 6.0 + gap * 2.0, radius * 2.0 + lift),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Every frame, paced by vsync like egui's own animations; drawn only
    // while visible, and a hidden window gets no frames at all.
    ui.ctx().request_repaint();
    let time = ui.input(|input| input.time);
    for index in 0..3 {
        let wave = ((time * std::f64::consts::TAU / 1.2) - f64::from(index) * 0.9).sin() as f32;
        let rise = wave.max(0.0);
        let center = pos2(
            rect.left() + radius + (radius * 2.0 + gap) * index as f32,
            rect.bottom() - radius - rise * lift,
        );
        ui.painter().circle_filled(
            center,
            radius,
            palette.secondary.gamma_multiply(0.45 + 0.55 * rise),
        );
    }
}

const REACTION_AFFORDANCE_SIZE: f32 = 26.0;
const REACTION_AFFORDANCE_GAP: f32 = 6.0;

/// Places the hover reaction control beside the bubble, swapping sides near an edge.
pub fn reaction_affordance_rect(bubble: Rect, bounds: Rect, own: bool) -> Rect {
    let size = Vec2::splat(REACTION_AFFORDANCE_SIZE);
    let before = bubble.left() - REACTION_AFFORDANCE_GAP - size.x;
    let after = bubble.right() + REACTION_AFFORDANCE_GAP;
    let (preferred, fallback) = if own {
        (before, after)
    } else {
        (after, before)
    };
    let fits = |x: f32| x >= bounds.left() && x + size.x <= bounds.right();
    let x = if fits(preferred) {
        preferred
    } else if fits(fallback) {
        fallback
    } else {
        preferred.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()))
    };
    // Centred on the whole bubble, quote and footer included.
    let y = (bubble.center().y - size.y / 2.0)
        .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    Rect::from_min_size(pos2(x, y), size)
}

/// Includes the small gap so moving from the bubble to the control does not hide it.
fn reaction_affordance_visible(pointer: Option<egui::Pos2>, bubble: Rect, button: Rect) -> bool {
    pointer.is_some_and(|pointer| bubble.union(button).contains(pointer))
}

fn open_reaction_picker_action(chat: &str, message: &str) -> Action {
    Action::OpenReactionPicker {
        chat: chat.to_owned(),
        message: message.to_owned(),
        beside_menu: false,
    }
}

/// The screen-reader label names whose message the hover control reacts to, so
/// a user can tell the controls apart. The visual tooltip stays a short "React".
fn reaction_button_label(from_me: bool, sender: &str) -> String {
    if from_me {
        tr("React to your message").to_owned()
    } else {
        tr("React to {sender}'s message").replace("{sender}", sender)
    }
}

/// Draws a Smile control beside a hovered message and opens the existing picker.
fn reaction_affordance(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    bubble: &egui::Response,
    actions: &mut Vec<Action>,
) {
    let bounds = ui.clip_rect().shrink(2.0);
    if bounds.width() < REACTION_AFFORDANCE_SIZE || bounds.height() < REACTION_AFFORDANCE_SIZE {
        return;
    }
    // Only the part of the bubble on screen can be hovered. Anchoring to it
    // keeps a bubble scrolled mostly out of view from parking its controls,
    // and catching the pointer, at the edge of the transcript.
    let shown = bubble.rect.intersect(bounds);
    if shown.height() < REACTION_AFFORDANCE_SIZE || shown.width() <= 0.0 {
        return;
    }
    let rect = reaction_affordance_rect(shown, bounds, message.from_me);
    let pointer = ui.input(|input| input.pointer.interact_pos());
    // Stay hidden under any floating layer, as the context menu does. The
    // affordance sits beside the bubble, so test the bubble itself rather than
    // the pointer, which may already be over the button, outside the bubble's
    // layer.
    let uncovered = ui
        .ctx()
        .layer_id_at(bubble.rect.center())
        .is_none_or(|layer| layer == bubble.layer_id);

    // Publish where the control actually landed, the same way the bubble rect is
    // published, so tests can act on the real rect instead of guessing it.
    ui.ctx()
        .data_mut(|data| data.insert_temp(bubble.id.with("react-rect"), rect));

    // Acquire under the same layer as the bubble so the row strip keeps clicks.
    let response = ui.interact(rect, bubble.id.with("react"), Sense::click());
    let sender = (view.names_or)(&message.sender, message.sender_name.as_deref());
    let label = reaction_button_label(message.from_me, &sender);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    theme::reveal_focus(&response);
    // Reply sits one step further out, where the chat accepts messages.
    let can_reply = view.chat.can_send()
        && !matches!(
            message.content,
            Content::Revoked { .. } | Content::PhoneOnly { .. }
        );
    let step = REACTION_AFFORDANCE_SIZE + 4.0;
    let outward = if rect.center().x < bubble.rect.center().x {
        -step
    } else {
        step
    };
    let reply_rect = rect.translate(vec2(outward, 0.0));
    let reply_fits = can_reply && bounds.contains_rect(reply_rect);
    let reach = if reply_fits {
        rect.union(reply_rect)
    } else {
        rect
    };
    let revealed = reaction_affordance_visible(pointer, shown, reach);
    if ui.is_rect_visible(rect) && (response.has_focus() || (uncovered && revealed)) {
        floating_circle(ui, &view.palette, rect, response.hovered());
        theme::paint_icon(
            ui,
            Icon::Smile,
            rect.shrink(5.0),
            15.0,
            if response.hovered() {
                view.palette.text
            } else {
                view.palette.secondary
            },
        );
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr("React"))
        .clicked()
    {
        actions.push(open_reaction_picker_action(&view.chat.id, &message.id));
    }
    if !reply_fits {
        return;
    }
    let reply = ui.interact(reply_rect, bubble.id.with("reply"), Sense::click());
    reply.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tr("Reply"))
    });
    theme::reveal_focus(&reply);
    if ui.is_rect_visible(reply_rect) && (reply.has_focus() || (uncovered && revealed)) {
        floating_circle(ui, &view.palette, reply_rect, reply.hovered());
        theme::paint_icon(
            ui,
            Icon::Reply,
            reply_rect.shrink(5.0),
            15.0,
            if reply.hovered() {
                view.palette.text
            } else {
                view.palette.secondary
            },
        );
    }
    if reply
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr("Reply"))
        .clicked()
    {
        actions.push(Action::Reply(message.id.clone()));
    }
}

/// Draws a message row and returns its bubble response for scrolling.
fn bubble(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    show_sender: bool,
    first_in_run: bool,
    actions: &mut Vec<Action>,
) -> Option<egui::Response> {
    let own = message.from_me;
    // Greys that read on the panel can vanish on the bubble; use its own.
    let view = &View {
        palette: view.palette.on_bubble(own),
        ..*view
    };
    let with_avatar = !own && shows_sender_pictures(view.chat);
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    let max_width = bubble_width_limit(ui.available_width(), own, carousel, with_avatar);
    // Register the empty strip beside the bubble from its previous rect, before
    // the row, so the avatar, the bubble, and the reactions win clicks.
    let id = bubble_id(&view.chat.id, &message.id);
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(id.with("rect")));
    if let Some(rect) = previous {
        let strip = Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
        let strip = ui.interact(strip, id.with("row"), Sense::CLICK);
        if strip.clicked() {
            actions.push(Action::FocusComposer);
        }
        reply_on_double_click(&strip, message, actions);
    }
    let mut response = None;
    ui.with_layout(
        Layout::top_down(if own { Align::Max } else { Align::Min }),
        |ui| {
            if with_avatar {
                ui.horizontal_top(|ui| {
                    let (rect, avatar) = ui.allocate_exact_size(
                        Vec2::splat(SENDER_AVATAR),
                        if show_sender {
                            Sense::CLICK
                        } else {
                            Sense::hover()
                        },
                    );
                    theme::focus_outline(ui, avatar.id, rect, SENDER_AVATAR / 2.0);
                    if show_sender
                        && avatar
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        actions.push(Action::ShowDialog(Dialog::ChatInfo(message.sender.clone())));
                    }
                    if show_sender && ui.is_rect_visible(rect) {
                        let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                        widgets::paint_avatar(
                            ui,
                            &view.palette,
                            rect,
                            name.trim_start_matches('~'),
                            &message.sender,
                            view.avatars
                                .get(&message.sender)
                                .and_then(|picture| picture.as_deref()),
                        );
                    }
                    // Keep bubble content vertically laid out inside the row.
                    ui.vertical(|ui| {
                        response = Some(bubble_frame(
                            ui,
                            view,
                            message,
                            show_sender,
                            first_in_run,
                            max_width,
                            actions,
                        ));
                    });
                });
            } else {
                response = Some(bubble_frame(
                    ui,
                    view,
                    message,
                    show_sender,
                    first_in_run,
                    max_width,
                    actions,
                ));
            }
            if !message.reactions.is_empty() {
                ui.add_space(-7.0);
                ui.horizontal(|ui| {
                    if with_avatar {
                        ui.add_space(SENDER_AVATAR + 8.0);
                    }
                    reactions(ui, view, message, actions);
                });
                ui.add_space(2.0);
            }
        },
    );
    response
}

/// Clamps message-selection drags to the view while the pointer is outside it.
/// This keeps a row under the pointer during edge scrolling. The input hook
/// adjusts positions before egui processes them, using the previous frame's
/// view rectangle.
pub struct SelectionLeash {
    pub view: std::sync::Arc<std::sync::Mutex<Option<Rect>>>,
    holding: bool,
}

impl SelectionLeash {
    pub fn new(view: std::sync::Arc<std::sync::Mutex<Option<Rect>>>) -> Self {
        Self {
            view,
            holding: false,
        }
    }
}

impl egui::plugin::Plugin for SelectionLeash {
    fn debug_name(&self) -> &'static str {
        "zapfast-selection-leash"
    }

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let Some(view) = *self.view.lock().unwrap_or_else(|p| p.into_inner()) else {
            self.holding = false;
            return;
        };
        let inside = |pos: &egui::Pos2| view.contains(*pos) && pos.x < view.right() - 16.0;
        let mut gone = Vec::new();
        for (index, event) in input.events.iter_mut().enumerate() {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    if *pressed {
                        // The chat list's resize edge reaches a few points
                        // into the view; a drag begun there resizes the list
                        // and must be free to leave the view to the left.
                        let on_list_edge = ctx
                            .read_response(super::chats::resize_handle_id())
                            .is_some_and(|edge| edge.rect.contains(*pos));
                        self.holding = inside(pos) && !on_list_edge;
                    } else {
                        if self.holding && !view.contains(*pos) {
                            *pos = clamp_into(*pos, view);
                        }
                        self.holding = false;
                    }
                }
                egui::Event::PointerMoved(pos) if self.holding && !view.contains(*pos) => {
                    *pos = clamp_into(*pos, view);
                }
                // Ignore PointerGone during a drag so selection continues.
                egui::Event::PointerGone if self.holding => gone.push(index),
                _ => {}
            }
        }
        for index in gone.into_iter().rev() {
            input.events.remove(index);
        }
    }
}

fn clamp_into(pos: egui::Pos2, view: Rect) -> egui::Pos2 {
    egui::pos2(
        pos.x.clamp(view.left() + 2.0, view.right() - 18.0),
        pos.y.clamp(view.top() + 2.0, view.bottom() - 2.0),
    )
}

/// The text "Copy text" takes from a message: its words, a caption, the
/// coordinates of a location, or a contact's card.
fn copyable_text(content: &Content) -> Option<String> {
    match content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.clone()),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.clone(),
        Content::Location {
            latitude,
            longitude,
            ..
        }
        | Content::LiveLocation {
            latitude,
            longitude,
            ..
        } => Some(format!("{latitude},{longitude}")),
        Content::Contact { vcard, .. } => Some(vcard.clone()),
        _ => None,
    }
}

/// What a message without text is called in copied text: "[photo]", "[video]",
/// "[document: notes.pdf]", and so on.
fn content_marker(content: &Content) -> Option<String> {
    match content {
        Content::Image { .. } => Some(tr("[photo]").to_owned()),
        Content::Video { gif: true, .. } => Some(tr("[GIF]").to_owned()),
        Content::Video { .. } => Some(tr("[video]").to_owned()),
        Content::Audio {
            voice_note: true,
            seconds,
            ..
        } => Some(match seconds {
            Some(seconds) => tr("[voice message, {duration}]")
                .replace("{duration}", &crate::util::duration(*seconds)),
            None => tr("[voice message]").to_owned(),
        }),
        Content::Audio { .. } => Some(tr("[audio]").to_owned()),
        Content::Document { file_name, .. } => {
            Some(tr("[document: {name}]").replace("{name}", file_name))
        }
        Content::Sticker { .. } => Some(tr("[sticker]").to_owned()),
        Content::Location { .. } => Some(tr("[location]").to_owned()),
        Content::LiveLocation { ended, .. } => Some(
            if *ended {
                tr("[live location ended]")
            } else {
                tr("[live location]")
            }
            .to_owned(),
        ),
        Content::Contact { display_name, .. } => {
            Some(tr("[contact: {name}]").replace("{name}", display_name))
        }
        Content::StickerPack { name, .. } => {
            Some(tr("[sticker pack: {name}]").replace("{name}", name))
        }
        Content::Poll { question, .. } => {
            Some(tr("[poll: {question}]").replace("{question}", question))
        }
        Content::Interactive {
            card: Some(card), ..
        } if card.image.is_some() || card.carousel.iter().any(|card| card.image.is_some()) => {
            Some(tr("[photo]").to_owned())
        }
        _ => None,
    }
}

/// Builds the transcript row used when copying across messages.
fn transcript_row(
    view: &View<'_>,
    message: &Message,
    body: String,
    placements: Vec<String>,
) -> crate::transcript::Row {
    let who = if message.from_me {
        (view.mention_names)(&message.sender)
    } else {
        (view.names_or)(&message.sender, message.sender_name.as_deref())
    };
    let marker = content_marker(&message.content);
    let reactions = if message.reactions.is_empty() {
        String::new()
    } else {
        let listed: Vec<String> = message
            .reactions
            .iter()
            .map(|reaction| {
                format!(
                    "{} {}",
                    reaction.emoji,
                    (view.names_or)(&reaction.sender, None)
                )
            })
            .collect();
        format!(" ({})", listed.join(", "))
    };
    let quote = message.quoted.as_ref().map(|quoted| {
        let name = quoted
            .sender_name
            .clone()
            .unwrap_or_else(|| (view.names_or)(&quoted.sender, None));
        let summary = markup::plain(&quoted.summary, &quote_mentions(view, quoted));
        let short: String = summary.chars().take(48).collect();
        let cut = if summary.chars().count() > 48 {
            "…"
        } else {
            ""
        };
        tr("(replying to {name}: \"{quote}\")")
            .replace("{name}", &name)
            .replace("{quote}", &format!("{short}{cut}"))
    });
    crate::transcript::Row {
        header: format!("[{}] {}: ", crate::util::copy_stamp(message.timestamp), who),
        body,
        placements,
        marker,
        reactions,
        quote,
    }
}

/// Selection-scroll distance based on pointer proximity to the view edge.
pub fn edge_scroll(pointer: f32, top: f32, bottom: f32) -> f32 {
    const EDGE: f32 = 36.0;
    const PACE: f32 = 0.3;
    if pointer < top + EDGE {
        -(top + EDGE - pointer).min(EDGE * 1.5) * PACE
    } else if pointer > bottom - EDGE {
        (pointer - (bottom - EDGE)).min(EDGE * 1.5) * PACE
    } else {
        0.0
    }
}

/// Starts a reply when the response was double-clicked, as the menu's "Reply".
fn reply_on_double_click(response: &egui::Response, message: &Message, actions: &mut Vec<Action>) {
    if response.double_clicked() && !matches!(message.content, Content::Revoked { .. }) {
        actions.push(Action::Reply(message.id.clone()));
    }
}

/// Stable message-bubble id used by interaction tests.
pub fn bubble_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("bubble", chat, message))
}

/// Where a voice message's speed chip was drawn, for interaction tests.
pub fn speed_chip_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("speed-chip", chat, message))
}

/// Where a speed choice in a voice message's menu was drawn, for
/// interaction tests.
pub fn speed_button_id(chat: &str, message: &str, speed: f32) -> egui::Id {
    egui::Id::new(("speed", chat, message, speed.to_bits()))
}

/// Draws a playback speed pill labelled with `speed`, highlighted when
/// `active` and faded while that speed is still `preparing`.
/// The speed chip's label colour: the usual text colour at 1x, then light
/// green from 1.5x (1.25x included), shading into yellow at 2x and red at 3x.
fn speed_colour(palette: &Palette, speed: f32) -> Color32 {
    if speed <= 1.0 {
        return palette.secondary;
    }
    // Lighter shades read on the dark chip, deeper ones on the light chip.
    let (green, yellow, red) = if palette.dark {
        (
            Color32::from_rgb(144, 238, 144),
            Color32::from_rgb(250, 214, 70),
            Color32::from_rgb(255, 99, 90),
        )
    } else {
        (
            Color32::from_rgb(46, 160, 67),
            Color32::from_rgb(190, 140, 0),
            Color32::from_rgb(211, 47, 47),
        )
    };
    if speed <= 1.5 {
        green
    } else if speed <= 2.0 {
        green.lerp_to_gamma(yellow, (speed - 1.5) / 0.5)
    } else {
        yellow.lerp_to_gamma(red, (speed - 2.0).min(1.0))
    }
}

/// A playback speed button. `chip` is the one in the voice player: its fill
/// stays the resting one and its label takes the speed's colour.
fn speed_pill(
    ui: &mut egui::Ui,
    view: &View<'_>,
    size: Vec2,
    speed: f32,
    active: bool,
    preparing: bool,
    chip: bool,
) -> egui::Response {
    let palette = view.palette;
    let label = crate::audio::speed_label(speed);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            active,
            tr("Playback speed {label}").replace("{label}", &label),
        )
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // The resting fill uses the hover step because incoming bubbles
        // share the resting surface colour.
        let fill = if active && !chip {
            palette
                .accent
                .gamma_multiply(if hovered { 0.42 } else { 0.30 })
        } else if hovered {
            palette.surface_active
        } else {
            palette.surface_hover
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let colour = if chip {
            speed_colour(&palette, speed)
        } else if active {
            palette.accent
        } else {
            palette.secondary
        };
        let colour = if preparing {
            colour.gamma_multiply(0.5)
        } else {
            colour
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, theme::medium(11.0), colour);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The message menu's row of every playback speed for a playable voice
/// message, so 1.25x and 1.75x are reachable without cycling.
fn speed_menu_row(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    actions: &mut Vec<Action>,
) {
    let speed = view.player.speed();
    let preparing = view.player.preparing_speed(&message.id);
    widgets::menu_separator(ui, &view.palette);
    // The phone's speeds on the first row, 2.5x and 3x on the second.
    let (phone, faster) = crate::audio::SPEEDS.split_at(5);
    for row in [phone, faster] {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 28.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(6.0);
                for &option in row {
                    let selected = option == speed;
                    let response = speed_pill(
                        ui,
                        view,
                        vec2(44.0, 24.0),
                        option,
                        selected,
                        preparing && selected,
                        false,
                    );
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            speed_button_id(&view.chat.id, &message.id, option),
                            response.rect,
                        );
                    });
                    if response.clicked() {
                        actions.push(Action::SetVoiceSpeed(option));
                        ui.close();
                    }
                }
            },
        );
    }
}

/// Draws a message bubble and its menu.
fn bubble_frame(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    show_sender: bool,
    first_in_run: bool,
    max_width: f32,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let palette = view.palette;
    let own = message.from_me;
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    // Stickers and round video messages draw without a bubble.
    let bare = matches!(
        message.content,
        Content::Sticker { .. } | Content::Video { note: true, .. }
    );
    let fill = if carousel || bare {
        Color32::TRANSPARENT
    } else if own {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    // Register the bubble from its previous rect before its contents so inner
    // links, quotes, and attachments win clicks. The bubble handles right-click.
    let bubble_id = bubble_id(&view.chat.id, &message.id);
    // Store the final rect separately. Reusing the early response would keep
    // the first frame's rect.
    let rect_id = bubble_id.with("rect");
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(rect_id));
    let early = previous.map(|rect| ui.interact(rect, bubble_id, Sense::CLICK));
    // Painted once the contents are measured, beneath them.
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let tail = (first_in_run && fill != Color32::TRANSPARENT).then_some(if own {
        widgets::Side::Right
    } else {
        widgets::Side::Left
    });
    // A picture without a caption carries its time over its corner, so
    // the bubble closes under it as evenly as it opens above it.
    let over_picture = time_over_picture(message);
    let inner = Frame::new()
        .inner_margin(Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: if over_picture { 6 } else { 5 },
        })
        .show(ui, |ui| {
            ui.set_max_width(max_width);
            ui.spacing_mut().item_spacing.y = 4.0;
            if show_sender && view.chat.is_group() {
                let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                let response = widgets::rich_text(
                    ui,
                    &name,
                    theme::semibold(13.0),
                    palette.sender(crate::util::hue(&message.sender)),
                );
                let response = ui
                    .interact(
                        response.rect,
                        ui.id().with(("sender", &message.id)),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    actions.push(Action::ShowDialog(Dialog::ChatInfo(message.sender.clone())));
                }
            }
            // Reserve the label's line now and paint it once the contents
            // are measured, at their start: an own bubble lays out from the
            // right, and its width is known only then.
            let forwarded = message.forwarded.then(|| forwarded_label(ui, &palette));
            // Cards share the bubble's settled width: at least CARD_WIDTH and
            // no more than the cap. Text spans that width and stays left-aligned.
            // Bubbles without cards use the natural text width.
            let cap = ((max_width - 20.0).min(ui.available_width())).max(0.0);
            let reserve = footer_width(ui, message);
            let settled = settled_width(ui, view, message, cap);
            let slot = match settled {
                Some(width) => {
                    if let Some(quoted) = &message.quoted {
                        quote_block(ui, view, message, quoted, width, actions);
                    }
                    content(ui, view, message, width, reserve, actions)
                }
                None => content(ui, view, message, cap, reserve, actions),
            };
            if over_picture && let Some(picture) = slot {
                footer_over_picture(ui, &palette, message, picture);
            } else {
                footer(ui, &palette, message, slot);
            }
            if matches!(message.content, Content::Poll { .. }) {
                super::polls::results_button(
                    ui,
                    &palette,
                    message,
                    settled.unwrap_or(cap),
                    actions,
                );
            }
            if let Content::Interactive {
                card: Some(card), ..
            } = &message.content
            {
                interactive_buttons(ui, view, message, card, settled.unwrap_or(cap), actions);
            }
            if let Some((reserved, galley)) = forwarded {
                let at = pos2(ui.min_rect().left(), reserved.top());
                let painted = paint_forwarded_label(ui, &palette, at, galley);
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id.with("forwarded"), painted);
                });
            }
        });
    if fill != Color32::TRANSPARENT && ui.is_rect_visible(inner.response.rect) {
        ui.painter().set(
            backdrop,
            widgets::bubble_shape(&palette, inner.response.rect, fill, tail),
        );
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(rect_id, inner.response.rect));
    let bubble = early.unwrap_or_else(|| ui.interact(inner.response.rect, bubble_id, Sense::CLICK));
    theme::reveal_focus(&bubble);
    theme::focus_outline(ui, bubble.id, inner.response.rect, 10.0);
    if ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    }) && let Some(focused) = ui
        .memory(|memory| memory.focused())
        .and_then(|id| ui.ctx().read_response(id))
        && (focused.id == bubble.id || inner.response.rect.contains(focused.rect.center()))
    {
        view.keyboard_navigation.set(true);
        if focused.gained_focus() && !ui.clip_rect().contains_rect(focused.rect) {
            ui.scroll_to_rect_animation(
                inner.response.rect.expand(4.0),
                None,
                egui::style::ScrollAnimation::none(),
            );
        }
    }
    // The context menu stays open beside the picker only when the picker came
    // from the menu itself.
    let reacting = view.reaction_menu && view.reaction == Some(message.id.as_str());
    // Inner widgets own their clicks, so this fires only on the bubble's padding
    // and footer. Double-click on the body keeps selecting the word.
    reply_on_double_click(&bubble, message, actions);

    reaction_affordance(ui, view, message, &bubble, actions);
    // Read right-click from input because inner widgets own their responses.
    // The whole row counts, the empty strip beside the bubble included, as in
    // other messaging apps (#240). Count only the part inside the transcript's
    // viewport: the chat header shares this layer, and a bubble scrolled under
    // it is hidden there. Open only when no floating layer covers the chat
    // panel.
    let viewport = ui.clip_rect();
    let shown =
        Rect::from_x_y_ranges(viewport.x_range(), bubble.rect.y_range()).intersect(viewport);
    let right_clicked = ui.input(|input| {
        input.pointer.secondary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| shown.contains(pos))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer == bubble.layer_id)
        });
    let force_menu = view.open_menu == Some(message.id.as_str());
    let quick = quick_reactions(message, view.reaction_emoji).len() as f32 + 1.0;
    let width = widgets::menu_width(
        ui,
        &[
            tr("Delete for everyone"),
            tr("Show in folder"),
            tr("Copy message ID"),
            &crate::i18n::gettext(view.locale, "Open in system player"),
            &crate::i18n::gettext(view.locale, "Message info"),
            &crate::i18n::gettext(view.locale, "Reload earlier messages"),
        ],
        true,
    )
    .max(quick * 36.0 + 12.0);
    let keyboard_clicked =
        bubble.clicked() && bubble.has_focus() && !ui.input(|input| input.pointer.any_click());
    let open = if right_clicked || force_menu || reacting || keyboard_clicked {
        Some(egui::SetOpenCommand::Bool(true))
    } else if bubble.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let popup = egui::Popup::menu(&bubble)
        .open_memory(open)
        .close_behavior(if reacting {
            egui::PopupCloseBehavior::IgnoreClicks
        } else {
            egui::PopupCloseBehavior::CloseOnClickOutside
        })
        .width(width)
        .frame(widgets::menu_frame(&palette));
    let popup = if reacting {
        // Keep the menu next to the picker, anchored to this message rather
        // than whichever pointer position happened to open the emoji grid.
        let screen = ui.ctx().content_rect();
        let menu = ui
            .ctx()
            .data(|data| data.get_temp::<Rect>(bubble_id.with("menu-rect")))
            .unwrap_or(bubble.rect);
        let x = menu.left().clamp(
            screen.left() + 8.0,
            (screen.right() - width - 468.0).max(screen.left() + 8.0),
        );
        popup.at_position(pos2(x, menu.top()))
    } else if force_menu || keyboard_clicked {
        popup.at_position(bubble.rect.left_top() + vec2(12.0, 8.0))
    } else {
        popup.at_pointer_fixed()
    };
    let menu = popup.show(|ui| {
        context_menu(ui, view, message, actions);
    });
    if let Some(menu) = menu {
        ui.ctx()
            .data_mut(|data| data.insert_temp(bubble_id.with("menu-rect"), menu.response.rect));
    }
    // Keep the target explicit for every menu action, not just the emoji
    // picker. Paint in the message layer so the menu itself remains above it.
    if egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        ui.painter().rect_stroke(
            inner.response.rect.expand(2.0),
            12.0,
            Stroke::new(theme::FOCUS_STROKE_WIDTH, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    if reacting && !egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        actions.push(Action::ClosePicker);
    }
    // This frame's final rect, for later scrolling, with the bubble's own
    // clicks: the frame alone only senses hover, so a Ctrl-click to select
    // or a click to add to a selection would never register.
    inner.response.union(bubble)
}

/// Minimum shared width for cards inside message bubbles.
const CARD_WIDTH: f32 = 320.0;
const CAROUSEL_CARD_WIDTH: f32 = 280.0;
/// Location cards: a map preview across the top, then the details.
const LOCATION_CARD_WIDTH: f32 = 300.0;
const CAROUSEL_GAP: f32 = 8.0;

/// Returns the shared card width, bounded by [`CARD_WIDTH`] and `cap`.
fn settled_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    if matches!(
        message.content,
        Content::Location { .. } | Content::LiveLocation { .. }
    ) {
        return Some(LOCATION_CARD_WIDTH.min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && !card.carousel.is_empty()
    {
        // Keep short carousels, their heading and their timestamp together.
        // Wider strips still occupy the cap and scroll horizontally.
        let count = card.carousel.len();
        let cards = count as f32 * (CAROUSEL_CARD_WIDTH + 20.0);
        let gaps = count.saturating_sub(1) as f32 * CAROUSEL_GAP;
        return Some((cards + gaps).min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && card.image.is_some()
    {
        return Some(CARD_WIDTH.min(cap));
    }
    let card = message.quoted.is_some()
        || match &message.content {
            Content::Text { preview, .. } => preview.is_some(),
            Content::Document { .. }
            | Content::Audio { .. }
            | Content::Poll { .. }
            | Content::PhoneOnly { .. } => true,
            Content::Interactive { card, .. } => card.is_some(),
            _ => false,
        };
    card.then(|| {
        let floor = CARD_WIDTH.min(cap).max(0.0);
        natural_text_width(ui, view, message, cap).map_or(floor, |width| width.clamp(floor, cap))
    })
}

/// Widest wrapped text row, including footer space on the last line.
fn natural_text_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    let palette = view.palette;
    let text = match &message.content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => text,
        Content::Image {
            caption: Some(caption),
            ..
        }
        | Content::Video {
            caption: Some(caption),
            ..
        }
        | Content::Document {
            caption: Some(caption),
            ..
        } => caption,
        // A shown transcript widens the card as a caption would.
        Content::Audio { .. } => {
            let key = (view.chat.id.clone(), message.id.clone());
            match view.transcripts.get(&key) {
                Some(transcript) if !view.transcripts_folded.contains(&key) => &transcript.text,
                _ => return None,
            }
        }
        _ => return None,
    };
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions_of(view, message), &style, cap);
    let widest = laid
        .galley
        .rows
        .iter()
        .map(|row| row.row.size.x)
        .fold(0.0, f32::max);
    let last = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    let reserve = footer_width(ui, message);
    Some(if last + 8.0 + reserve <= cap {
        widest.max(last + 8.0 + reserve)
    } else {
        widest
    })
}

/// Width of a quote's accent bar.
const QUOTE_BAR: f32 = 4.0;
/// Side of a quoted photo's square in a reply.
const QUOTE_PICTURE: f32 = 48.0;
/// Corner radius of a quote.
const QUOTE_RADIUS: u8 = 6;

/// Where a quoted photo or video's small picture comes from.
#[derive(Clone)]
enum QuotePicture {
    /// The downloaded photo.
    File(PathBuf),
    /// The thumbnail that came with the message.
    Thumbnail {
        chat: ChatId,
        id: String,
        bytes: Vec<u8>,
    },
}

/// The picture a reply to `message` shows beside its quote, if any: the
/// downloaded photo, or else the thumbnail of a photo or video.
fn quote_picture(message: &Message) -> Option<QuotePicture> {
    let thumbnail = || {
        message
            .thumbnail
            .clone()
            .map(|bytes| QuotePicture::Thumbnail {
                chat: message.chat.clone(),
                id: message.id.clone(),
                bytes,
            })
    };
    match &message.content {
        Content::Image { media, .. } => media
            .path
            .clone()
            .map(QuotePicture::File)
            .or_else(thumbnail),
        Content::Video { .. } => thumbnail(),
        _ => None,
    }
}

/// Pictures for the quotes in `messages`, by quoted message id, from the
/// quoted messages that are loaded.
fn quote_pictures(messages: &[Message]) -> HashMap<String, QuotePicture> {
    let quoted: HashSet<&str> = messages
        .iter()
        .filter_map(|message| message.quoted.as_ref().map(|quoted| quoted.id.as_str()))
        .collect();
    if quoted.is_empty() {
        return HashMap::new();
    }
    messages
        .iter()
        .filter(|message| quoted.contains(message.id.as_str()))
        .filter_map(|message| Some((message.id.clone(), quote_picture(message)?)))
        .collect()
}

/// Paints a quoted picture cropped to fill `rect`, or `fallback` while it
/// loads.
fn paint_quote_picture(
    ui: &egui::Ui,
    picture: &QuotePicture,
    rect: Rect,
    corners: CornerRadius,
    fallback: Color32,
) {
    let image = match picture {
        QuotePicture::File(path) => widgets::file_image(ui, path),
        QuotePicture::Thumbnail { chat, id, bytes } => {
            egui::Image::new(thumbnail_uri(ui.ctx(), chat, id, bytes))
        }
    };
    let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(ui.ctx(), rect.size())
    else {
        ui.painter().rect_filled(rect, corners, fallback);
        return;
    };
    egui::Image::from_texture(texture)
        .uv(cover_uv(texture.size, rect.size()))
        .corner_radius(corners)
        .paint_at(ui, rect);
}

/// The part of a picture of `size` that fills a frame of `frame` without
/// stretching, centred.
fn cover_uv(size: Vec2, frame: Vec2) -> Rect {
    let picture = size.x.max(1.0) / size.y.max(1.0);
    let target = frame.x.max(1.0) / frame.y.max(1.0);
    if picture > target {
        let visible = target / picture;
        let left = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(left + visible, 1.0))
    } else {
        let visible = picture / target;
        let top = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(1.0, top + visible))
    }
}

fn quote_block(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    quoted: &crate::model::Quoted,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let mine = view.me == Some(quoted.sender.as_str());
    let who = if mine {
        tr("You").to_owned()
    } else {
        (view.names_or)(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary, &quote_mentions(view, quoted));
    // As in WhatsApp, the bar and name take the quoted sender's colour, the
    // one their name has in groups, kept readable on this bubble.
    let bubble = if message.from_me {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    let tint = theme::readable_on(
        bubble,
        if mine {
            palette.accent
        } else {
            palette.sender(crate::util::hue(&quoted.sender))
        },
        palette.text,
        3.0,
    );
    // A quoted photo or video shows a small square of it at the right end,
    // as on the phone.
    let picture = view.quote_pictures.get(&quoted.id);
    let side = if picture.is_some() {
        QUOTE_PICTURE
    } else {
        0.0
    };
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(QUOTE_RADIUS))
        .inner_margin(Margin {
            left: QUOTE_BAR as i8 + 7,
            right: if picture.is_some() {
                side as i8 + 8
            } else {
                10
            },
            top: 5,
            bottom: 6,
        })
        .show(ui, |ui| {
            // Include frame margins in the settled width. Use a bounded,
            // left-aligned layout because own bubbles inherit right-to-left flow.
            let extra = if picture.is_some() { side - 2.0 } else { 0.0 };
            let inner_width = (width - QUOTE_BAR - 17.0 - extra).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(inner_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(inner_width);
                    ui.set_min_height((side - 11.0).max(0.0));
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, &who, theme::semibold(12.5), tint);
                    widgets::rich_text(ui, &summary, theme::regular(12.5), palette.secondary);
                },
            );
        })
        .response;
    if let Some(picture) = picture {
        let rect = Rect::from_min_max(
            egui::pos2(response.rect.right() - side, response.rect.top()),
            response.rect.right_bottom(),
        );
        let corners = CornerRadius {
            nw: 0,
            sw: 0,
            ne: QUOTE_RADIUS,
            se: QUOTE_RADIUS,
        };
        if ui.is_rect_visible(rect) {
            paint_quote_picture(ui, picture, rect, corners, palette.surface);
        }
    }
    // The bar runs the quote's full height along its rounded left edge.
    let bar = Rect::from_min_size(response.rect.min, vec2(QUOTE_BAR, response.rect.height()));
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: QUOTE_RADIUS,
            sw: QUOTE_RADIUS,
            ne: 0,
            se: 0,
        },
        tint,
    );
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("quote"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("quote", &message.id, &quoted.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::ScrollTo(quoted.id.clone()));
    }
}

/// Keeps row contents left-to-right inside right-aligned own bubbles.
fn mirrored_row(
    ui: &mut egui::Ui,
    own: bool,
    first: impl FnOnce(&mut egui::Ui),
    second: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        if own {
            second(ui);
            first(ui);
        } else {
            first(ui);
            second(ui);
        }
    });
}

/// Size of the forwarded label's arrow.
const FORWARDED_ICON: f32 = 14.0;
/// Gap between the forwarded label's arrow and its word.
const FORWARDED_GAP: f32 = 4.0;
/// Height of the forwarded label's line: its word's height, without the
/// taller line an icon and a label in a row would take.
const FORWARDED_HEIGHT: f32 = 15.0;

/// Reserves the line for a forwarded message's label at the top of its
/// bubble and returns that space with the laid-out word, for
/// [`paint_forwarded_label`] once the bubble's contents are measured.
fn forwarded_label(ui: &mut egui::Ui, palette: &Palette) -> (Rect, std::sync::Arc<egui::Galley>) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "Forwarded",
        0.0,
        egui::TextFormat {
            font_id: theme::regular(12.5),
            color: palette.dim,
            italics: true,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let width = FORWARDED_ICON + FORWARDED_GAP + galley.size().x;
    let (reserved, response) =
        ui.allocate_exact_size(vec2(width, FORWARDED_HEIGHT), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Forwarded"));
    // Sit closer to what follows than the bubble's usual spacing.
    ui.add_space(-1.0);
    (reserved, galley)
}

/// Paints a forwarded label with its left edge at `at`, as WhatsApp does:
/// at the start of the bubble on either side. Returns where it went.
fn paint_forwarded_label(
    ui: &egui::Ui,
    palette: &Palette,
    at: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
) -> Rect {
    let rect = Rect::from_min_size(
        at,
        vec2(
            FORWARDED_ICON + FORWARDED_GAP + galley.size().x,
            FORWARDED_HEIGHT,
        ),
    );
    let icon = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - FORWARDED_ICON / 2.0),
        Vec2::splat(FORWARDED_ICON),
    );
    theme::paint_icon(ui, Icon::Forward, icon, FORWARDED_ICON, palette.dim);
    ui.painter().galley(
        pos2(
            icon.right() + FORWARDED_GAP,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        palette.dim,
    );
    rect
}

/// Width of the message footer.
fn footer_width(ui: &egui::Ui, message: &Message) -> f32 {
    let font = theme::regular(11.0);
    let time = ui
        .painter()
        .layout_no_wrap(
            crate::util::clock(message.timestamp),
            font.clone(),
            Color32::WHITE,
        )
        .size()
        .x;
    let edited = if message.edited {
        ui.painter()
            .layout_no_wrap(tr("edited").to_owned(), font, Color32::WHITE)
            .size()
            .x
            + 4.0
    } else {
        0.0
    };
    let not_sent = if not_sent(message) {
        ui.painter()
            .layout_no_wrap(tr(NOT_SENT).to_owned(), theme::medium(11.0), Color32::WHITE)
            .size()
            .x
            + 6.0
    } else {
        0.0
    };
    time + edited + not_sent + if message.from_me { 19.0 } else { 0.0 }
}

/// Whether the message's time and ticks sit over its picture rather than
/// on a line of their own: a picture without a caption, as in WhatsApp.
fn time_over_picture(message: &Message) -> bool {
    matches!(message.content, Content::Image { caption: None, .. })
}

/// Space between a picture's edges and the time drawn over it: the scrim
/// around the time keeps a few points clear of the rounded corner.
const OVER_PICTURE_INSET: Vec2 = vec2(10.0, 6.0);

/// Paints the time and ticks over the bottom corner of a picture without a
/// caption, in white on a soft dark scrim so they read on any picture.
fn footer_over_picture(ui: &mut egui::Ui, palette: &Palette, message: &Message, picture: Rect) {
    let font = theme::regular(11.0);
    let time =
        ui.painter()
            .layout_no_wrap(crate::util::clock(message.timestamp), font, Color32::WHITE);
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let width =
        time.size().x + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0) + tick_width;
    let row = Rect::from_min_max(
        pos2(
            picture.right() - OVER_PICTURE_INSET.x - width,
            picture.bottom() - OVER_PICTURE_INSET.y - 15.0,
        ),
        picture.right_bottom() - OVER_PICTURE_INSET,
    );
    if ui.is_rect_visible(picture) {
        let scrim = row.expand2(vec2(6.0, 2.0)).intersect(picture);
        ui.painter()
            .rect_filled(scrim, scrim.height() / 2.0, Color32::from_black_alpha(110));
    }
    let mut x = row.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, row.center().y), Vec2::splat(15.0));
        widgets::ticks_in(ui, palette, ticks, message.status, Color32::WHITE);
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, row.center().y - time.size().y / 2.0),
        time,
        Color32::WHITE,
    );
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, row.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, Color32::WHITE);
        let status = Rect::from_min_max(label.min, pos2(row.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(footer_id(&message.chat, &message.id), row);
    });
}

/// Where a message's time and ticks were drawn, for layout tests.
pub fn footer_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("footer")
}

fn not_sent(message: &Message) -> bool {
    message.from_me && message.status == Delivery::Failed
}

/// Paints the time and ticks at the bubble's right edge without widening it.
fn footer(ui: &mut egui::Ui, palette: &Palette, message: &Message, slot: Option<Rect>) {
    let font = theme::regular(11.0);
    let time = ui.painter().layout_no_wrap(
        crate::util::clock(message.timestamp),
        font.clone(),
        palette.secondary,
    );
    let edited = message.edited.then(|| {
        ui.painter()
            .layout_no_wrap(tr("edited").to_owned(), font, palette.dim)
    });
    // A red dot alone does not say what went wrong or what to do. The word
    // uses the text colour: the danger red on an outgoing bubble is too faint
    // to read, and the red icon beside it already carries the alarm.
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(tr(NOT_SENT).to_owned(), theme::medium(11.0), palette.text)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let width = time.size().x
        + edited.as_ref().map_or(0.0, |galley| galley.size().x + 4.0)
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width;
    let rect = match slot {
        Some(slot) => slot,
        None => {
            let row_width = ui.min_rect().width().max(width);
            let (rect, _) = ui.allocate_exact_size(vec2(row_width, 15.0), Sense::hover());
            rect
        }
    };
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            footer_id(&message.chat, &message.id),
            Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max),
        );
    });
    let mut x = rect.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, rect.center().y), Vec2::splat(15.0));
        widgets::ticks(ui, palette, ticks, message.status);
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - time.size().y / 2.0),
        time,
        palette.secondary,
    );
    if let Some(edited) = edited {
        x -= edited.size().x + 4.0;
        ui.painter().galley(
            pos2(x, rect.center().y - edited.size().y / 2.0),
            edited,
            palette.dim,
        );
    }
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, rect.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, palette.text);
        // Explain on hover and to screen readers; hovering takes no clicks
        // from the bubble.
        let status = Rect::from_min_max(label.min, pos2(rect.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tr(NOT_SENT_HINT))
        });
        response.on_hover_text(tr(NOT_SENT_HINT));
    }
}

/// The reaction chips of a message: emoji, count, whether one is ours, and
/// who sent them. Skin-tone variants of one emoji share a chip, which shows
/// our own tone when we reacted and the first one seen otherwise.
fn reaction_chips(
    reactions: &[Reaction],
    who: impl Fn(&Reaction) -> String,
) -> Vec<(String, u32, bool, Vec<String>)> {
    let mut counts: Vec<(String, u32, bool, Vec<String>)> = Vec::new();
    for reaction in reactions {
        let name = who(reaction);
        let family = crate::app::emoji_family(&reaction.emoji);
        match counts
            .iter_mut()
            .find(|(emoji, ..)| crate::app::emoji_family(emoji) == family)
        {
            Some((emoji, count, mine, names)) => {
                *count += 1;
                *mine |= reaction.from_me;
                if reaction.from_me {
                    emoji.clone_from(&reaction.emoji);
                }
                names.push(name);
            }
            None => counts.push((reaction.emoji.clone(), 1, reaction.from_me, vec![name])),
        }
    }
    counts
}

/// How far one press of ↑ or ↓ scrolls the history, in points: a few lines.
const KEY_LINE_SCROLL: f32 = 72.0;

fn reactions(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let counts = reaction_chips(&message.reactions, |reaction| {
        if reaction.from_me {
            tr("You").to_owned()
        } else {
            (view.names_or)(&reaction.sender, None)
        }
    });
    // The chips draw no background, so their padding is the gap between
    // reactions: kept narrow, with the first one still where it was.
    ui.spacing_mut().item_spacing.x = 2.0;
    ui.add_space(4.0);
    for (emoji, count, mine, names) in counts {
        let label = if count > 1 {
            format!("{emoji} {count}")
        } else {
            emoji.clone()
        };
        let line = widgets::line(ui, &label, theme::regular(13.0), palette.text, 200.0, 1);
        let size = line.size() + vec2(4.0, 6.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
        }
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(names.join(", "));
        if response.clicked() {
            // Clicking our reaction removes it; clicking another adds it.
            actions.push(Action::React {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
                emoji: if mine { String::new() } else { emoji },
            });
        }
    }
}

pub(crate) const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// Our existing reaction to a message.
pub(crate) fn own_reaction(message: &Message) -> Option<&str> {
    message
        .reactions
        .iter()
        .find(|reaction| reaction.from_me)
        .map(|reaction| reaction.emoji.as_str())
}

/// Emoji to send for a reaction choice: empty string removes our current one.
pub(crate) fn reaction_choice(current: Option<&str>, emoji: &str) -> String {
    if current == Some(emoji) {
        String::new()
    } else {
        emoji.to_owned()
    }
}

/// Quick reactions plus our current reaction when needed.
fn quick_reactions<'a>(message: &'a Message, preferred: &'a [(String, u32)]) -> Vec<&'a str> {
    let mut list = Vec::new();
    for emoji in preferred
        .iter()
        .map(|(emoji, _)| emoji.as_str())
        .chain(QUICK_REACTIONS.iter().copied())
    {
        if emojis::get(emoji).is_some() && !list.contains(&emoji) {
            list.push(emoji);
        }
        if list.len() == QUICK_REACTIONS.len() {
            break;
        }
    }
    if let Some(mine) = own_reaction(message)
        && !list.contains(&mine)
    {
        list.push(mine);
    }
    list
}

fn context_menu(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let chat = &view.chat.id;
    let mine = own_reaction(message);
    // On one of several selected messages, the menu acts on all of them.
    let group: Vec<String> = if view.selection.len() > 1 && view.selection.contains(&message.id) {
        view.selection.to_vec()
    } else {
        vec![message.id.clone()]
    };
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for emoji in quick_reactions(message, view.reaction_emoji) {
                let chosen = mine == Some(emoji);
                let line = widgets::line(ui, emoji, theme::regular(20.0), palette.text, 40.0, 1);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                theme::focus_outline(ui, response.id, rect, 17.0);
                if chosen {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_active);
                    ui.painter().circle_stroke(
                        rect.center(),
                        16.0,
                        Stroke::new(1.5, palette.accent),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_hover);
                }
                line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                let response = if chosen {
                    response.on_hover_text(tr("Remove your reaction"))
                } else {
                    response
                };
                if response.clicked() {
                    // Selecting our current reaction removes it.
                    actions.push(Action::React {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        emoji: reaction_choice(mine, emoji),
                    });
                    ui.close();
                }
            }
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            theme::focus_outline(ui, response.id, rect, 17.0);
            if ui.is_rect_visible(rect) {
                let hovered = response.hovered();
                ui.painter().circle_filled(
                    rect.center(),
                    17.0,
                    if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    },
                );
                ui.painter()
                    .circle_stroke(rect.center(), 16.0, Stroke::new(1.0, palette.outline));
                theme::paint_icon(ui, Icon::Plus, rect, 16.0, palette.secondary);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tr("React with any emoji"))
                .clicked()
            {
                actions.push(Action::OpenReactionPicker {
                    chat: chat.clone(),
                    message: message.id.clone(),
                    beside_menu: true,
                });
            }
        },
    );
    widgets::menu_separator(ui, &palette);
    if media_unsent(view, message)
        && widgets::menu_item(ui, &palette, Some(Icon::Refresh), tr("Send again"))
    {
        actions.push(Action::RetryMedia {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    if !matches!(message.content, Content::Revoked { .. })
        && widgets::menu_item(ui, &palette, Some(Icon::Reply), tr("Reply"))
    {
        actions.push(Action::Reply(message.id.clone()));
    }
    if !matches!(
        message.content,
        Content::Revoked { .. }
            | Content::Unsupported { .. }
            | Content::PhoneOnly { .. }
            | Content::Poll { .. }
            | Content::Interactive { .. }
    ) && widgets::menu_item(ui, &palette, Some(Icon::Forward), tr("Forward"))
    {
        actions.push(Action::ShowDialog(Dialog::Forward {
            chat: chat.clone(),
            messages: group.clone(),
        }));
    }
    if widgets::menu_item(ui, &palette, Some(Icon::Check), tr("Select")) {
        actions.push(Action::SelectMessage(message.id.clone()));
    }
    let text = copyable_text(&message.content);
    if let Some(text) = text
        && widgets::menu_item(ui, &palette, Some(Icon::Copy), tr("Copy text"))
    {
        let mentions = mentions_of(view, message);
        actions.push(Action::CopyText(markup::plain(&text, &mentions)));
    }
    if let Content::Audio { media, .. } = &message.content
        && let Some(path) = media
            .path
            .as_ref()
            .filter(|_| !media_sending(view, message))
    {
        let key = (chat.clone(), message.id.clone());
        match view.transcripts.get(&key) {
            Some(transcript) => {
                if view.transcripts_folded.contains(&key)
                    && widgets::menu_item(ui, &palette, Some(Icon::Captions), tr("Show transcript"))
                {
                    actions.push(Action::Transcribe {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        path: path.clone(),
                    });
                }
                if widgets::menu_item(ui, &palette, Some(Icon::Copy), tr("Copy transcript")) {
                    actions.push(Action::CopyText(transcript.text.clone()));
                }
            }
            None => {
                if view.transcriber.progress(&key).is_none()
                    && widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::Captions),
                        tr("Transcribe audio"),
                    )
                {
                    actions.push(Action::Transcribe {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        path: path.clone(),
                    });
                }
            }
        }
    }
    let age = view.now - message.timestamp;
    let can_edit = message.from_me
        && message.content.editable_text().is_some()
        && age <= crate::app::EDIT_WINDOW.as_secs() as i64;
    let can_revoke = if group.len() > 1 {
        view.selection_revocable
    } else {
        message.from_me
            && !matches!(message.content, Content::Revoked { .. })
            && age <= crate::app::REVOKE_WINDOW.as_secs() as i64
    };
    if can_edit && widgets::menu_item(ui, &palette, Some(Icon::Pencil), tr("Edit")) {
        actions.push(Action::Edit(message.id.clone()));
    }
    if can_revoke && widgets::menu_item(ui, &palette, Some(Icon::Trash), tr("Delete for everyone"))
    {
        actions.push(Action::ShowDialog(Dialog::ConfirmDeleteMessage {
            chat: view.chat.id.clone(),
            messages: group.clone(),
            for_everyone: true,
            on_phone: false,
        }));
    }
    if widgets::menu_item(ui, &palette, Some(Icon::EyeOff), tr("Delete for me")) {
        actions.push(Action::ShowDialog(Dialog::ConfirmDeleteMessage {
            chat: view.chat.id.clone(),
            messages: group.clone(),
            for_everyone: false,
            on_phone: true,
        }));
    }
    if let Content::Sticker { media, .. } = &message.content
        && let Some(path) = &media.path
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Star),
            &crate::i18n::gettext(view.locale, "Add to favorites"),
        )
    {
        actions.push(Action::SaveSticker(path.clone()));
    }
    if let Some(media) = message.content.media() {
        match &media.path {
            Some(path) => {
                let open = if matches!(message.content, Content::Video { gif: false, .. }) {
                    crate::i18n::gettext(view.locale, "Open in system player")
                } else {
                    tr("Open file").into()
                };
                if widgets::menu_item(ui, &palette, Some(Icon::ExternalLink), &open) {
                    actions.push(Action::OpenFile(path.clone()));
                }
                if matches!(message.content, Content::Image { .. })
                    && widgets::menu_item(ui, &palette, Some(Icon::Copy), tr("Copy image"))
                {
                    actions.push(Action::CopyImage(path.clone()));
                }
                if widgets::menu_item(ui, &palette, Some(Icon::Download), tr("Save as…")) {
                    actions.push(if group.len() > 1 {
                        Action::SaveSelection {
                            chat: chat.clone(),
                            messages: group.clone(),
                        }
                    } else {
                        Action::SaveAttachmentAs {
                            path: path.clone(),
                            name: attachment_name(&message.content, path),
                        }
                    });
                }
                if let Some(folder) = path.parent()
                    && widgets::menu_item(ui, &palette, Some(Icon::FileText), tr("Show in folder"))
                {
                    actions.push(Action::OpenFolder(folder.to_path_buf()));
                }
            }
            None => {
                let downloading = matches!(media.state, MediaState::Downloading);
                if widgets::menu_item_enabled(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    if downloading {
                        tr("Downloading…")
                    } else {
                        tr("Download")
                    },
                    !downloading,
                ) {
                    actions.push(Action::Download {
                        card: None,
                        chat: chat.clone(),
                        message: message.id.clone(),
                    });
                }
                // Among several selected messages, saving still takes them
                // all, downloading what is missing first.
                if group.len() > 1
                    && widgets::menu_item(ui, &palette, Some(Icon::Download), tr("Save as…"))
                {
                    actions.push(Action::SaveSelection {
                        chat: chat.clone(),
                        messages: group.clone(),
                    });
                }
            }
        }
    }
    if let Content::Audio { media, .. } = &message.content
        && media.path.is_some()
    {
        speed_menu_row(ui, view, message, actions);
    }
    widgets::menu_separator(ui, &palette);
    // The menu holds actions only. Sent, delivery, and read times, per member
    // in a group, live in "Message info".
    if message.from_me
        && !matches!(message.content, Content::Revoked { .. })
        && !matches!(
            message.status,
            Delivery::None | Delivery::Pending | Delivery::Failed
        )
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(view.locale, "Message info"),
        )
    {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: chat.clone(),
            message: message.id.clone(),
        }));
    }
    if widgets::menu_item_enabled(
        ui,
        &palette,
        Some(Icon::Refresh),
        &crate::i18n::gettext(view.locale, "Reload earlier messages"),
        view.connected && !matches!(message.status, Delivery::Pending | Delivery::Failed),
    ) {
        actions.push(Action::ReloadHistory {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    // The id helps when looking a message up for a bug report.
    if widgets::menu_item(ui, &palette, Some(Icon::Copy), tr("Copy message ID")) {
        actions.push(Action::CopyText(message.id.clone()));
    }
}

fn mentions_of(view: &View<'_>, message: &Message) -> Vec<markup::Mention> {
    message
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

fn quote_mentions(view: &View<'_>, quoted: &crate::model::Quoted) -> Vec<markup::Mention> {
    quoted
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

fn vcard_tel_preference(property: &str) -> Option<u8> {
    let mut best = None;
    for parameter in property.split(';').skip(1) {
        let (name, value) = parameter.split_once('=').unwrap_or(("TYPE", parameter));
        let value = value.trim_matches('"');
        let rank = if name.eq_ignore_ascii_case("PREF") {
            value
                .parse::<u8>()
                .ok()
                .filter(|rank| (1..=100).contains(rank))
        } else if name.eq_ignore_ascii_case("TYPE")
            && value
                .split(',')
                .any(|value| value.eq_ignore_ascii_case("PREF"))
        {
            Some(1)
        } else {
            None
        };
        if let Some(rank) = rank {
            best = Some(best.map_or(rank, |current: u8| current.min(rank)));
        }
    }
    best
}

/// The first card of a shared contact, as the bubble uses it.
#[derive(Debug, PartialEq)]
struct SharedContact {
    name: String,
    /// The telephone number as the card writes it.
    number: String,
    /// The WhatsApp account behind the number, in digits: the `waid`
    /// parameter WhatsApp adds, or a number written in international form.
    /// A local number without a country code cannot name one.
    account: Option<String>,
    /// The first name from the card's structured `N` property, middle names
    /// included, for the contact editor; none when the card has none.
    first_name: Option<String>,
}

/// The first name in a vCard `N` value
/// (`family;given;additional;prefix;suffix`), middle names included.
/// Components are split at unescaped semicolons only.
fn vcard_first_name(value: &str) -> Option<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.next() {
                Some('n' | 'N') => parts.last_mut()?.push(' '),
                Some(escaped) => parts.last_mut()?.push(escaped),
                None => {}
            },
            ';' => parts.push(String::new()),
            _ => parts.last_mut()?.push(character),
        }
    }
    let part = |index: usize| parts.get(index).map_or("", |part| part.trim());
    let first = [part(1), part(2)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!first.is_empty()).then_some(first)
}

fn vcard_tel_account(property: &str, value: &str) -> Option<String> {
    let digits = |text: &str| {
        text.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    };
    let waid = property.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.eq_ignore_ascii_case("WAID")
            .then(|| digits(value.trim_matches('"')))
    });
    waid.or_else(|| value.trim().starts_with('+').then(|| digits(value)))
        .filter(|account| account.len() >= 7)
}

fn shared_contact_details(vcard: &str, fallback_name: &str) -> Option<SharedContact> {
    let mut name = None;
    let mut first_name = None;
    let mut first_phone = None;
    let mut preferred_phone: Option<(u8, (String, Option<String>))> = None;
    let mut first_card: Vec<String> = Vec::new();
    for line in vcard.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(previous) = first_card.last_mut() {
                previous.push_str(line.trim_start());
            }
            continue;
        }
        if line.eq_ignore_ascii_case("END:VCARD") && !first_card.is_empty() {
            break;
        }
        if line.eq_ignore_ascii_case("BEGIN:VCARD") {
            if first_card.is_empty() {
                first_card.push(line.to_owned());
            }
            continue;
        }
        if !first_card.is_empty() {
            first_card.push(line.to_owned());
        }
    }
    if first_card.is_empty() {
        first_card.extend(vcard.lines().map(str::to_owned));
    }
    for line in first_card {
        let Some((property, value)) = line.split_once(':') else {
            continue;
        };
        let raw_name = property.split(';').next().unwrap_or(property);
        let property_name = raw_name.rsplit('.').next().unwrap_or(raw_name);
        if property_name.eq_ignore_ascii_case("FN") {
            name = Some(value.trim().to_owned());
        } else if property_name.eq_ignore_ascii_case("N") {
            first_name = vcard_first_name(value);
        } else if property_name.eq_ignore_ascii_case("TEL") {
            let number = value.trim().to_owned();
            if number.chars().filter(char::is_ascii_digit).count() < 7 {
                continue;
            }
            let phone = (number, vcard_tel_account(property, value));
            if let Some(rank) = vcard_tel_preference(property) {
                let replace = preferred_phone
                    .as_ref()
                    .is_none_or(|(best_rank, _)| rank < *best_rank);
                if replace {
                    preferred_phone = Some((rank, phone));
                }
            } else if first_phone.is_none() {
                first_phone = Some(phone);
            }
        }
    }
    let (number, account) = preferred_phone.map(|(_, phone)| phone).or(first_phone)?;
    let name = name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name.to_owned());
    Some(SharedContact {
        name,
        number,
        account,
        first_name,
    })
}

/// Draws a message body and returns optional footer space on its last line.
fn content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let own = message.from_me;
    // Add non-text messages to cross-message transcript copies.
    let has_body = match &message.content {
        Content::Text { .. } => true,
        // Image-only cards and carousels without a body draw no text.
        Content::Interactive { card, .. } => card.as_ref().is_none_or(|card| {
            !card.body.is_empty() || (card.image.is_none() && card.carousel.is_empty())
        }),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.is_some(),
        Content::Revoked { kept } => kept.is_some(),
        _ => false,
    };
    if !has_body {
        view.copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(view, message, String::new(), Vec::new()));
    }
    match &message.content {
        Content::Interactive { text, card } => {
            let Some(card) = card else {
                let span = message.quoted.is_some().then_some(width);
                return rich_body(ui, view, message, text, width, Some(reserve), span, actions);
            };
            if !card.carousel.is_empty() {
                carousel(ui, view, message, card, width, actions);
                return None;
            }
            if let Some(image) = &card.image {
                picture(ui, view, message, image, width, None, actions);
                ui.add_space(4.0);
                if card.body.is_empty() {
                    return None;
                }
            }
            let body = if card.body.is_empty() {
                tr("Interactive message")
            } else {
                &card.body
            };
            rich_body(
                ui,
                view,
                message,
                body,
                width,
                Some(reserve),
                Some(width),
                actions,
            )
        }
        Content::Text { text, preview } => {
            if let Some(preview) = preview {
                preview_card(ui, view, message, preview, width, actions);
            }
            let span = (message.quoted.is_some() || preview.is_some()).then_some(width);
            rich_body(ui, view, message, text, width, Some(reserve), span, actions)
        }
        Content::Image { caption, media } => {
            let drawn = picture(ui, view, message, media, width, None, actions);
            ui.ctx().data_mut(|data| {
                data.insert_temp(bubble_id(&view.chat.id, &message.id).with("picture"), drawn);
            });
            let Some(caption) = caption else {
                // The time and ticks go over the picture's corner.
                return Some(drawn);
            };
            let drawn = drawn.width();
            {
                // Wrap the caption to the settled image or quote width.
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            }
        }
        Content::Sticker { media, animated } => {
            picture(ui, view, message, media, width, Some(*animated), actions);
            None
        }
        Content::Video {
            caption,
            media,
            seconds,
            gif,
            note,
        } => {
            let drawn = if *note {
                video_note(ui, view, message, media, *seconds, actions)
            } else {
                video(ui, view, message, media, *seconds, *gif, width, actions)
            };
            caption.as_ref().and_then(|caption| {
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            })
        }
        Content::Audio {
            media,
            seconds,
            waveform,
            ..
        } => voice_player(ui, view, message, media, *seconds, waveform, width, actions),
        Content::Document {
            media,
            file_name,
            caption,
            pages,
        } => {
            let mut detail = Vec::new();
            if let Some(pages) = pages {
                detail.push(
                    crate::i18n::ngettext(crate::i18n::current(), "{} page", "{} pages", *pages)
                        .replace("{}", &pages.to_string()),
                );
            }
            detail.push(crate::util::bytes(media.size));
            attachment(
                ui,
                view,
                message,
                media,
                file_name,
                &detail.join(" · "),
                width,
                actions,
            );
            caption.as_ref().and_then(|caption| {
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    width,
                    Some(reserve),
                    Some(width),
                    actions,
                )
            })
        }
        Content::Location {
            latitude,
            longitude,
            name,
            address,
        } => {
            let title = name
                .clone()
                .unwrap_or_else(|| crate::i18n::gettext(view.locale, "Location").into_owned());
            location_card(
                ui,
                view,
                message,
                &message.id,
                &title,
                address.clone(),
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::LiveLocation {
            latitude,
            longitude,
            accuracy_m,
            speed_mps,
            sequence,
            updated,
            ..
        } => {
            let over = message
                .content
                .live_location_over(message.timestamp, view.now);
            let title = if over {
                crate::i18n::gettext(view.locale, "Live location ended")
            } else {
                crate::i18n::gettext(view.locale, "Live location")
            };
            let detail = (!over).then(|| {
                // An absolute time stays true without repainting.
                let at = if *updated > 0 {
                    *updated
                } else {
                    message.timestamp
                };
                let mut meta = vec![
                    crate::i18n::gettext(view.locale, "Updated {time}")
                        .replace("{time}", &crate::util::clock(at)),
                ];
                if let Some(speed) = speed_mps.filter(|speed| *speed >= 0.5) {
                    meta.push(format!("{:.0} km/h", speed * 3.6));
                }
                if let Some(accuracy) = accuracy_m {
                    meta.push(format!("±{accuracy} m"));
                }
                meta.join(" · ")
            });
            // Each position gets its own preview, as a later one replaces the first.
            let key = format!("{}-{sequence}-{updated}", message.id);
            location_card(
                ui,
                view,
                message,
                &key,
                &title,
                detail,
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::Contact {
            display_name,
            vcard,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Contact, 18.0, palette.accent);
            };
            let details = shared_contact_details(vcard, display_name);
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let name = details
                        .as_ref()
                        .map_or(display_name.as_str(), |contact| contact.name.as_str());
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    match details.as_ref() {
                        Some(SharedContact {
                            account: Some(account),
                            first_name,
                            ..
                        }) => {
                            let id = format!("{account}@s.whatsapp.net");
                            ui.horizontal(|ui| {
                                if theme::pill_button(
                                    ui,
                                    &palette,
                                    crate::i18n::gettext(view.locale, "Chat").as_ref(),
                                    true,
                                )
                                .clicked()
                                {
                                    actions.push(Action::StartChat {
                                        id: id.clone(),
                                        name: name.to_owned(),
                                    });
                                }
                                if !view.contacts.contains_key(&id)
                                    && theme::pill_button(
                                        ui,
                                        &palette,
                                        crate::i18n::gettext(view.locale, "Add").as_ref(),
                                        false,
                                    )
                                    .clicked()
                                {
                                    // Split where the card's first name ends, so a
                                    // first name of several words stays whole.
                                    let (first, last) =
                                        crate::util::editor_names(name, first_name.as_deref());
                                    actions.push(Action::NewContact {
                                        phone: account.clone(),
                                        first,
                                        last,
                                        to_phone: None,
                                    });
                                }
                            });
                        }
                        // Without an account there is nothing to open, so
                        // the number stays readable, as it was.
                        Some(contact) => {
                            theme::text(
                                ui,
                                &contact.number,
                                theme::regular(12.5),
                                palette.secondary,
                            );
                        }
                        None => {}
                    }
                });
            });
            None
        }
        Content::StickerPack {
            name,
            publisher,
            count,
            caption,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Sticker, 20.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    let stickers =
                        crate::i18n::ngettext(view.locale, "{} sticker", "{} stickers", *count)
                            .replace("{}", &count.to_string());
                    let detail = if publisher.trim().is_empty() {
                        stickers
                    } else {
                        format!("{publisher} · {stickers}")
                    };
                    widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
                    if let Some(caption) = caption {
                        widgets::rich_text(ui, caption, theme::regular(13.5), palette.text);
                    }
                    if theme::link(
                        ui,
                        crate::i18n::gettext(view.locale, "View stickers"),
                        theme::regular(12.5),
                        palette.link,
                    )
                    .clicked()
                    {
                        actions.push(Action::ViewStickerPack(message.id.clone()));
                    }
                });
            });
            None
        }
        Content::Poll { .. } => {
            super::polls::ballot(
                ui,
                &palette,
                message,
                width,
                view.connected,
                view.poll_voting
                    .contains(&(message.chat.clone(), message.id.clone())),
                actions,
            );
            None
        }
        Content::Revoked { kept } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Ban, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        tr("This message was deleted"),
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            let kept = kept.as_deref()?;
            let mut original = message.clone();
            original.content = kept.clone();
            content(ui, view, &original, width, reserve, actions)
        }
        Content::PhoneOnly {
            live_location: true,
            ..
        } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::MapPin, 18.0, palette.accent);
                },
                |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(view.locale, "Live location"),
                            theme::medium(14.0),
                            palette.text,
                        );
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(
                                view.locale,
                                "Open WhatsApp on your phone to follow it.",
                            ),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    });
                },
            );
            None
        }
        Content::PhoneOnly {
            view_once, once, ..
        } => {
            use crate::model::OnceMedia;
            let text = match once {
                Some(OnceMedia::Photo) => {
                    tr("View once photo. For your privacy, it opens only on your phone.")
                }
                Some(OnceMedia::Video) => {
                    tr("View once video. For your privacy, it opens only on your phone.")
                }
                Some(OnceMedia::Voice) => {
                    tr("View once voice message. For your privacy, it opens only on your phone.")
                }
                Some(OnceMedia::Audio) => {
                    tr("View once audio. For your privacy, it opens only on your phone.")
                }
                None if *view_once => {
                    tr("View once message. For your privacy, it opens only on your phone.")
                }
                None => tr("This message can only be seen on your phone."),
            };
            // The audio box's width, so the notice wraps inside it instead of
            // running out to the cap and being cut.
            ui.scope(|ui| {
                ui.set_width(width);
                mirrored_row(
                    ui,
                    own,
                    |ui| {
                        theme::icon(ui, Icon::Smartphone, 14.0, palette.dim);
                    },
                    |ui| {
                        theme::paragraph(ui, text, theme::regular(13.5), palette.secondary);
                    },
                );
            });
            None
        }
        Content::Unsupported { what } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::CircleAlert, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        tr("Unsupported: {what}")
                            .replace("{what}", &crate::model::unsupported_kind(what)),
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
    }
}

/// Horizontal, independently cached cards with overlaid previous/next controls.
/// Keep a partial next card visible and preserve native wheel/touchpad scrolling.
fn carousel(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    if !card.body.is_empty() {
        rich_body(
            ui,
            view,
            message,
            &card.body,
            width,
            None,
            Some(width),
            actions,
        );
        ui.add_space(4.0);
    }
    // Reserve a glimpse of the next card only when there is another card.
    let inset = if card.carousel.len() > 1 { 36.0 } else { 20.0 };
    let card_width = CAROUSEL_CARD_WIDTH.min((width - inset).max(140.0));
    let id = bubble_id(&message.chat, &message.id);
    for direction in [-1, 1] {
        ui.ctx().data_mut(|data| {
            data.remove::<Rect>(id.with(("carousel-arrow", direction)));
        });
    }
    let output = egui::ScrollArea::horizontal()
        .id_salt(("carousel", &message.chat, &message.id))
        .max_width(width)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show_viewport(ui, |ui, viewport| {
            let strip = ui
                .with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = CAROUSEL_GAP;
                    for (index, child) in card.carousel.iter().enumerate() {
                        let row = carousel_row(message, index, child);
                        ui.push_id(("carousel-card", index), |ui| {
                            let response = Frame::new()
                                .fill(if message.from_me {
                                    view.palette.bubble_out
                                } else {
                                    view.palette.bubble_in
                                })
                                .corner_radius(10)
                                .inner_margin(Margin {
                                    left: 10,
                                    right: 10,
                                    top: 6,
                                    bottom: 5,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(card_width);
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.set_width(card_width);
                                        if child.image.is_some() {
                                            carousel_picture(
                                                ui, view, message, index, child, card_width,
                                                actions,
                                            );
                                        }
                                        if !child.body.is_empty() {
                                            rich_body(
                                                ui,
                                                view,
                                                &row,
                                                &child.body,
                                                card_width,
                                                None,
                                                Some(card_width),
                                                actions,
                                            );
                                        }
                                        interactive_buttons(
                                            ui, view, &row, child, card_width, actions,
                                        );
                                    });
                                });
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    bubble_id(&message.chat, &message.id)
                                        .with(("carousel-card", index)),
                                    response.response.rect,
                                )
                            });
                        });
                    }
                })
                .response
                .rect;
            let limit = (strip.width() - viewport.width()).max(0.0);
            let visible = Rect::from_min_size(
                pos2(strip.left() + viewport.left(), strip.top()),
                vec2(viewport.width(), strip.height()),
            );
            if limit > 1.0 {
                for direction in [-1, 1] {
                    let available = if direction < 0 {
                        viewport.left() > 1.0
                    } else {
                        viewport.left() < limit - 1.0
                    };
                    if !available {
                        continue;
                    }
                    let x = if direction < 0 {
                        visible.left() + 26.0
                    } else {
                        visible.right() - 26.0
                    };
                    let rect =
                        Rect::from_center_size(pos2(x, visible.center().y), Vec2::splat(40.0));
                    let arrow = carousel_arrow(
                        ui,
                        &view.palette,
                        rect,
                        id.with(("carousel-arrow", direction)),
                        direction < 0,
                    );
                    if arrow.clicked() {
                        let step = card_width + 20.0 + CAROUSEL_GAP;
                        let target = (viewport.left() + direction as f32 * step).clamp(0.0, limit);
                        // Issue this inside the horizontal ScrollArea so the
                        // conversation's vertical scroll cannot consume it.
                        ui.scroll_with_delta(vec2(viewport.left() - target, 0.0));
                    }
                }
            }
            visible
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("carousel-viewport"), output.inner));
}

/// Stands in for one carousel card when drawing its text and actions. Only the
/// parent's identity is kept: cloning the whole message would copy every
/// card's thumbnail per card each frame, and the parent's quote and reactions
/// would repeat on each card in copied transcripts.
fn carousel_row(message: &Message, index: usize, card: &crate::model::InteractiveCard) -> Message {
    Message {
        id: format!("{}-card-{index}", message.id),
        chat: message.chat.clone(),
        sender: message.sender.clone(),
        sender_name: message.sender_name.clone(),
        from_me: message.from_me,
        timestamp: message.timestamp,
        history_order: message.history_order,
        content: Content::Interactive {
            text: card.body.clone(),
            card: None,
        },
        status: message.status,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: message.edited,
        mentions: message.mentions.clone(),
        forwarded: false,
        thumbnail: None,
    }
}

fn carousel_arrow(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    previous: bool,
) -> egui::Response {
    let label = if previous {
        tr("Previous card")
    } else {
        tr("Next card")
    };
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.ctx().data_mut(|data| data.insert_temp(id, rect));
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() || response.has_focus() {
            palette.surface_hover
        } else {
            palette.overlay
        };
        ui.painter()
            .circle_filled(rect.center() + vec2(0.0, 2.0), 21.0, palette.shadow);
        ui.painter().circle_filled(rect.center(), 20.0, fill);
        ui.painter().circle_stroke(
            rect.center(),
            19.5,
            Stroke::new(
                1.0,
                if response.has_focus() {
                    palette.link
                } else {
                    palette.outline
                },
            ),
        );
        theme::paint_icon(
            ui,
            if previous {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            },
            rect,
            if response.is_pointer_button_down_on() {
                21.0
            } else {
                23.0
            },
            palette.text,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// A carousel uses a short image preview; opening it shows the full picture.
fn carousel_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    index: usize,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Some(media) = &card.image else { return };
    let size = vec2(width, width * 0.56);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = match (&media.path, &media.state) {
        (Some(_), _) => tr("Open card image"),
        (None, MediaState::Failed(_)) => tr("Retry card image download"),
        (None, MediaState::Downloading) => tr("Downloading card image"),
        (None, MediaState::Idle) => tr("Download card image"),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    // Tab can land on a card scrolled out of the carousel's viewport.
    theme::reveal_focus(&response);
    let visible = ui.is_rect_visible(rect);
    if visible {
        ui.painter().rect_filled(rect, 6.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                card.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-card-{index}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(16) as f32,
                    media.height.unwrap_or(9) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv_size = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv_size))
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
            } else if media.path.is_some() {
                theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, view.palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 24.0),
                    Align2::CENTER_CENTER,
                    tr("Could not display this picture. Click to open it."),
                    theme::regular(11.5),
                    view.palette.secondary,
                );
            }
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match &media.state {
                MediaState::Downloading => theme::paint_spinner(ui, disc, 20.0, Color32::WHITE),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        tr("Download failed. Click to retry."),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(theme::FOCUS_STROKE_WIDTH, view.palette.link),
                egui::StrokeKind::Inside,
            );
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · {}", tr("Click to retry")));
    }
    if let Some(path) = &media.path {
        if clicked {
            actions.push(Action::OpenFile(path.clone()));
        }
    } else if !matches!(media.state, MediaState::Downloading)
        && (clicked
            || (visible
                && auto_download_allowed(
                    media,
                    crate::settings::AutoDownloadKind::Image,
                    view.download_settings,
                )
                && matches!(media.state, MediaState::Idle)))
    {
        actions.push(Action::Download {
            chat: message.chat.clone(),
            message: message.id.clone(),
            card: Some(index),
        });
    }
}

/// Full-width action rows below the message timestamp, matching business cards.
/// Show only executable actions as active, with the same keyboard path as clicks.
fn interactive_buttons(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::model::InteractiveAction;
    let palette = &view.palette;
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, button) in card.buttons.iter().enumerate() {
        let sends = matches!(
            button.action,
            InteractiveAction::Reply | InteractiveAction::Select(_)
        );
        let enabled = button.url.is_some()
            || match &button.action {
                InteractiveAction::Copy(_) => true,
                InteractiveAction::Reply | InteractiveAction::Select(_) => {
                    view.connected
                        && view.chat.can_send()
                        && !message.from_me
                        && !message.edited
                        && !view
                            .interactive_pending
                            .contains(&(message.chat.clone(), message.id.clone()))
                }
                InteractiveAction::Unavailable => false,
            };
        // Muted/link colours target incoming surfaces; outgoing tinted bubbles
        // need the main foreground to keep small labels readable in both themes.
        let color = if message.from_me {
            palette.text
        } else if enabled {
            palette.link
        } else {
            palette.secondary
        };
        let icon = if button.url.is_some() {
            Some(Icon::ExternalLink)
        } else {
            match button.action {
                InteractiveAction::Reply => None,
                InteractiveAction::Select(_) => Some(Icon::ListChecks),
                InteractiveAction::Copy(_) => Some(Icon::Copy),
                InteractiveAction::Unavailable => Some(Icon::Smartphone),
            }
        };
        let line = widgets::line(
            ui,
            &button.label,
            theme::medium(14.0),
            color,
            (width - 48.0).max(1.0),
            2,
        );
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, (line.size().y + 22.0).max(44.0)),
            Sense::hover(),
        );
        // Action rows belong to the card itself, including its side padding.
        // The final row follows the bubble's bottom corners and margin.
        let last = index + 1 == card.buttons.len() && !card.needs_phone;
        let row_rect = Rect::from_min_max(
            rect.min - vec2(10.0, 0.0),
            rect.max + vec2(10.0, if last { 5.0 } else { 0.0 }),
        );
        let corners = CornerRadius {
            sw: if last { 10 } else { 0 },
            se: if last { 10 } else { 0 },
            ..CornerRadius::ZERO
        };
        let response = ui.interact(
            row_rect,
            bubble_id(&message.chat, &message.id).with(("interactive-action", index)),
            sense,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &button.label)
        });
        theme::reveal_focus(&response);
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                bubble_id(&message.chat, &message.id).with(("interactive-button", index)),
                row_rect,
            )
        });
        if ui.is_rect_visible(rect) {
            if enabled && (response.hovered() || response.has_focus()) {
                ui.painter().rect_filled(
                    row_rect,
                    corners,
                    palette
                        .text
                        .gamma_multiply(if response.is_pointer_button_down_on() {
                            0.08
                        } else {
                            0.04
                        }),
                );
            }
            ui.painter().hline(
                row_rect.x_range(),
                rect.top(),
                Stroke::new(1.0, palette.secondary.gamma_multiply(0.2)),
            );
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    4.0,
                    Stroke::new(1.0, palette.link),
                    egui::StrokeKind::Inside,
                );
            }
            let inset = if icon.is_some() { 22.0 } else { 0.0 };
            let content_width = line.size().x + inset;
            let left = rect.center().x - content_width / 2.0;
            if let Some(icon) = icon {
                theme::paint_icon(
                    ui,
                    icon,
                    Rect::from_center_size(
                        egui::pos2(left + 7.0, rect.center().y),
                        Vec2::splat(14.0),
                    ),
                    14.0,
                    color,
                );
            }
            line.paint(
                ui,
                egui::pos2(left + inset, rect.center().y - line.size().y / 2.0),
                color,
            );
        }
        let reply = |choice| Action::ReplyInteractive {
            chat: message.chat.clone(),
            message: message.id.clone(),
            button: index,
            choice,
        };
        if !enabled {
            let reason = if sends
                && view
                    .interactive_pending
                    .contains(&(message.chat.clone(), message.id.clone()))
            {
                tr("Sending reply…")
            } else if sends && !view.connected {
                tr("Connect to WhatsApp to reply")
            } else if sends && !view.chat.can_send() {
                tr("This conversation is read-only")
            } else if sends && message.from_me {
                tr("Reply options are for the recipient")
            } else {
                tr("Open this option in WhatsApp Web or on your phone")
            };
            response.on_hover_text(format!("{}\n{reason}", button.label));
        } else if let Some(url) = &button.url {
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{url}", button.label))
                .clicked()
            {
                actions.push(Action::OpenUrl(url.clone()));
            }
        } else {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            match &button.action {
                InteractiveAction::Reply => {
                    if response
                        .on_hover_text(tr("Send reply: {label}").replace("{label}", &button.label))
                        .clicked()
                    {
                        actions.push(reply(None));
                    }
                }
                InteractiveAction::Copy(code) => {
                    if response.on_hover_text(tr("Copy code")).clicked() {
                        actions.push(Action::CopyText(code.clone()));
                    }
                }
                InteractiveAction::Select(_) => {
                    if response.clicked() {
                        actions.push(Action::ShowDialog(crate::model::Dialog::InteractiveList {
                            chat: message.chat.clone(),
                            message: message.id.clone(),
                            button: index,
                        }));
                    }
                }
                InteractiveAction::Unavailable => {}
            }
        }
    }
    if card.needs_phone {
        ui.add_space(6.0);
        widgets::rich_text(
            ui,
            tr("More content in WhatsApp Web or on your phone"),
            theme::regular(12.0),
            palette.secondary,
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Draws formatted message text. `reserve` leaves footer space on the last
/// line. `span` sets a minimum left-aligned row width for text below cards.
#[allow(clippy::too_many_arguments)]
fn rich_body(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    text: &str,
    width: f32,
    reserve: Option<f32>,
    span: Option<f32>,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let mentions = mentions_of(view, message);
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions, &style, width);
    let rtl = crate::bidi::message_rtl(text);
    let last_row = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    // Right-aligned text ends at the block's edge. Like official WhatsApp,
    // only a single line keeps the time beside it; otherwise it gets a row.
    let single = laid.galley.rows.len() == 1 && span.is_none();
    let inline = reserve.filter(|reserve| last_row + 8.0 + reserve <= width && (!rtl || single));
    let size = laid.galley.size();
    let mut allocation = match inline {
        Some(reserve) => vec2(size.x.max(last_row + 8.0 + reserve), size.y),
        None => size,
    };
    if let Some(span) = span {
        // Span the card width and keep the text left-aligned in own bubbles.
        allocation.x = allocation.x.max(span);
    }
    // Register the body for transcript formatting when copying across messages.
    view.copy_rows
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(transcript_row(
            view,
            message,
            laid.galley.text().to_owned(),
            laid.placements().to_vec(),
        ));
    // Click links and drag to select text.
    // Text selection and pointer links do not need a sequential Tab stop.
    // The surrounding transcript remains available to accessibility readers.
    let (_, rect) = ui.allocate_space(allocation);
    // egui matches selection endpoints to widgets by id every frame and drops
    // the selection when one is missed. A positional auto id shifts whenever
    // a sibling allocates differently (virtualized rows), killing the
    // selection mid-drag; an explicit id keeps the anchor alive.
    let response = ui.interact(
        rect,
        bubble_id(&view.chat.id, &message.id).with("body-text"),
        Sense::CLICK | Sense::DRAG,
    );
    // Store the body rect for selection tests.
    ui.ctx().data_mut(|data| {
        data.insert_temp(bubble_id(&view.chat.id, &message.id).with("body"), rect);
    });
    // Keep off-screen selected bodies registered so scrolling does not lose
    // the selection anchor or omit copied text.
    let visible = ui.is_rect_visible(rect);
    let selection_alive = ui.input(|input| input.pointer.primary_down())
        || ui
            .ctx()
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection());
    let origin = if rtl && inline.is_none() {
        pos2(rect.right() - size.x, rect.top())
    } else {
        rect.min
    };
    if visible || selection_alive {
        markup::paint_selectable(ui, &laid, &response, origin, palette.text, visible);
    }
    if !laid.links.is_empty()
        && let Some(pos) = response.hover_pos()
    {
        let cursor = laid.galley.cursor_from_pos(pos - origin);
        if let Some(url) = laid.link_at(cursor.index.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if response.clicked() {
                actions.push(Action::OpenUrl(url.to_owned()));
            }
        }
    }
    inline.map(|reserve| {
        Rect::from_min_max(
            pos2(rect.right() - reserve, rect.bottom() - 15.0),
            rect.right_bottom(),
        )
    })
}

/// Link preview with image, title, and description.
fn preview_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview: &LinkPreview,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let thumbnail = message.thumbnail.as_deref();
    let domain = domain_of(&preview.url);
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            // Include margins in the settled card width.
            let card_width = (width - 16.0).max(0.0);
            ui.set_width(card_width);
            // Limit text to the space beside the thumbnail and keep it
            // left-aligned in own bubbles.
            let column = (card_width - if thumbnail.is_some() { 72.0 } else { 0.0 }).max(0.0);
            ui.allocate_ui_with_layout(vec2(card_width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(card_width);
                ui.horizontal(|ui| {
                    if let Some(bytes) = thumbnail {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                        // Off-screen cards are laid out too; only a visible
                        // one keeps its thumbnail resident.
                        if ui.is_rect_visible(rect) {
                            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
                            egui::Image::new(uri)
                                .fit_to_exact_size(rect.size())
                                .corner_radius(4.0)
                                .paint_at(ui, rect);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.set_width(column);
                        if let Some(title) = &preview.title {
                            widgets::rich_text(ui, title, theme::semibold(13.5), palette.text);
                        }
                        if let Some(description) = &preview.description {
                            let line = widgets::line(
                                ui,
                                description,
                                theme::regular(12.5),
                                palette.secondary,
                                ui.available_width(),
                                2,
                            );
                            let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
                            line.paint(ui, rect.min, palette.secondary);
                        }
                        theme::text(ui, &domain, theme::regular(12.0), palette.dim);
                    });
                });
            });
        })
        .response;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("preview"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("preview", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::OpenUrl(preview.url.clone()));
    }
}

/// The host a link preview names under its title.
fn domain_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// A location as a card: the map preview WhatsApp sent across the top, then
/// a pinned title, an optional detail line, and a link to open the spot.
#[expect(clippy::too_many_arguments)]
fn location_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview_key: &str,
    title: &str,
    detail: Option<String>,
    (latitude, longitude): (f64, f64),
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    // A bounded, left-aligned layout: own bubbles inherit right-to-left flow,
    // which would otherwise stretch the card and make its width oscillate.
    let card = ui.available_width().min(LOCATION_CARD_WIDTH);
    ui.allocate_ui_with_layout(vec2(card, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(card);
        ui.spacing_mut().item_spacing.y = 2.0;
        // The preview already marks the spot. Crop the square preview to the
        // card's shape.
        if let Some(bytes) = message.thumbnail.as_deref()
            && !bytes.is_empty()
        {
            let uri = thumbnail_uri(ui.ctx(), &message.chat, preview_key, bytes);
            let height = (card * 0.56).round();
            let crop = (1.0 - height / card) / 2.0;
            ui.add(
                egui::Image::new(uri)
                    .maintain_aspect_ratio(false)
                    .uv(Rect::from_min_max(pos2(0.0, crop), pos2(1.0, 1.0 - crop)))
                    .fit_to_exact_size(Vec2::new(card, height))
                    .corner_radius(8.0),
            );
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::MapPin, 16.0, palette.accent);
            widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
        });
        if let Some(detail) = detail {
            widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
        }
        if theme::link(
            ui,
            crate::i18n::gettext(view.locale, "Open in a map").as_ref(),
            theme::regular(12.5),
            palette.link,
        )
        .clicked()
        {
            actions.push(Action::OpenUrl(format!(
                "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=16/{latitude}/{longitude}"
            )));
        }
    });
}

fn thumbnail_uri(ctx: &egui::Context, chat: &str, id: &str, bytes: &[u8]) -> String {
    let uri = format!(
        "bytes://thumb-{}-{}",
        chat.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        id
    );
    crate::image_cache::include(ctx, uri.clone(), bytes);
    uri
}

/// Longest side a video's tiny poster is enlarged to before it is drawn.
const SMOOTH_POSTER_SIDE: u32 = 480;
/// How many enlarged posters stay in memory; the rest are made again.
const SMOOTH_POSTERS_KEPT: usize = 48;

type SmoothPosters = std::sync::Arc<std::sync::Mutex<HashMap<String, std::sync::Arc<[u8]>>>>;

/// The poster that came with a video, enlarged with a cubic filter and a
/// light blur: the phone's is about a hundred pixels wide, and stretching it
/// on the graphics card shows its blocks. The poster itself is returned when
/// it is already large, or cannot be read.
fn smooth_poster(ctx: &egui::Context, key: &str, bytes: &[u8]) -> std::sync::Arc<[u8]> {
    let memo: SmoothPosters = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<SmoothPosters>(egui::Id::new("smooth-posters"))
            .clone()
    });
    let mut memo = memo.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(made) = memo.get(key) {
        return made.clone();
    }
    let made: std::sync::Arc<[u8]> = enlarge_poster(bytes).unwrap_or_else(|| bytes.into()).into();
    if memo.len() >= SMOOTH_POSTERS_KEPT {
        memo.clear();
    }
    memo.insert(key.to_owned(), made.clone());
    made
}

fn enlarge_poster(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::imageops::{FilterType, blur, resize};
    let picture = image::load_from_memory(bytes).ok()?.into_rgb8();
    let (width, height) = picture.dimensions();
    let longest = width.max(height);
    if longest == 0 || longest >= SMOOTH_POSTER_SIDE {
        return None;
    }
    let scale = SMOOTH_POSTER_SIDE as f32 / longest as f32;
    let enlarged = resize(
        &picture,
        (width as f32 * scale).round() as u32,
        (height as f32 * scale).round() as u32,
        FilterType::CatmullRom,
    );
    // Enough to melt the JPEG's blocks, little enough to keep its edges.
    let smoothed = blur(&enlarged, scale * 0.4);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&smoothed)
        .ok()?;
    Some(out)
}

/// Default image bounds based on [`CARD_WIDTH`].
const PICTURE_WIDTH: f32 = CARD_WIDTH;
const PICTURE_HEIGHT: f32 = 440.0;
const STICKER_SIDE: f32 = 180.0;
/// Width of an image plus bubble padding.
const HEADER_ROW: f32 = 44.0;

/// Fits an image within bounds without upscaling and with a readable minimum.
fn fit_picture(width: f32, height: f32, max_width: f32, max_height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (4.0, 3.0)
    };
    let max_width = max_width.max(0.0);
    let max_height = max_height.max(0.0);
    let scale = (max_width / width).min(max_height / height).clamp(0.0, 1.0);
    let scale = if width * scale < 120.0 {
        (120.0 / width).min(max_width / width).max(0.0)
    } else {
        scale
    };
    vec2(
        (width * scale).max(0.0),
        (height * scale).max(90.0).max(0.0),
    )
}

/// Fits a sticker to the standard square size.
fn fit_sticker(width: f32, height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (1.0, 1.0)
    };
    let scale = (STICKER_SIDE / width).min(STICKER_SIDE / height);
    vec2(width * scale, height * scale)
}

/// Reserved size for an image or video before and after download.
fn frame_size(
    media: &Media,
    thumbnail_hint: Option<(u32, u32)>,
    max_width: f32,
    max_height: f32,
) -> Vec2 {
    let (w, h) = match (media.width, media.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => match thumbnail_hint {
            Some((w, h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ => (4.0, 3.0),
        },
    };
    fit_picture(w, h, max_width, max_height)
}

/// Draws an image or sticker, using its preview until downloaded. Returns its width.
fn picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    width: f32,
    sticker: Option<bool>,
    actions: &mut Vec<Action>,
) -> Rect {
    let palette = view.palette;
    let (max_width, max_height) = match sticker {
        Some(_) => (STICKER_SIDE, STICKER_SIDE),
        None => (width.min(PICTURE_WIDTH), PICTURE_HEIGHT),
    };
    if let Some(path) = &media.path {
        if sticker == Some(true) {
            let size = fit_sticker(
                media.width.unwrap_or(180) as f32,
                media.height.unwrap_or(180) as f32,
            );
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            if ui.is_rect_visible(rect) {
                match animation::frame(ui, path, rect, view.animate && response.hovered()) {
                    animation::Frame::Ready(texture) => {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    _ => {
                        ui.painter().rect_filled(rect, 6.0, palette.surface);
                        theme::paint_icon(ui, Icon::Sticker, rect, 32.0, palette.secondary);
                    }
                }
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                actions.push(Action::OpenFile(path.clone()));
            }
            return rect;
        }
        // A row that is off screen only reserves its space. Loading the image
        // decodes it and uploads a texture, so it waits until it is scrolled
        // into view, and `image_cache` can release it once it leaves again.
        // The space is the size the picture was last drawn at, when known:
        // a message without dimensions (or with wrong ones) would otherwise
        // take one height on screen and another off it, and a picture across
        // the top edge of a transcript held at its end would flip between
        // them on every frame, shaking the whole chat (#179).
        let fit = |pixels: Vec2| {
            if sticker.is_some() {
                fit_sticker(pixels.x, pixels.y)
            } else {
                fit_picture(pixels.x, pixels.y, max_width, max_height)
            }
        };
        let pixels_id = egui::Id::new(("picture-pixels", path));
        let drawn = ui.ctx().data(|data| data.get_temp::<Vec2>(pixels_id));
        let reserved = drawn.map_or_else(|| frame_size(media, None, max_width, max_height), fit);
        let position = ui.next_widget_position();
        if !ui.is_rect_visible(Rect::from_min_size(position, reserved)) {
            return ui.allocate_exact_size(reserved, Sense::hover()).0;
        }
        let image = widgets::file_image(ui, path);
        return match image.load_for_size(ui.ctx(), vec2(max_width, max_height)) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                if drawn != Some(texture.size) {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(pixels_id, texture.size));
                }
                let size = fit(texture.size);
                let response = ui.add(
                    image
                        .fit_to_exact_size(size)
                        .corner_radius(if sticker.is_some() { 0.0 } else { 6.0 })
                        .sense(Sense::click()),
                );
                let rect = response.rect;
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    let action = match crate::image_preview::open_target(path, sticker.is_none()) {
                        crate::image_preview::OpenTarget::Preview => {
                            Action::PreviewImage(path.clone())
                        }
                        crate::image_preview::OpenTarget::External => {
                            Action::OpenFile(path.clone())
                        }
                    };
                    actions.push(action);
                }
                if sticker.is_none() {
                    sending_overlay(ui, view, message, rect, actions);
                }
                rect
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => {
                // A picture released while away loads again at its old size.
                let size = match drawn {
                    Some(pixels) => fit(pixels),
                    None if sticker.is_some() => Vec2::splat(STICKER_SIDE),
                    None => frame_size(media, None, max_width, max_height),
                };
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, 6.0, palette.surface);
                    theme::paint_spinner(ui, rect, 22.0, palette.accent);
                }
                rect
            }
            Err(_) => {
                let size = if sticker.is_some() {
                    Vec2::splat(STICKER_SIDE)
                } else {
                    frame_size(media, None, max_width, max_height)
                };
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, 6.0, palette.surface);
                    theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, palette.danger);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 24.0),
                        Align2::CENTER_CENTER,
                        tr("Could not display this picture. Click to open it."),
                        theme::regular(11.5),
                        palette.secondary,
                    );
                }
                if response.clicked() {
                    actions.push(Action::OpenFile(path.clone()));
                }
                rect
            }
        };
    }
    let size = frame_size(media, None, max_width, max_height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let thumbnail = message
            .thumbnail
            .as_deref()
            .filter(|_| sticker.is_none())
            .map(|bytes| thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes));
        match thumbnail {
            Some(uri) => {
                egui::Image::new(uri)
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
                ui.painter()
                    .rect_filled(rect, 6.0, Color32::from_black_alpha(60));
            }
            None => {
                ui.painter().rect_filled(rect, 6.0, palette.surface);
            }
        }
        let disc = Rect::from_center_size(rect.center(), Vec2::splat(44.0));
        match &media.state {
            MediaState::Downloading => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_spinner(ui, disc, 22.0, Color32::WHITE);
            }
            MediaState::Failed(_) => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 34.0),
                    Align2::CENTER_CENTER,
                    tr("Download failed. Click to retry."),
                    theme::regular(11.5),
                    Color32::WHITE,
                );
            }
            MediaState::Idle => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(
                    ui,
                    if sticker.is_some() {
                        Icon::Sticker
                    } else {
                        Icon::Download
                    },
                    disc,
                    22.0,
                    Color32::WHITE,
                );
                if sticker.is_none() {
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::util::bytes(media.size),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
            }
        }
    }
    let wants = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !matches!(media.state, MediaState::Downloading);
    let auto = ui.is_rect_visible(rect)
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(
            media,
            if sticker == Some(true) {
                crate::settings::AutoDownloadKind::AnimatedSticker
            } else {
                crate::settings::AutoDownloadKind::Image
            },
            view.download_settings,
        );
    if wants || auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    rect
}

/// Whether an own attachment or voice message is being prepared, uploaded
/// and sent.
fn media_sending(view: &View<'_>, message: &Message) -> bool {
    message.from_me
        && view
            .media_sending
            .contains(&(view.chat.id.clone(), message.id.clone()))
}

/// Whether an own attachment failed to go out and can be sent again.
fn media_unsent(view: &View<'_>, message: &Message) -> bool {
    message.from_me
        && message.status == Delivery::Failed
        && matches!(
            message.content,
            Content::Image { .. }
                | Content::Video { .. }
                | Content::Audio { .. }
                | Content::Document { .. }
        )
        && !media_sending(view, message)
}

/// Share of an own attachment uploaded so far, in percent, once its upload
/// has started.
fn media_progress(view: &View<'_>, message: &Message) -> Option<u8> {
    view.media_progress
        .get(&(view.chat.id.clone(), message.id.clone()))
        .copied()
}

/// A sending attachment's disc, as on the phone: a ring that fills as it
/// uploads (a spinner until the upload starts) around an X that cancels it,
/// with the share done below.
fn paint_upload_progress(ui: &egui::Ui, disc: Rect, percent: Option<u8>, hovered: bool) {
    let center = disc.center();
    ui.painter().circle_filled(
        center,
        disc.width() / 2.0,
        Color32::from_black_alpha(if hovered { 170 } else { 120 }),
    );
    match percent {
        Some(percent) => {
            let radius = disc.width() / 2.0 - 5.0;
            ui.painter().circle_stroke(
                center,
                radius,
                Stroke::new(3.0, Color32::from_white_alpha(60)),
            );
            if percent > 0 {
                ui.painter().add(egui::Shape::line(
                    crate::video::arc(center, radius, f32::from(percent) / 100.0),
                    Stroke::new(3.0, Color32::WHITE),
                ));
            }
            let galley = ui.painter().layout_no_wrap(
                format!("{percent}%"),
                theme::medium(11.0),
                Color32::WHITE,
            );
            let chip = Rect::from_center_size(
                pos2(center.x, disc.bottom() + 6.0 + galley.size().y / 2.0 + 2.0),
                galley.size() + vec2(10.0, 4.0),
            );
            ui.painter()
                .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
            ui.painter()
                .galley(chip.min + vec2(5.0, 2.0), galley, Color32::WHITE);
        }
        None => theme::paint_spinner(ui, disc, disc.width() / 2.0, Color32::WHITE),
    }
    theme::paint_icon(ui, Icon::X, disc, 18.0, Color32::WHITE);
}

/// Over an own picture or video, as on the phone: the upload's progress
/// with a button that cancels it while it is sent, and a button to send it
/// again if that failed or was cancelled.
fn sending_overlay(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let disc = Rect::from_center_size(rect.center(), Vec2::splat(48.0));
    if media_sending(view, message) {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_black_alpha(60));
        let response = ui
            .interact(
                disc,
                ui.id().with(("cancel-upload", &message.id)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Cancel sending"));
        paint_upload_progress(ui, disc, media_progress(view, message), response.hovered());
        if response.clicked() {
            actions.push(Action::CancelMedia {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
            });
        }
    } else if media_unsent(view, message) {
        let response = ui
            .interact(disc, ui.id().with(("resend", &message.id)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Send again"));
        ui.painter().circle_filled(
            disc.center(),
            22.0,
            Color32::from_black_alpha(if response.hovered() { 170 } else { 120 }),
        );
        theme::paint_icon(ui, Icon::Refresh, disc, 22.0, Color32::WHITE);
        if response.clicked() {
            actions.push(Action::RetryMedia {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
            });
        }
    }
}

/// Media downloads when visible as its kind's setting says, within the
/// shared size limit.
fn auto_download_allowed(
    media: &Media,
    kind: crate::settings::AutoDownloadKind,
    settings: &crate::settings::Settings,
) -> bool {
    media.is_within_download_limit(settings.attachment_limit_bytes())
        && settings.auto_downloads(kind)
}

/// Draws a video. GIFs play in place; other videos play in the bubble once
/// clicked, downloading first when needed, with controls along the bottom.
#[allow(clippy::too_many_arguments)]
fn video(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    gif: bool,
    width: f32,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    // Messages can arrive without a poster (history from the phone often
    // leaves it out); the frame still draws as a video, on a dark fill.
    let thumbnail = message.thumbnail.as_deref();
    let limit = width.min(PICTURE_WIDTH);
    // Without its size, a widescreen frame that fills the bubble: the hint is
    // in pixels, so a bare 16 by 9 would shrink it to the narrowest picture.
    let size = frame_size(
        media,
        Some((1280, 720)),
        limit,
        PICTURE_HEIGHT.min(limit * 1.3),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let playing = match (&media.path, gif) {
        (Some(path), true) => Some(animation::frame(
            ui,
            path,
            rect,
            view.animate && response.hovered(),
        )),
        _ => None,
    };
    if let Some(animation::Frame::Ready(texture)) = &playing {
        if ui.is_rect_visible(rect) {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && let Some(path) = &media.path
        {
            actions.push(Action::OpenFile(path.clone()));
        }
        return size.x;
    }
    // While expanded, the video plays only over the window; its message
    // shows the poster, as a video that is not playing.
    let status = media
        .path
        .as_ref()
        .filter(|_| !gif && view.video_expanded != Some(message.id.as_str()))
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        match status.as_ref().and_then(|status| status.frame.as_ref()) {
            Some(frame) => {
                ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                paint_texture(
                    ui,
                    fit_within(frame.size_vec2(), rect),
                    frame.id(),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    6.0,
                );
            }
            None => {
                // A downloaded video shows its own first frame, sharp at the
                // bubble's size; the phone's tiny poster stands in until that
                // frame is decoded, and for a video that is not downloaded.
                let first_frame = media.path.as_ref().and_then(|path| {
                    match animation::frame(ui, path, rect, false) {
                        animation::Frame::Ready(texture) => Some(texture),
                        _ => None,
                    }
                });
                match (first_frame, thumbnail) {
                    (Some(texture), _) => {
                        ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                        paint_texture(
                            ui,
                            fit_within(texture.size_vec2(), rect),
                            texture.id(),
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            6.0,
                        );
                    }
                    (None, Some(thumbnail)) => {
                        // Registering the poster decodes it, so it waits for the row to show.
                        let key = format!("{}-{}", message.chat, message.id);
                        let smooth = smooth_poster(ui.ctx(), &key, thumbnail);
                        // Its own id: a quote of this message registers the plain poster.
                        let id = format!("{}-smooth", message.id);
                        let uri = thumbnail_uri(ui.ctx(), &message.chat, &id, &smooth);
                        egui::Image::new(uri)
                            .fit_to_exact_size(size)
                            .corner_radius(6.0)
                            .paint_at(ui, rect);
                    }
                    (None, None) => {
                        let fill = if media.path.is_some() {
                            Color32::BLACK
                        } else {
                            Color32::from_gray(28)
                        };
                        ui.painter().rect_filled(rect, 6.0, fill);
                    }
                }
            }
        }
        let state = status.as_ref().map(|status| status.state);
        let outgoing = media_sending(view, message) || media_unsent(view, message);
        if outgoing {
            sending_overlay(ui, view, message, rect, actions);
        } else if state != Some(State::Playing) {
            ui.painter()
                .rect_filled(rect, 6.0, Color32::from_black_alpha(40));
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(48.0));
            ui.painter()
                .circle_filled(disc.center(), 24.0, Color32::from_black_alpha(140));
            match (&media.path, &media.state) {
                (Some(_), _)
                    if state == Some(State::Loading)
                        || matches!(playing, Some(animation::Frame::Pending)) =>
                {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (Some(_), _) if gif => {
                    theme::paint_icon(ui, Icon::ExternalLink, disc, 22.0, Color32::WHITE)
                }
                (None, MediaState::Downloading) => {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (None, MediaState::Failed(_)) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger)
                }
                (Some(_), _) | (None, MediaState::Idle) => {
                    theme::paint_icon(ui, Icon::Play, disc, 22.0, Color32::WHITE)
                }
            }
        }
        match (&status, &media.path) {
            (Some(status), Some(path)) => {
                // Controls show while paused and while the pointer is over
                // the video, as in other players.
                if status.state == State::Paused
                    || (status.state == State::Playing && ui.rect_contains_pointer(rect))
                {
                    let controls = VideoControls {
                        palette: &view.palette,
                        locale: view.locale,
                        video: view.video,
                        message: &message.id,
                        path,
                    };
                    video_controls(ui, &controls, rect, status, actions);
                    expand_button(ui, view.locale, &message.id, rect, actions);
                }
            }
            _ => {
                let mut label = Vec::new();
                if gif {
                    label.push("GIF".to_owned());
                }
                if let Some(seconds) = seconds {
                    label.push(crate::util::duration(seconds));
                }
                match media_progress(view, message).filter(|_| media_sending(view, message)) {
                    Some(percent) => label.push(format!(
                        "{} / {}",
                        crate::util::bytes(media.size * u64::from(percent) / 100),
                        crate::util::bytes(media.size)
                    )),
                    None if media.path.is_none() || media_sending(view, message) => {
                        label.push(crate::util::bytes(media.size))
                    }
                    None => {}
                }
                if !label.is_empty() {
                    let galley = ui.painter().layout_no_wrap(
                        label.join(" · "),
                        theme::medium(11.5),
                        Color32::WHITE,
                    );
                    let chip = Rect::from_min_size(
                        pos2(rect.left() + 8.0, rect.bottom() - galley.size().y - 14.0),
                        galley.size() + vec2(12.0, 6.0),
                    );
                    ui.painter().rect_filled(
                        chip,
                        chip.height() / 2.0,
                        Color32::from_black_alpha(140),
                    );
                    ui.painter()
                        .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
                }
            }
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(
            media,
            crate::settings::AutoDownloadKind::Video,
            view.download_settings,
        );
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !media_sending(view, message)
    {
        video_clicked(view, message, media, gif, auto, actions);
    }
    size.x
}

/// What a click on a video does: a downloaded video plays or pauses, a GIF
/// that cannot play here opens in the system viewer, and a video still on
/// WhatsApp's servers downloads and then plays. `downloading` is set when
/// this frame already asked for the download.
fn video_clicked(
    view: &View<'_>,
    message: &Message,
    media: &Media,
    gif: bool,
    downloading: bool,
    actions: &mut Vec<Action>,
) {
    match &media.path {
        Some(path) if gif => actions.push(Action::OpenFile(path.clone())),
        Some(path) => actions.push(Action::PlayVideo {
            message: message.id.clone(),
            path: path.clone(),
        }),
        None => {
            if !downloading && !matches!(media.state, MediaState::Downloading) {
                actions.push(Action::Download {
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                });
            }
            if !gif {
                actions.push(Action::PlayVideoWhenDownloaded(message.id.clone()));
            }
        }
    }
}

/// Play/pause, the time, a seek bar, and a sound switch along the bottom of
/// a playing video.
/// What the controls of a loaded video act on and look like.
pub(crate) struct VideoControls<'a> {
    pub palette: &'a Palette,
    pub locale: crate::i18n::Locale,
    pub video: &'a crate::video::Player,
    pub message: &'a str,
    pub path: &'a Path,
}

pub(crate) fn video_controls(
    ui: &mut egui::Ui,
    controls: &VideoControls<'_>,
    rect: Rect,
    status: &crate::video::Status,
    actions: &mut Vec<Action>,
) {
    let VideoControls {
        palette,
        locale,
        video,
        message,
        path,
    } = *controls;
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 32.0), rect.max);
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        },
        Color32::from_black_alpha(150),
    );
    let id = ui.id().with(("video-controls", message));
    let toggle = Rect::from_center_size(pos2(bar.left() + 18.0, bar.center().y), Vec2::splat(26.0));
    let playing = status.state == crate::video::State::Playing;
    theme::paint_icon(
        ui,
        if playing { Icon::Pause } else { Icon::Play },
        toggle,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if playing {
        crate::i18n::gettext(locale, "Pause")
    } else {
        crate::i18n::gettext(locale, "Play")
    };
    if ui
        .interact(toggle, id.with("toggle"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::PlayVideo {
            message: message.to_owned(),
            path: path.to_owned(),
        });
    }
    let sound = Rect::from_center_size(pos2(bar.right() - 18.0, bar.center().y), Vec2::splat(26.0));
    let muted = video.muted();
    theme::paint_icon(
        ui,
        if muted { Icon::VolumeX } else { Icon::Volume2 },
        sound,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if muted {
        crate::i18n::gettext(locale, "Unmute")
    } else {
        crate::i18n::gettext(locale, "Mute")
    };
    if ui
        .interact(sound, id.with("sound"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::ToggleVideoSound);
    }
    let time = format!(
        "{} / {}",
        crate::util::duration(status.position.as_secs() as u32),
        crate::util::duration(status.total.as_secs() as u32)
    );
    let galley = ui
        .painter()
        .layout_no_wrap(time, theme::medium(11.5), Color32::WHITE);
    let text = pos2(toggle.right() + 4.0, bar.center().y - galley.size().y / 2.0);
    let track = Rect::from_min_max(
        pos2(text.x + galley.size().x + 10.0, bar.center().y - 8.0),
        pos2(sound.left() - 6.0, bar.center().y + 8.0),
    );
    ui.painter().galley(text, galley, Color32::WHITE);
    if track.width() < 12.0 {
        return;
    }
    let response = ui
        .interact(track, id.with("seek"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pointed = response
        .interact_pointer_pos()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    // Seeking restarts the decoder, so a drag seeks once, where it ends.
    let fraction = match pointed {
        Some(fraction) if response.dragged() => fraction,
        _ => status.fraction(),
    };
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 3.0));
    ui.painter()
        .rect_filled(line, 1.5, Color32::from_white_alpha(90));
    let played = pos2(line.left() + fraction * line.width(), line.center().y);
    ui.painter().rect_filled(
        Rect::from_min_max(line.min, pos2(played.x, line.bottom())),
        1.5,
        palette.accent,
    );
    ui.painter().circle_filled(played, 5.0, palette.accent);
    if (response.clicked() || response.drag_stopped())
        && let Some(fraction) = pointed
    {
        actions.push(Action::SeekVideo {
            message: message.to_owned(),
            fraction,
        });
    }
}

/// A button in the top right corner of a video that shows it over the
/// window, drawn with the other controls.
fn expand_button(
    ui: &mut egui::Ui,
    locale: crate::i18n::Locale,
    message: &str,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    if rect.width() < 80.0 || rect.height() < 80.0 {
        return;
    }
    let button = Rect::from_center_size(
        pos2(rect.right() - 20.0, rect.top() + 20.0),
        Vec2::splat(28.0),
    );
    ui.painter()
        .circle_filled(button.center(), 14.0, Color32::from_black_alpha(150));
    theme::paint_icon(ui, Icon::Maximize, button, 15.0, Color32::WHITE);
    if ui
        .interact(
            button,
            ui.id().with(("video-expand", message)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::gettext(locale, "Expand video").as_ref())
        .clicked()
    {
        actions.push(Action::ExpandVideo(message.to_owned()));
    }
}

/// How long the floating date stays after the scrolling stops, and how much
/// of that it spends fading.
const FLOATING_DAY: f64 = 1.2;
const FLOATING_FADE: f64 = 0.3;

/// The date of the messages at the top of the view, centred over it while
/// the reader scrolls, as WhatsApp shows it. It fades once the scrolling
/// stops and is gone at the end of the chat.
fn floating_day(
    ui: &egui::Ui,
    app: &App,
    chat: &str,
    view: Rect,
    day: Option<i64>,
    scrolling: bool,
) {
    let id = egui::Id::new(("floating-day", chat));
    let now = ui.input(|input| input.time);
    if scrolling {
        ui.ctx().data_mut(|data| data.insert_temp(id, now));
    }
    let Some(day) = day else {
        return;
    };
    let Some(since) = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(id))
        .map(|last| now - last)
        .filter(|since| *since < FLOATING_DAY)
    else {
        return;
    };
    let fade = ((FLOATING_DAY - since) / FLOATING_FADE).min(1.0) as f32;
    let label = crate::util::scroll_day(app.locale, day);
    let palette = app.palette;
    egui::Area::new(id.with("area"))
        .order(egui::Order::Middle)
        .interactable(false)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(pos2(view.center().x, view.top() + 10.0))
        .show(ui.ctx(), |ui| {
            ui.multiply_opacity(fade);
            widgets::chip(ui, &palette, &label);
        });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
}

/// Side of a round video message.
const NOTE_SIDE: f32 = 220.0;

/// Draws a round video message (PTV) as a circle that plays in place when
/// clicked, with a ring for the progress. Returns its width.
fn video_note(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(NOTE_SIDE), Sense::click());
    let status = media
        .path
        .as_ref()
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        let center = rect.center();
        // Leave room around the picture for the progress ring.
        let picture = rect.shrink(5.0);
        let frame = status
            .as_ref()
            .and_then(|status| status.frame.as_ref())
            .map(|frame| (frame.id(), frame.size_vec2()));
        let poster = || {
            let bytes = message.thumbnail.as_deref()?;
            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
            match egui::Image::new(uri).load_for_size(ui.ctx(), picture.size()) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some((texture.id, texture.size)),
                _ => None,
            }
        };
        match frame.or_else(poster) {
            Some((texture, size)) => paint_texture(
                ui,
                picture,
                texture,
                crate::video::square_uv(size.x, size.y),
                picture.width() / 2.0,
            ),
            None => {
                ui.painter()
                    .circle_filled(center, picture.width() / 2.0, palette.surface);
            }
        }
        let state = status.as_ref().map(|status| status.state);
        let ring = NOTE_SIDE / 2.0 - 2.0;
        if let Some(status) = &status {
            ui.painter().circle_stroke(
                center,
                ring,
                Stroke::new(3.0, palette.secondary.gamma_multiply(0.35)),
            );
            ui.painter().add(egui::Shape::line(
                crate::video::arc(center, ring, status.fraction()),
                Stroke::new(3.0, palette.accent),
            ));
        }
        let hovered = ui.rect_contains_pointer(rect);
        let disc = Rect::from_center_size(center, Vec2::splat(48.0));
        let waiting = matches!(
            (&media.path, &media.state, state),
            (Some(_), _, Some(State::Loading)) | (None, MediaState::Downloading, _)
        );
        let icon = match (&media.path, &media.state, state) {
            _ if waiting => None,
            // Only a pause sign under the pointer covers a playing video.
            (Some(_), _, Some(State::Playing)) => hovered.then_some(Icon::Pause),
            (None, MediaState::Failed(_), _) => Some(Icon::CircleAlert),
            _ => Some(Icon::Play),
        };
        if state != Some(State::Playing) {
            ui.painter().circle_filled(
                center,
                picture.width() / 2.0,
                Color32::from_black_alpha(40),
            );
        }
        if waiting || icon.is_some() {
            ui.painter()
                .circle_filled(center, 24.0, Color32::from_black_alpha(140));
        }
        if waiting {
            theme::paint_spinner(ui, disc, 24.0, Color32::WHITE);
        } else if let Some(icon) = icon {
            let color = if icon == Icon::CircleAlert {
                palette.danger
            } else {
                Color32::WHITE
            };
            theme::paint_icon(ui, icon, disc, 22.0, color);
        }
        // The time while it plays, otherwise its length.
        let label = match &status {
            Some(status) => Some(crate::util::duration(status.position.as_secs() as u32)),
            None => seconds
                .map(crate::util::duration)
                .or_else(|| media.path.is_none().then(|| crate::util::bytes(media.size))),
        };
        if let Some(label) = label {
            let galley = ui
                .painter()
                .layout_no_wrap(label, theme::medium(11.5), Color32::WHITE);
            let chip = Rect::from_center_size(
                pos2(center.x, picture.bottom() - galley.size().y / 2.0 - 16.0),
                galley.size() + vec2(12.0, 6.0),
            );
            ui.painter()
                .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
            ui.painter()
                .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(
            media,
            crate::settings::AutoDownloadKind::Video,
            view.download_settings,
        );
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        video_clicked(view, message, media, false, auto, actions);
    }
    NOTE_SIDE
}

/// Paints a texture into `rect` with rounded corners; a radius of half the
/// side makes a circle.
pub(crate) fn paint_texture(
    ui: &egui::Ui,
    rect: Rect,
    texture: egui::TextureId,
    uv: Rect,
    radius: f32,
) {
    ui.painter().add(
        egui::epaint::RectShape::filled(
            rect,
            CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8),
            Color32::WHITE,
        )
        .with_texture(texture, uv),
    );
}

/// The largest rect with the proportions of `size` centred in `rect`.
pub(crate) fn fit_within(size: Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

#[allow(clippy::too_many_arguments)]
fn attachment(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    title: &str,
    detail: &str,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let cancel_ring = std::cell::Cell::new(None);
    let save_button = std::cell::Cell::new(None);
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // Include margins in the settled row width.
            let card = (width - 20.0).max(0.0);
            ui.set_width(card);

            let disc = |ui: &mut egui::Ui| {
                let (slot, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                widgets::paint_file_badge(ui, &palette, slot.shrink2(vec2(0.0, 1.0)), title);
            };
            let sending = media_sending(view, message);
            let progress = media_progress(view, message).filter(|_| sending);
            let action = |ui: &mut egui::Ui| match (&media.path, &media.state) {
                _ if sending => {
                    // A ring that fills as it uploads, around an X that
                    // cancels it.
                    let (ring, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    match progress {
                        Some(percent) => {
                            ui.painter().circle_stroke(
                                ring.center(),
                                10.0,
                                Stroke::new(2.5, palette.accent.gamma_multiply(0.25)),
                            );
                            ui.painter().add(egui::Shape::line(
                                crate::video::arc(ring.center(), 10.0, f32::from(percent) / 100.0),
                                Stroke::new(2.5, palette.accent),
                            ));
                        }
                        None => theme::paint_spinner(ui, ring, 11.0, palette.accent),
                    }
                    theme::paint_icon(ui, Icon::X, ring, 11.0, palette.accent);
                    cancel_ring.set(Some(ring));
                }
                (None, MediaState::Downloading) => {
                    theme::spinner(ui, 18.0, palette.accent);
                }
                // Saves a copy where the person chooses, downloading first
                // when needed; the rest of the card opens the file.
                _ => {
                    // As wide as the icon: the row is laid out to the card's width.
                    let (slot, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    theme::paint_icon(ui, Icon::Download, slot, 18.0, palette.secondary);
                    save_button.set(Some(slot));
                }
            };
            let column = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    // Reserve 70 points for the icon, action, and gaps.
                    ui.set_width((card - 70.0).max(0.0));
                    widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
                    let detail = match (&media.state, progress) {
                        (_, Some(percent)) => format!(
                            "{} / {} · {percent}%",
                            crate::util::bytes(media.size * u64::from(percent) / 100),
                            crate::util::bytes(media.size)
                        ),
                        (MediaState::Failed(error), None) => {
                            format!("{error}. {}", tr("Click to retry."))
                        }
                        _ => detail.to_owned(),
                    };
                    theme::text(ui, detail, theme::regular(12.0), palette.secondary);
                });
            };
            // Fix the row left-to-right at the card width in own bubbles.
            ui.allocate_ui_with_layout(
                vec2(card, 52.0),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    disc(ui);
                    column(ui);
                    action(ui);
                },
            );
        })
        .response;
    let auto = ui.is_rect_visible(response.rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(
            media,
            crate::settings::AutoDownloadKind::Document,
            view.download_settings,
        );
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("card"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("attachment", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // Over the card, so it takes the click.
    let cancelled = cancel_ring.get().is_some_and(|ring| {
        ui.interact(
            ring.expand(4.0),
            ui.id().with(("cancel-upload", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr("Cancel sending"))
        .clicked()
    });
    let save = !cancelled
        && save_button.get().is_some_and(|slot| {
            ui.interact(
                slot.expand(6.0),
                ui.id().with(("save-attachment", &message.id)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Save as…"))
            .clicked()
        });
    if cancelled {
        actions.push(Action::CancelMedia {
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    } else if (save || response.clicked()) && !media_sending(view, message) {
        match &media.path {
            Some(path) if save => actions.push(Action::SaveAttachmentAs {
                path: path.clone(),
                name: attachment_name(&message.content, path),
            }),
            Some(path) => actions.push(Action::OpenFile(path.clone())),
            None => actions.push(Action::DownloadDocument {
                chat: view.chat.id.clone(),
                message: message.id.clone(),
                save_as: save,
            }),
        }
    }
}

/// The voice player's play or pause button: a solid shape nearly as large
/// as the button, without a disc behind it, as on the phone.
fn play_button(
    ui: &mut egui::Ui,
    palette: Palette,
    size: f32,
    playing: bool,
    own: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, size / 2.0);
    let tooltip = if playing { tr("Pause") } else { tr("Play") };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // A little darker than the accent, to stand out in the bubble; on the
        // built-in dark theme's green own bubble, a lighter leaf green does.
        let (rest, hover) = if own && palette.dark && palette.accent == Palette::dark().accent {
            let leaf = Color32::from_rgb(0x1f, 0xaa, 0x59);
            (leaf, leaf.lerp_to_gamma(Color32::WHITE, 0.15))
        } else {
            (
                palette.accent.lerp_to_gamma(Color32::BLACK, 0.18),
                palette.accent,
            )
        };
        let colour = if hovered { hover } else { rest };
        let scale = if hovered { 1.05 } else { 1.0 };
        let centre = rect.center();
        if playing {
            let bar = vec2(size * 0.2, size * 0.66) * scale;
            let offset = size * 0.15 * scale;
            for side in [-1.0, 1.0] {
                ui.painter().rect_filled(
                    Rect::from_center_size(centre + vec2(side * offset, 0.0), bar),
                    size * 0.06,
                    colour,
                );
            }
        } else {
            // A right-pointing triangle, nudged right so it looks centred.
            let height = size * 0.74 * scale;
            let width = height * 0.87;
            let nudge = width * 0.1;
            let corners = [
                centre + vec2(-width / 2.0 + nudge, -height / 2.0),
                centre + vec2(width / 2.0 + nudge, 0.0),
                centre + vec2(-width / 2.0 + nudge, height / 2.0),
            ];
            ui.painter().add(egui::Shape::convex_polygon(
                rounded_corners(&corners, size * 0.07),
                colour,
                Stroke::NONE,
            ));
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.on_hover_text(tooltip)
}

/// The outline of a convex polygon with each corner rounded off by `radius`.
fn rounded_corners(corners: &[egui::Pos2], radius: f32) -> Vec<egui::Pos2> {
    const STEPS: usize = 6;
    let count = corners.len();
    let mut points = Vec::with_capacity(count * (STEPS + 1));
    for index in 0..count {
        let corner = corners[index];
        let before = corners[(index + count - 1) % count];
        let after = corners[(index + 1) % count];
        let to_before = (before - corner).normalized();
        let to_after = (after - corner).normalized();
        let angle = to_before.dot(to_after).clamp(-1.0, 1.0).acos();
        // The arc touches both edges at `reach` from the corner.
        let reach = radius / (angle / 2.0).tan();
        let start = corner + to_before * reach;
        let end = corner + to_after * reach;
        let bisector = (to_before + to_after).normalized();
        let centre = corner + bisector * (radius / (angle / 2.0).sin());
        let from = (start - centre).angle();
        let mut sweep = (end - centre).angle() - from;
        if sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        } else if sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        for step in 0..=STEPS {
            let turn = from + sweep * step as f32 / STEPS as f32;
            points.push(centre + radius * vec2(turn.cos(), turn.sin()));
        }
    }
    points
}

/// In-chat voice and audio player.
#[allow(clippy::too_many_arguments)]
fn voice_player(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    waveform: &[u8],
    width: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    use crate::audio::State;
    let palette = view.palette;
    let status = view.player.status(&message.id);
    let button = 36.0;
    let bar_height = 30.0;
    let chip = 44.0;
    // As on the phone, an own voice message spins while it is encoded,
    // uploaded and sent, and offers to go again if that failed; it plays
    // once the server has it.
    let own_voice = message.from_me
        && matches!(
            message.content,
            Content::Audio {
                voice_note: true,
                ..
            }
        );
    let sending = own_voice
        && view
            .media_sending
            .contains(&(view.chat.id.clone(), message.id.clone()));
    let unsent = own_voice && !sending && message.status == Delivery::Failed;
    let playable = media.path.as_ref().filter(|_| !sending && !unsent);
    let key = (view.chat.id.clone(), message.id.clone());
    let transcript = view.transcripts.get(&key);
    let folded = transcript.is_some() && view.transcripts_folded.contains(&key);
    let progress = view.transcriber.progress(&key);
    // The button asks for a transcript, or opens a folded one.
    let transcribe_button = playable.is_some()
        && progress.is_none()
        && (folded || (transcript.is_none() && view.download_settings.show_transcribe_button));
    // The chip appears with the playable clip; the waveform takes its space
    // back while the audio is still downloading.
    let shows_chip = playable.is_some();
    let wave_width = (width - button - 10.0 - if shows_chip { chip + 10.0 } else { 0.0 }).max(0.0);
    let bars: Vec<u8> = if !waveform.is_empty() {
        waveform.to_vec()
    } else if let Some(bars) = view.player.bars(&message.id) {
        bars.to_vec()
    } else {
        vec![12; crate::voice::BARS]
    };
    let fill = palette.accent.gamma_multiply(0.22);
    let hover = palette.accent.gamma_multiply(0.38);
    let waiting = |ui: &mut egui::Ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), button / 2.0, fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                theme::spinner(ui, 18.0, palette.accent);
            });
        });
    };
    // Force left-to-right layout at the player's width inside own bubbles.
    ui.allocate_ui_with_layout(
        vec2(width.max(0.0), button),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match (&media.path, &media.state) {
                _ if sending => waiting(ui),
                _ if unsent => {
                    if theme::circle_button(
                        ui,
                        Icon::Refresh,
                        button,
                        fill,
                        hover,
                        palette.accent,
                        tr("Send again"),
                    )
                    .clicked()
                    {
                        actions.push(Action::RetryMedia {
                            chat: view.chat.id.clone(),
                            message: message.id.clone(),
                        });
                    }
                }
                (None, MediaState::Downloading) => waiting(ui),
                (None, _) => {
                    if theme::circle_button(
                        ui,
                        Icon::Download,
                        button,
                        fill,
                        hover,
                        palette.accent,
                        tr("Download"),
                    )
                    .clicked()
                    {
                        actions.push(Action::Download {
                            card: None,
                            chat: view.chat.id.clone(),
                            message: message.id.clone(),
                        });
                    }
                }
                (Some(path), _) => match status.state {
                    State::Loading => waiting(ui),
                    State::Playing | State::Paused | State::Idle => {
                        if play_button(
                            ui,
                            palette,
                            button,
                            status.state == State::Playing,
                            message.from_me,
                        )
                        .clicked()
                        {
                            actions.push(Action::PlayVoice {
                                message: message.id.clone(),
                                path: path.clone(),
                            });
                        }
                    }
                },
            }
            let mut wave_middle = None;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (rect, response) =
                    ui.allocate_exact_size(vec2(wave_width, bar_height), Sense::click());
                wave_middle = Some(rect.center().y);
                let pitch = 3.0;
                let count = (rect.width() / pitch).floor() as usize;
                let fraction = if status.total > Duration::ZERO {
                    status.position.as_secs_f32() / status.total.as_secs_f32()
                } else {
                    0.0
                };
                let played_until = rect.left() + fraction * rect.width();
                let quiet = palette.secondary.gamma_multiply(0.7);
                if count > 0 {
                    for index in 0..count {
                        let level = f32::from(bars[index * bars.len() / count]) / 100.0;
                        let height = (2.0 + level * (bar_height - 4.0)).max(2.0);
                        let x = rect.left() + index as f32 * pitch + 1.0;
                        let colour = if status.state != State::Idle && x <= played_until {
                            palette.accent
                        } else {
                            quiet
                        };
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                egui::pos2(x, rect.center().y),
                                vec2(2.0, height),
                            ),
                            1.0,
                            colour,
                        );
                    }
                }
                if matches!(status.state, State::Playing | State::Paused) && rect.width() >= 10.0 {
                    let knob = played_until.clamp(rect.left() + 5.0, rect.right() - 5.0);
                    ui.painter().circle_filled(
                        egui::pos2(knob, rect.center().y),
                        5.0,
                        palette.accent,
                    );
                }
                if let Some(path) = playable {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let fraction = if rect.width() > 0.0 {
                            ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        actions.push(Action::SeekVoice {
                            message: message.id.clone(),
                            path: path.clone(),
                            fraction,
                        });
                    }
                }
                // Show playback position while active, otherwise total duration.
                let shown = match status.state {
                    State::Playing | State::Paused => {
                        crate::util::duration(status.position.as_secs() as u32)
                    }
                    _ => seconds
                        .or_else(|| {
                            (status.total > Duration::ZERO).then_some(status.total.as_secs() as u32)
                        })
                        .map(crate::util::duration)
                        .unwrap_or_else(|| crate::util::bytes(media.size)),
                };
                let text = match &media.state {
                    MediaState::Failed(error) => format!("{error}. {}", tr("Click to retry.")),
                    _ => shown,
                };
                if let Some(path) = playable.filter(|_| transcribe_button) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        theme::text(ui, text, theme::regular(11.5), palette.secondary);
                        if transcribe_pill(ui, view, folded).clicked() {
                            actions.push(Action::Transcribe {
                                chat: view.chat.id.clone(),
                                message: message.id.clone(),
                                path: path.clone(),
                            });
                        }
                    });
                } else {
                    theme::text(ui, text, theme::regular(11.5), palette.secondary);
                }
            });
            // Speed chip, cycling 1x, 1.5x, and 2x like the phone. The
            // message menu lists every speed, including 1.25x and 1.75x.
            if shows_chip {
                let speed = view.player.speed();
                // The label follows the click at once, faded until this
                // clip actually plays at that speed.
                let preparing = view.player.preparing_speed(&message.id);
                // Reserve the chip's place in the row, then draw it level
                // with the middle of the waveform rather than the whole row.
                let size = vec2(chip, 20.0);
                let (slot, _) = ui.allocate_exact_size(size, Sense::hover());
                let at = Rect::from_center_size(
                    egui::pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    size,
                );
                let response = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                        speed_pill(ui, view, size, speed, speed > 1.0, preparing, true)
                    })
                    .inner;
                ui.ctx().data_mut(|data| {
                    data.insert_temp(speed_chip_id(&view.chat.id, &message.id), response.rect);
                });
                if response.clicked() {
                    actions.push(Action::SetVoiceSpeed(crate::audio::next_cycled_speed(
                        speed,
                    )));
                }
                response.on_hover_text(if preparing {
                    tr("Preparing playback speed")
                } else {
                    tr("Playback speed. Right-click for every speed.")
                });
            }
        },
    );
    let auto = media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(
            media,
            crate::settings::AutoDownloadKind::Audio,
            view.download_settings,
        );
    if auto {
        actions.push(Action::Download {
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    // Received voice messages are transcribed as they show up, once.
    if view.download_settings.transcribe_automatically
        && !message.from_me
        && matches!(
            message.content,
            Content::Audio {
                voice_note: true,
                ..
            }
        )
        && transcript.is_none()
        && progress.is_none()
        && let Some(path) = playable
    {
        actions.push(Action::Transcribe {
            chat: view.chat.id.clone(),
            message: message.id.clone(),
            path: path.clone(),
        });
    }
    if let Some(progress) = &progress {
        transcription_status(ui, view, message, playable, progress, width, actions);
        None
    } else {
        transcript
            .filter(|_| !folded)
            .map(|transcript| transcript_block(ui, view, message, transcript, width, actions))
    }
}

/// The icon-only button on a voice message's duration row that transcribes
/// it, or shows its folded transcript.
fn transcribe_pill(ui: &mut egui::Ui, view: &View<'_>, folded: bool) -> egui::Response {
    let palette = view.palette;
    let tooltip = if folded {
        tr("Show transcript")
    } else {
        tr("Transcribe this audio")
    };
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 18.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        // As the speed chip: incoming bubbles share the resting surface colour.
        let fill = if hovered {
            palette.surface_active
        } else {
            palette.surface_hover
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let colour = if hovered {
            palette.text
        } else {
            palette.secondary
        };
        theme::paint_icon(ui, Icon::Captions, rect, 13.0, colour);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// The thin line between a voice message's player and its transcript.
fn transcript_divider(ui: &mut egui::Ui, palette: &Palette, width: f32) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, palette.secondary.gamma_multiply(0.25)),
    );
    ui.add_space(4.0);
}

/// A transcription on its way: waiting, downloading the model or running,
/// each with a cancel button, or its failure with a retry.
fn transcription_status(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    audio: Option<&PathBuf>,
    progress: &crate::transcribe::Progress,
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::transcribe::Progress;
    let palette = view.palette;
    transcript_divider(ui, &palette, width);
    let chat = view.chat.id.clone();
    let id = message.id.clone();
    let fraction = match progress {
        Progress::Downloading { received, total } if *total > 0 => {
            Some(*received as f32 / *total as f32)
        }
        Progress::Transcribing(percent) => Some(f32::from(*percent) / 100.0),
        _ => None,
    };
    ui.allocate_ui_with_layout(
        vec2(width, 22.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Progress::Failed(error) = progress {
                if let Some(audio) = audio
                    && retry_link(ui, &palette).clicked()
                {
                    actions.push(Action::Transcribe {
                        chat,
                        message: id,
                        path: audio.clone(),
                    });
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    theme::icon(ui, Icon::CircleAlert, 15.0, palette.danger);
                    theme::text(
                        ui,
                        tr("Could not transcribe"),
                        theme::regular(12.5),
                        palette.text,
                    )
                    .on_hover_text(error.as_str());
                });
                return;
            }
            if theme::icon_button(
                ui,
                Icon::X,
                13.0,
                palette.secondary,
                palette.text,
                tr("Cancel"),
            )
            .clicked()
            {
                actions.push(Action::CancelTranscription { chat, message: id });
            }
            let detail = match progress {
                Progress::Downloading { received, total } => tr("{done} of {total}")
                    .replace("{done}", &crate::util::bytes(*received))
                    .replace("{total}", &crate::util::bytes(*total)),
                Progress::Transcribing(percent) => format!("{percent}%"),
                _ => String::new(),
            };
            if !detail.is_empty() {
                theme::text(ui, detail, theme::regular(11.5), palette.secondary);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let label = match progress {
                    Progress::Downloading { .. } => {
                        theme::icon(ui, Icon::Download, 14.0, palette.accent);
                        tr("Downloading the transcription model")
                    }
                    Progress::Waiting => {
                        theme::spinner(ui, 13.0, palette.accent);
                        tr("Waiting for another transcription…")
                    }
                    _ => {
                        theme::spinner(ui, 13.0, palette.accent);
                        tr("Transcribing…")
                    }
                };
                theme::text(ui, label, theme::regular(12.5), palette.text);
            });
        },
    );
    if let Some(fraction) = fraction {
        ui.add_space(2.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, palette.secondary.gamma_multiply(0.25));
        let mut done = rect;
        done.set_width(rect.width() * fraction.clamp(0.0, 1.0));
        ui.painter().rect_filled(done, 2.0, palette.accent);
    }
}

/// "Try again", in the accent colour with its icon.
fn retry_link(ui: &mut egui::Ui, palette: &Palette) -> egui::Response {
    let label = tr("Try again");
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::medium(12.5), palette.accent);
    let size = vec2(galley.size().x + 20.0, 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, 4.0);
    if ui.is_rect_visible(rect) {
        let colour = if response.hovered() {
            palette.accent_hover
        } else {
            palette.accent
        };
        let icon = Rect::from_min_size(rect.min, vec2(14.0, rect.height()));
        theme::paint_icon(ui, Icon::Refresh, icon, 14.0, colour);
        ui.painter().galley(
            pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0),
            galley,
            colour,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A voice message's transcript, selectable, over a row with its language
/// and buttons to copy and fold it. Returns that row, where the message's
/// time goes.
fn transcript_block(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    transcript: &crate::transcribe::Transcript,
    width: f32,
    actions: &mut Vec<Action>,
) -> Rect {
    let palette = view.palette;
    transcript_divider(ui, &palette, width);
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&transcript.text)
                    .font(theme::regular(BODY_SIZE))
                    .color(palette.text),
            )
            .wrap()
            .selectable(true),
        );
    });
    ui.add_space(2.0);
    ui.allocate_ui_with_layout(
        vec2(width, 22.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.x = 4.0;
            theme::icon(ui, Icon::Captions, 13.0, palette.secondary);
            let label = if transcript.language.is_empty() {
                tr("Transcript").to_owned()
            } else {
                format!(
                    "{} · {}",
                    tr("Transcript"),
                    crate::transcribe::language_name(&transcript.language)
                )
            };
            theme::text(ui, label, theme::regular(11.5), palette.secondary);
            if theme::icon_button(
                ui,
                Icon::Copy,
                13.0,
                palette.secondary,
                palette.text,
                tr("Copy transcript"),
            )
            .clicked()
            {
                actions.push(Action::CopyText(transcript.text.clone()));
            }
            if theme::icon_button(
                ui,
                Icon::ChevronUp,
                13.0,
                palette.secondary,
                palette.text,
                tr("Hide transcript"),
            )
            .clicked()
            {
                actions.push(Action::FoldTranscript {
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                });
            }
        },
    )
    .response
    .rect
}

/// Voice-recording controls and live waveform.
fn recording_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (elapsed, levels) = match app.recording.as_ref() {
        Some(recorder) => (recorder.elapsed(), recorder.levels()),
        None => return,
    };
    // As tall as the one-line composer it replaces, so the field keeps its
    // height while recording.
    let line_height = ui
        .painter()
        .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
        .size()
        .y;
    let row_height = (line_height + COMPOSER_PADDING)
        .round()
        .max(COMPOSER_CONTROL);
    let button = row_height;
    // As in WhatsApp: discard at the start, the light and the time, the
    // waveform across the rest, and send at the end.
    ui.allocate_ui_with_layout(
        vec2(ui.available_width().max(0.0), row_height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if theme::circle_button(
                ui,
                Icon::Trash,
                button,
                palette.surface,
                palette.surface_hover,
                palette.secondary,
                tr("Discard"),
            )
            .clicked()
            {
                app.actions.push(Action::CancelRecording);
            }
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            let pulse = 0.55 + 0.45 * (elapsed.as_secs_f32() * 3.0).sin().abs();
            ui.painter()
                .circle_filled(dot.center(), 5.0, palette.danger.gamma_multiply(pulse));
            theme::text(
                ui,
                crate::util::duration(elapsed.as_secs() as u32),
                theme::medium(14.0),
                palette.text,
            );
            // Recent audio levels, newest on the right against send.
            let spacing = ui.spacing().item_spacing.x;
            let wave_width = (ui.available_width() - button - spacing).max(0.0);
            let (rect, _) = ui.allocate_exact_size(vec2(wave_width, 28.0), Sense::hover());
            ui.ctx()
                .data_mut(|data| data.insert_temp(recording_wave_id(), rect));
            let pitch = 3.0;
            let count = (rect.width() / pitch).floor() as usize;
            let shown = &levels[levels.len().saturating_sub(count)..];
            for (index, level) in shown.iter().enumerate() {
                let height = 2.0_f32 + (level * 4.0).min(1.0) * 24.0;
                let x = rect.right() - (shown.len() - index) as f32 * pitch + 1.0;
                ui.painter().rect_filled(
                    Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(2.0, height)),
                    1.0,
                    palette.accent,
                );
            }
            if theme::circle_button(
                ui,
                Icon::Send,
                button,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                tr("Send"),
            )
            .clicked()
            {
                app.actions.push(Action::SendRecording);
            }
        },
    );
}

/// Where the recorder's waveform was drawn, for layout tests.
pub(crate) fn recording_wave_id() -> egui::Id {
    egui::Id::new("recording-wave")
}

/// Whether every selected message is ours, not yet deleted, and still within
/// the time WhatsApp lets us delete it for everyone.
fn all_revocable(conversation: &Conversation, selected: &[String], now: i64) -> bool {
    !selected.is_empty()
        && selected.iter().all(|id| {
            conversation.message(id).is_some_and(|message| {
                message.from_me
                    && !matches!(message.content, Content::Revoked { .. })
                    && now - message.timestamp <= crate::app::REVOKE_WINDOW.as_secs() as i64
            })
        })
}

/// The text of the selected messages, in chat order: one message gives its
/// words; several give a line each, headed like a copy across messages.
pub(crate) fn selection_text(app: &App, chat: &str, selected: &[String]) -> String {
    let Some(conversation) = app.conversations.get(chat) else {
        return String::new();
    };
    let picked: Vec<&Message> = conversation
        .messages
        .iter()
        .filter(|message| selected.contains(&message.id))
        .collect();
    if let [only] = picked.as_slice() {
        return message_body(app, only);
    }
    picked
        .iter()
        .map(|message| headed_line(app, message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The selected messages without an attachment, in chat order and headed
/// with their time and sender, for the text file saved with a selection.
/// Empty when none of them has words of its own.
pub(crate) fn selection_transcript(app: &App, chat: &str, selected: &[String]) -> String {
    let Some(conversation) = app.conversations.get(chat) else {
        return String::new();
    };
    let lines: Vec<String> = conversation
        .messages
        .iter()
        .filter(|message| selected.contains(&message.id) && message.content.media().is_none())
        .filter(|message| !message_body(app, message).trim().is_empty())
        .map(|message| headed_line(app, message))
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

/// A message's words, with mentions spelled out and a marker such as
/// "[photo]" for what is not text.
fn message_body(app: &App, message: &Message) -> String {
    let mentions: Vec<markup::Mention> = message
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: app.mention_name(&mention.id),
        })
        .collect();
    let text = copyable_text(&message.content)
        .map(|text| markup::plain(&text, &mentions))
        .unwrap_or_default();
    match (content_marker(&message.content), text.is_empty()) {
        (Some(marker), true) => marker,
        (Some(marker), false) => format!("{marker} {text}"),
        (None, _) => text,
    }
}

/// "[time] sender: words", as a copy across several messages reads.
fn headed_line(app: &App, message: &Message) -> String {
    let who = if message.from_me {
        app.mention_name(&message.sender)
    } else {
        app.display_name_or(&message.sender, message.sender_name.as_deref())
    };
    format!(
        "[{}] {}: {}",
        crate::util::copy_stamp(message.timestamp),
        who,
        message_body(app, message)
    )
}

/// Replaces the composer while messages are selected.
fn selection_bar(app: &mut App, ui: &mut egui::Ui, chat: &str, selected: &[String]) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            Icon::X,
            18.0,
            palette.secondary,
            palette.text,
            tr("Cancel selection"),
        )
        .clicked()
            || ui.input(|input| input.key_pressed(Key::Escape))
        {
            app.actions.push(Action::CancelSelection);
        }
        let count = if selected.len() == 1 {
            tr("1 selected").to_owned()
        } else {
            tr("{count} selected").replace("{count}", &selected.len().to_string())
        };
        theme::text(ui, &count, theme::medium(14.5), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // From the right: forward, copy, delete, save.
            if theme::icon_button(
                ui,
                Icon::Forward,
                18.0,
                palette.secondary,
                palette.text,
                tr("Forward"),
            )
            .tab_stop(Stop::ForwardSelection)
            .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::Forward {
                    chat: chat.to_owned(),
                    messages: selected.to_vec(),
                }));
            }
            let text = selection_text(app, chat, selected);
            ui.add_enabled_ui(!text.is_empty(), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::Copy,
                    18.0,
                    palette.secondary,
                    palette.text,
                    tr("Copy"),
                )
                .clicked()
                {
                    app.actions.push(Action::CopyText(text));
                }
            });
            let revocable = app.conversations.get(chat).is_some_and(|conversation| {
                all_revocable(conversation, selected, crate::util::now())
            });
            let trash = theme::icon_button(
                ui,
                Icon::Trash,
                18.0,
                palette.secondary,
                palette.text,
                tr("Delete"),
            );
            let confirm = |app: &mut App, for_everyone: bool| {
                app.actions
                    .push(Action::ShowDialog(Dialog::ConfirmDeleteMessage {
                        chat: chat.to_owned(),
                        messages: selected.to_vec(),
                        for_everyone,
                        on_phone: !for_everyone,
                    }));
            };
            if revocable {
                // Both ways are open: the menu asks which.
                let width = widgets::menu_width(
                    ui,
                    &[tr("Delete for everyone"), tr("Delete for me")],
                    true,
                );
                egui::Popup::menu(&trash)
                    .width(width)
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        if widgets::menu_item(
                            ui,
                            &palette,
                            Some(Icon::Trash),
                            tr("Delete for everyone"),
                        ) {
                            confirm(app, true);
                        }
                        if widgets::menu_item(ui, &palette, Some(Icon::EyeOff), tr("Delete for me"))
                        {
                            confirm(app, false);
                        }
                    });
            } else if trash.clicked() {
                confirm(app, false);
            }
            // Attachments are saved as files; the words of the other
            // messages go together in a text file.
            let savable = app.conversations.get(chat).is_some_and(|conversation| {
                selected.iter().any(|id| {
                    conversation
                        .message(id)
                        .is_some_and(|message| message.content.media().is_some())
                })
            }) || !selection_transcript(app, chat, selected).is_empty();
            ui.add_enabled_ui(savable, |ui| {
                if theme::icon_button(
                    ui,
                    Icon::Download,
                    18.0,
                    palette.secondary,
                    palette.text,
                    tr("Save as…"),
                )
                .clicked()
                {
                    app.actions.push(Action::SaveSelection {
                        chat: chat.to_owned(),
                        messages: selected.to_vec(),
                    });
                }
            });
        });
    });
}

/// The file name to suggest when saving an attachment: the sender's name for
/// documents, the cached file's name otherwise. Path separators are dropped so
/// a crafted name cannot point the dialog somewhere else.
pub(crate) fn attachment_name(content: &Content, path: &Path) -> String {
    let name = match content {
        Content::Document { file_name, .. } if !file_name.trim().is_empty() => file_name.clone(),
        _ => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr("attachment").to_owned()),
    };
    name.replace(['/', '\\'], "_")
}

/// Whether a conversation has visible content. Used by tests.
#[allow(dead_code)]
pub fn has_messages(conversation: &Conversation) -> bool {
    !conversation.messages.is_empty()
}

#[allow(dead_code)]
fn status_label(status: Delivery) -> &'static str {
    match status {
        Delivery::None => "",
        Delivery::Pending => "sending",
        Delivery::Sent => "sent",
        Delivery::Delivered => "delivered",
        Delivery::Read => "read",
        Delivery::Played => "played",
        Delivery::Failed => "failed",
    }
}

#[allow(dead_code)]
fn chat_of(chat: &ChatId) -> &str {
    chat
}


/// A wavy underline below a misspelled word in the composer, row by row.
fn paint_misspelling(
    ui: &egui::Ui,
    galley: &egui::Galley,
    origin: egui::Pos2,
    start: usize,
    end: usize,
    color: egui::Color32,
) {
    let mut rows: Vec<Rect> = Vec::new();
    for index in start..end {
        let Some(bounds) = crate::bidi::char_bounds(galley, index, index + 1) else {
            continue;
        };
        match rows.last_mut() {
            Some(row) if (row.bottom() - bounds.bottom()).abs() < 1.0 => *row = row.union(bounds),
            _ => rows.push(bounds),
        }
    }
    let stroke = egui::Stroke::new(1.0, color);
    for row in rows {
        let row = row.translate(origin.to_vec2());
        let y = row.bottom() - 1.0;
        let mut points = Vec::new();
        let mut x = row.left();
        let mut up = false;
        while x < row.right() {
            points.push(egui::pos2(x, if up { y - 1.5 } else { y }));
            x += 2.0;
            up = !up;
        }
        points.push(egui::pos2(row.right(), if up { y - 1.5 } else { y }));
        ui.painter().add(egui::Shape::line(points, stroke));
    }
}

#[cfg(test)]
mod tests {

    /// A video's poster from the phone is about a hundred pixels wide; it is
    /// enlarged before it is drawn, and a large one is left alone.
    #[test]
    fn a_small_video_poster_is_enlarged_and_a_large_one_is_kept() {
        let jpeg = |width: u32, height: u32| {
            let picture = image::RgbImage::from_fn(width, height, |x, y| {
                image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 90])
            });
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut bytes)
                .encode_image(&picture)
                .unwrap();
            bytes
        };
        let enlarged = enlarge_poster(&jpeg(96, 54)).expect("a small poster grows");
        let (width, height) = image::load_from_memory(&enlarged)
            .unwrap()
            .to_rgb8()
            .dimensions();
        assert_eq!((width, height), (SMOOTH_POSTER_SIDE, 270));
        assert!(
            enlarge_poster(&jpeg(640, 360)).is_none(),
            "a large poster stays as it is"
        );
        assert!(enlarge_poster(b"not a picture").is_none());
    }
    use super::*;

    #[test]
    fn sender_pictures_show_in_groups_only() {
        let chat = |id: &str| Chat::new(id.into(), "Chat".into());
        assert!(shows_sender_pictures(&chat("120363012345678901@g.us")));
        assert!(!shows_sender_pictures(&chat("393331234567@s.whatsapp.net")));
        assert!(!shows_sender_pictures(&chat(
            "120363055566677788@newsletter"
        )));
    }

    fn contact(name: &str, number: &str, account: Option<&str>) -> Option<SharedContact> {
        Some(SharedContact {
            name: name.to_owned(),
            number: number.to_owned(),
            account: account.map(str::to_owned),
            first_name: None,
        })
    }

    #[test]
    fn a_shared_contact_keeps_the_first_name_of_its_card() {
        let first = |card: &str| {
            shared_contact_details(card, "Fallback")
                .expect("a contact")
                .first_name
        };
        // A first name of two words stays whole, as the card has it (#314).
        assert_eq!(
            first("BEGIN:VCARD\nN:;My Dih;;;\nFN:My Dih\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("My Dih")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:Evans;Mary;Ann;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("Mary Ann")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:Smith;Ada\\;Jo;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("Ada;Jo")
        );
        assert_eq!(
            first("BEGIN:VCARD\nN:山田;太郎;;;\nTEL:+15550101234\nEND:VCARD").as_deref(),
            Some("太郎")
        );
        // Without a structured first name nothing is known.
        assert_eq!(
            first("BEGIN:VCARD\nN:Evans;;;;\nTEL:+15550101234\nEND:VCARD"),
            None
        );
        assert_eq!(
            first("BEGIN:VCARD\nFN:Ada Lovelace\nTEL:+15550101234\nEND:VCARD"),
            None
        );
    }

    #[test]
    fn shared_contact_accepts_case_insensitive_vcard_property_names() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nversion:3.0\nitem1.fn:Case-insensitive name\nitem1.tel;type=cell;waid=1234567:+1 234567\nEND:VCARD",
                "Fallback",
            ),
            contact("Case-insensitive name", "+1 234567", Some("1234567"))
        );
    }

    #[test]
    fn shared_contact_uses_first_valid_telephone_when_multiple_are_present() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Two numbers\nTEL;TYPE=HOME:+1111111\nTEL;TYPE=CELL:+2222222\nEND:VCARD",
                "Fallback",
            ),
            contact("Two numbers", "+1111111", Some("1111111"))
        );
    }

    #[test]
    fn shared_contact_prefers_lowest_vcard_tel_preference() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Preferred number\nTEL;TYPE=HOME:+1111111\nitem1.TEL;TYPE=CELL;PREF=2:+2222222\nTEL;TYPE=WORK;PREF=1:+3333333\nEND:VCARD",
                "Fallback",
            ),
            contact("Preferred number", "+3333333", Some("3333333"))
        );
    }

    #[test]
    fn shared_contact_prefers_vcard_name_and_reads_the_account_from_waid() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nVERSION:3.0\nFN:Alice Example\nitem1.TEL;waid=15550101234:+1 (555) 010-1234\nEND:VCARD",
                "Contact from sender",
            ),
            contact("Alice Example", "+1 (555) 010-1234", Some("15550101234"))
        );
    }

    #[test]
    fn a_local_number_names_no_account_and_stays_readable() {
        // Without a country code the digits are not a WhatsApp account.
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Local\nTEL;TYPE=CELL:0171 1234567\nEND:VCARD",
                "Fallback",
            ),
            contact("Local", "0171 1234567", None)
        );
    }

    #[test]
    fn shared_contact_without_vcard_name_uses_message_name_and_rejects_missing_phone() {
        assert_eq!(
            shared_contact_details("BEGIN:VCARD\nTEL:+1234567\nEND:VCARD", "Shared person"),
            contact("Shared person", "+1234567", Some("1234567"))
        );
        assert_eq!(
            shared_contact_details("BEGIN:VCARD\nFN:No phone\nEND:VCARD", "Fallback"),
            None
        );
    }

    #[test]
    fn shared_contact_details_use_the_first_vcard_when_multiple_contacts_are_shared() {
        assert_eq!(
            shared_contact_details(
                "BEGIN:VCARD\nFN:Alice\nTEL:+15550101234\nEND:VCARD\nBEGIN:VCARD\nFN:Bob\nTEL:+15550105678\nEND:VCARD",
                "Several contacts",
            ),
            contact("Alice", "+15550101234", Some("15550101234"))
        );
    }

    #[test]
    fn saved_attachments_suggest_a_plain_file_name() {
        let document = |file_name: &str| Content::Document {
            media: media(None, None),
            file_name: file_name.into(),
            caption: None,
            pages: None,
        };
        let cached = Path::new("/cache/media/abc123.pdf");
        assert_eq!(attachment_name(&document("Notes.pdf"), cached), "Notes.pdf");
        assert_eq!(
            attachment_name(&document("../../.bashrc"), cached),
            ".._.._.bashrc"
        );
        assert_eq!(attachment_name(&document("  "), cached), "abc123.pdf");
    }

    fn media(w: Option<u32>, h: Option<u32>) -> Media {
        Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: w,
            height: h,
            path: None,
            state: MediaState::Idle,
        }
    }

    #[test]
    fn picture_placeholder_matches_decoded_dimensions() {
        for limit in [200.0, PICTURE_WIDTH] {
            for (width, height) in [(900, 1200), (600, 1600), (1600, 900)] {
                assert_eq!(
                    frame_size(
                        &media(Some(width), Some(height)),
                        None,
                        limit,
                        PICTURE_HEIGHT
                    ),
                    fit_picture(width as f32, height as f32, limit, PICTURE_HEIGHT),
                    "decoding a {width}x{height} photo must not shift the transcript"
                );
            }
        }
    }

    #[test]
    fn reaction_affordance_action_targets_the_message_it_belongs_to() {
        let action = open_reaction_picker_action("chat@example", "message-42");
        assert!(matches!(
            action,
            Action::OpenReactionPicker { chat, message, beside_menu: false }
                if chat == "chat@example" && message == "message-42"
        ));
    }

    #[test]
    fn skin_tone_reactions_share_one_chip_in_our_tone() {
        let react = |sender: &str, from_me: bool, emoji: &str| Reaction {
            sender: sender.to_owned(),
            from_me,
            emoji: emoji.to_owned(),
        };
        let who = |reaction: &Reaction| reaction.sender.clone();
        // Ours comes last: its tone wins and the count is summed.
        let chips = reaction_chips(&[react("ana", false, "🙏"), react("me", true, "🙏🏽")], who);
        assert_eq!(
            chips,
            [("🙏🏽".to_owned(), 2, true, vec!["ana".into(), "me".into()])]
        );
        // Without ours, the first one seen stays; other emoji keep their chip.
        let chips = reaction_chips(
            &[
                react("ana", false, "🙏🏽"),
                react("bia", false, "🙏"),
                react("caio", false, "👍"),
            ],
            who,
        );
        assert_eq!(chips.len(), 2);
        assert_eq!(
            (chips[0].0.as_str(), chips[0].1, chips[0].2),
            ("🙏🏽", 2, false)
        );
        assert_eq!((chips[1].0.as_str(), chips[1].1), ("👍", 1));
    }

    #[test]
    fn reaction_button_label_names_whose_message_it_reacts_to() {
        assert_eq!(reaction_button_label(true, "Ada"), "React to your message");
        assert_eq!(
            reaction_button_label(false, "Ada"),
            "React to Ada's message"
        );
    }

    #[test]
    fn reaction_affordance_sits_beside_the_bubble_without_clipping() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let incoming = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let outgoing = Rect::from_min_max(pos2(260.0, 20.0), pos2(380.0, 80.0));

        let incoming_button = reaction_affordance_rect(incoming, bounds, false);
        let outgoing_button = reaction_affordance_rect(outgoing, bounds, true);

        assert!(incoming_button.left() > incoming.right());
        assert!(outgoing_button.right() < outgoing.left());
        assert!(bounds.contains_rect(incoming_button));
        assert!(bounds.contains_rect(outgoing_button));

        let edge = Rect::from_min_max(pos2(360.0, 260.0), pos2(398.0, 298.0));
        assert!(bounds.contains_rect(reaction_affordance_rect(edge, bounds, false)));
    }

    #[test]
    fn reaction_affordance_stays_visible_while_crossing_to_its_button() {
        let bubble = Rect::from_min_max(pos2(20.0, 20.0), pos2(140.0, 80.0));
        let button = Rect::from_min_max(pos2(146.0, 24.0), pos2(172.0, 50.0));

        assert!(reaction_affordance_visible(
            Some(bubble.center()),
            bubble,
            button
        ));
        assert!(reaction_affordance_visible(
            Some(button.center()),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(
            Some(pos2(300.0, 200.0)),
            bubble,
            button
        ));
        assert!(!reaction_affordance_visible(None, bubble, button));
    }

    #[test]
    fn picture_frames_keep_their_shape_within_the_limit() {
        let landscape = frame_size(&media(Some(1600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!((landscape.x - 340.0).abs() < 0.01);
        assert!((landscape.y - 255.0).abs() < 0.01);
        let tall = frame_size(&media(Some(600), Some(1200)), None, 340.0, PICTURE_HEIGHT);
        assert!(tall.y > 340.0 && tall.y <= PICTURE_HEIGHT);
        let exact = fit_picture(900.0, 1600.0, PICTURE_WIDTH, PICTURE_HEIGHT);
        assert!((exact.y - PICTURE_HEIGHT).abs() < 0.01);
        assert!(exact.x < PICTURE_WIDTH);
        let unknown = frame_size(&media(None, None), Some((16, 9)), 340.0, PICTURE_HEIGHT);
        assert!(unknown.x > unknown.y);
        let tiny = frame_size(&media(Some(40), Some(40)), None, 340.0, PICTURE_HEIGHT);
        assert!(tiny.x >= 120.0);
    }

    #[test]
    fn a_failed_footer_reserves_room_for_its_label() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut message = Message {
            id: "fixture".into(),
            chat: "1@s.whatsapp.net".into(),
            sender: "me@s.whatsapp.net".into(),
            sender_name: None,
            from_me: true,
            timestamp: 1000,
            content: Content::text("Fixture"),
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        let mut widths = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            widths.push(footer_width(ui, &message));
            message.status = Delivery::Failed;
            widths.push(footer_width(ui, &message));
            // Only our own messages can fail to send.
            message.from_me = false;
            widths.push(footer_width(ui, &message) + 19.0);
        });
        output.textures_delta.clear();
        assert!(widths[1] > widths[0] + 30.0, "{widths:?}");
        assert_eq!(widths[2], widths[0]);
    }

    #[test]
    fn stickers_follow_their_own_switches_within_the_size_limit() {
        use crate::settings::AutoDownloadKind;
        let settings = crate::settings::Settings {
            auto_download_image: false,
            auto_download_animated_sticker: true,
            ..Default::default()
        };
        let mut sticker = media(Some(180), Some(180));
        assert!(auto_download_allowed(
            &sticker,
            AutoDownloadKind::AnimatedSticker,
            &settings
        ));
        // Static stickers download as images do.
        assert!(!auto_download_allowed(
            &sticker,
            AutoDownloadKind::Image,
            &settings
        ));
        sticker.size = crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1;
        assert!(!auto_download_allowed(
            &sticker,
            AutoDownloadKind::AnimatedSticker,
            &settings
        ));
    }

    /// The typing dots animate at the display's rate, not a fixed timer.
    #[test]
    fn typing_dots_repaint_every_frame() {
        let ctx = egui::Context::default();
        let palette = crate::theme::Palette::dark();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| typing_dots(ui, &palette));
        output.textures_delta.clear();
        let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    #[test]
    fn composer_triggers_only_start_at_word_boundaries() {
        assert_eq!(standalone_trigger(":", 1, ':'), Some(0));
        assert_eq!(standalone_trigger("hello :", 7, ':'), Some(6));
        assert_eq!(standalone_trigger("hello @", 7, '@'), Some(6));
        assert_eq!(standalone_trigger("19:30", 3, ':'), None);
        assert_eq!(standalone_trigger("mail@example.com", 5, '@'), None);
    }

    #[test]
    fn mention_query_runs_from_the_at_to_the_cursor() {
        assert_eq!(active_mention("hello @mi", Some(6), 9), Some((9, "mi")));
        assert_eq!(active_mention("hello @mi\n", Some(6), 10), None);
        assert_eq!(active_mention("hello @mi", Some(99), 9), None);
    }

    #[test]
    fn emoji_query_ends_at_spaces_and_punctuation() {
        assert_eq!(active_emoji("hello :gri", Some(6), 10), Some((10, "gri")));
        assert_eq!(
            active_emoji("hello :gri_ning", Some(6), 15),
            Some((15, "gri_ning"))
        );
        assert_eq!(active_emoji("hello :gri ", Some(6), 11), None);
        assert_eq!(active_emoji("hello :gri!", Some(6), 11), None);
        assert_eq!(active_emoji("hello gri", Some(6), 9), None);
    }

    #[test]
    fn emoji_shortcodes_rank_ahead_of_name_matches() {
        let grinning = emojis::get("😀").expect("known emoji");
        assert_eq!(emoji_match_score(grinning, "grinning"), Some(0));
        assert_eq!(emoji_match_score(grinning, "grin"), Some(1));
        assert_eq!(emoji_match_score(grinning, "face"), Some(3));
        assert_eq!(emoji_match_score(grinning, "rocket"), None);
    }

    #[test]
    fn completion_offers_skin_tone_capable_emoji() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        let offers = |query: &str, emoji: &str| {
            emoji_candidates(&app, query)
                .iter()
                .any(|suggestion| suggestion.emoji == emoji)
        };
        assert!(offers("pregnant", "🤰"));
        assert!(offers("thumbsup", "👍"));
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::*;
    use crate::model::{Delivery, Reaction};

    fn with_reactions(reactions: Vec<Reaction>) -> Message {
        Message {
            id: "m1".into(),
            chat: "a@s.whatsapp.net".into(),
            sender: "a@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            content: Content::text("hi"),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions,
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    fn reaction(from_me: bool, emoji: &str) -> Reaction {
        Reaction {
            sender: if from_me { "me" } else { "them" }.into(),
            from_me,
            emoji: emoji.into(),
        }
    }

    #[test]
    fn only_our_own_reaction_counts_as_chosen() {
        let message = with_reactions(vec![reaction(false, "😂"), reaction(true, "❤️")]);
        assert_eq!(own_reaction(&message), Some("❤️"));
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
        assert_eq!(
            own_reaction(&with_reactions(vec![reaction(false, "😂")])),
            None
        );
    }

    #[test]
    fn quick_reactions_start_with_preferences_and_fill_with_unique_defaults() {
        let message = with_reactions(Vec::new());
        let preferred = vec![("🦀".into(), 5), ("👍".into(), 2), ("invalid".into(), 1)];
        let quick = quick_reactions(&message, &preferred);
        assert_eq!(&quick[..2], &["🦀", "👍"]);
        assert_eq!(quick.len(), 6);
        assert_eq!(quick.iter().filter(|&&emoji| emoji == "👍").count(), 1);
    }

    #[test]
    fn an_unusual_reaction_of_ours_joins_the_quick_row() {
        let message = with_reactions(vec![reaction(true, "🦀")]);
        let quick = quick_reactions(&message, &[]);
        assert_eq!(quick.len(), QUICK_REACTIONS.len() + 1);
        assert_eq!(quick.last(), Some(&"🦀"));
    }

    #[test]
    fn reaction_choice_toggles_or_replaces_our_emoji() {
        assert_eq!(reaction_choice(None, "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("🦀"), "🦀"), "");
        assert_eq!(reaction_choice(Some("👍"), "🦀"), "🦀");
        assert_eq!(reaction_choice(Some("❤️"), "❤️"), "");
    }

    #[test]
    fn another_senders_custom_reaction_is_kept_on_the_message() {
        let message = with_reactions(vec![reaction(false, "🏆")]);
        assert_eq!(own_reaction(&message), None);
        assert_eq!(message.reactions[0].emoji, "🏆");
        assert!(!message.reactions[0].from_me);
        assert_eq!(quick_reactions(&message, &[]), QUICK_REACTIONS.to_vec());
    }
}

/// Side of a tile in the strip of attachments waiting to be sent.
const PENDING_TILE: f32 = 64.0;

/// Attachments waiting to be sent, over the history as on the phone: the
/// chosen one large, and all of them in a strip below, to pick, remove or
/// add to. The caption is typed in the composer underneath.
fn pending_preview(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    ui.painter().rect_filled(area, 0.0, palette.panel);
    let count = app.pending.len();
    // A file just added is the one shown.
    let selected_id = egui::Id::new("pending-selected");
    let (selected, seen) = ui.ctx().data(|data| {
        (
            data.get_temp::<usize>(selected_id).unwrap_or(0),
            data.get_temp::<usize>(selected_id.with("count"))
                .unwrap_or(0),
        )
    });
    let mut selected = if count > seen { count - 1 } else { selected }.min(count - 1);
    forget_pending_posters(ui.ctx(), &app.pending);

    let top = Rect::from_min_size(area.min, vec2(area.width(), 52.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(top.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                20.0,
                palette.secondary,
                palette.text,
                tr("Discard (Esc)"),
            )
            .clicked()
            {
                app.actions.push(Action::ClearPending);
            }
        },
    );
    let title = match &app.pending[selected] {
        crate::app::Pending::Picture { .. } => tr("Pasted picture").to_owned(),
        crate::app::Pending::File(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let title = widgets::line(
        ui,
        &title,
        theme::semibold(14.0),
        palette.text,
        (area.width() - 140.0).max(40.0),
        1,
    );
    title.paint(ui, top.center() - title.size() / 2.0, palette.text);

    let strip = Rect::from_min_max(
        egui::pos2(area.left(), area.bottom() - PENDING_TILE - 24.0),
        area.right_bottom(),
    );
    let stage = Rect::from_min_max(
        egui::pos2(area.left() + 24.0, top.bottom() + 4.0),
        egui::pos2(area.right() - 24.0, strip.top() - 8.0),
    );
    if stage.width() > 16.0 && stage.height() > 16.0 {
        let item = &mut app.pending[selected];
        pending_large(ui, &palette, item, selected, stage);
    }

    let tiles = count as f32 + 1.0;
    let row = tiles * PENDING_TILE + (tiles - 1.0) * 8.0;
    let mut x = (strip.center().x - row / 2.0).max(strip.left() + 12.0);
    let y = strip.top() + 8.0;
    let mut remove = None;
    for index in 0..count {
        let rect = Rect::from_min_size(egui::pos2(x, y), Vec2::splat(PENDING_TILE));
        x += PENDING_TILE + 8.0;
        let response = ui
            .interact(rect, ui.id().with(("pending-tile", index)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            selected = index;
            // The caption keeps the keyboard.
            app.focus_composer = true;
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        ui.painter().rect_filled(rect, 8.0, palette.surface);
        pending_thumbnail(
            ui,
            &palette,
            &mut app.pending[index],
            index,
            rect.shrink(3.0),
        );
        if index == selected {
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        // Remove button in the corner.
        let close = Rect::from_center_size(rect.right_top() + vec2(-10.0, 10.0), Vec2::splat(18.0));
        let close_response = ui
            .interact(close, ui.id().with(("unstage", index)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("Remove"));
        ui.painter()
            .circle_filled(close.center(), 9.0, palette.overlay);
        theme::paint_icon(ui, Icon::X, close, 12.0, palette.text);
        if close_response.clicked() {
            remove = Some(index);
            app.focus_composer = true;
        }
    }
    // More files join the same message.
    let add = Rect::from_min_size(egui::pos2(x, y), Vec2::splat(PENDING_TILE));
    let add_response = ui
        .interact(add, ui.id().with("pending-add"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::gettext(app.locale, "Attach").as_ref());
    ui.painter().rect_stroke(
        add,
        8.0,
        Stroke::new(
            1.5,
            if add_response.hovered() {
                palette.accent
            } else {
                palette.outline
            },
        ),
        egui::StrokeKind::Inside,
    );
    theme::paint_icon(ui, Icon::Plus, add, 22.0, palette.secondary);
    if add_response.clicked() {
        app.actions.push(Action::Attach);
    }

    if let Some(index) = remove {
        app.actions.push(Action::RemovePending(index));
        if selected > index || selected + 1 == count {
            selected = selected.saturating_sub(1);
        }
    }
    let remaining = if remove.is_some() { count - 1 } else { count };
    ui.ctx().data_mut(|data| {
        data.insert_temp(selected_id, selected);
        data.insert_temp(selected_id.with("count"), remaining);
    });
}

/// The largest side a pasted picture's preview texture is made at.
const PENDING_TEXTURE_SIDE: usize = 2048;

/// A pasted picture's preview texture, made once, no larger than
/// [`PENDING_TEXTURE_SIDE`] (and the GPU's limit).
fn pending_texture(
    ctx: &egui::Context,
    index: usize,
    width: usize,
    height: usize,
    rgba: &[u8],
    texture: &mut Option<egui::TextureHandle>,
) -> egui::TextureId {
    texture
        .get_or_insert_with(|| {
            let limit = ctx
                .input(|input| input.max_texture_side)
                .min(PENDING_TEXTURE_SIDE);
            let image = if width > limit || height > limit {
                let scale = limit as f32 / width.max(height) as f32;
                let (w, h) = (
                    ((width as f32 * scale) as u32).max(1),
                    ((height as f32 * scale) as u32).max(1),
                );
                match image::RgbaImage::from_raw(width as u32, height as u32, rgba.to_vec()) {
                    Some(full) => {
                        let small = image::imageops::resize(
                            &full,
                            w,
                            h,
                            image::imageops::FilterType::Triangle,
                        );
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &small)
                    }
                    None => egui::ColorImage::example(),
                }
            } else {
                egui::ColorImage::from_rgba_unmultiplied([width, height], rgba)
            };
            ctx.load_texture(
                format!("pending-picture-{index}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        })
        .id()
}

/// Longest side of a video's first frame in the attachment preview.
const PENDING_POSTER_SIDE: u32 = 1280;

/// The first frames of the videos in the attachment preview, read on a
/// thread each.
#[derive(Clone, Default)]
struct PendingPosters(std::sync::Arc<std::sync::Mutex<HashMap<PathBuf, PosterSlot>>>);

enum PosterSlot {
    Reading,
    Read(Option<(egui::ColorImage, std::time::Duration)>),
    Shown(Option<(egui::TextureHandle, std::time::Duration)>),
}

fn pending_posters(ctx: &egui::Context) -> PendingPosters {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<PendingPosters>(egui::Id::new("pending-posters"))
            .clone()
    })
}

/// A pending video's first frame and length: `None` while it is read,
/// `Some(None)` when no decoder here reads it.
fn pending_poster(
    ctx: &egui::Context,
    path: &Path,
) -> Option<Option<(egui::TextureHandle, std::time::Duration)>> {
    let posters = pending_posters(ctx);
    let mut slots = posters.0.lock().ok()?;
    match slots.get_mut(path) {
        None => {
            slots.insert(path.to_owned(), PosterSlot::Reading);
            let (file, posters, ctx) = (path.to_owned(), posters.clone(), ctx.clone());
            let spawned = std::thread::Builder::new()
                .name("video-poster".into())
                .spawn(move || {
                    let poster = crate::video::poster(&file, PENDING_POSTER_SIDE);
                    if let Ok(mut slots) = posters.0.lock()
                        && let Some(slot) = slots.get_mut(&file)
                    {
                        *slot = PosterSlot::Read(poster);
                    }
                    ctx.request_repaint();
                });
            if spawned.is_err() {
                slots.insert(path.to_owned(), PosterSlot::Shown(None));
            }
            None
        }
        Some(PosterSlot::Reading) => None,
        Some(slot @ PosterSlot::Read(_)) => {
            let PosterSlot::Read(poster) = std::mem::replace(slot, PosterSlot::Reading) else {
                return None;
            };
            let shown = poster.map(|(image, length)| {
                let texture = ctx.load_texture(
                    format!("pending-poster-{}", path.display()),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                (texture, length)
            });
            *slot = PosterSlot::Shown(shown.clone());
            Some(shown)
        }
        Some(PosterSlot::Shown(shown)) => Some(shown.clone()),
    }
}

/// Forgets the first frames of videos no longer attached.
fn forget_pending_posters(ctx: &egui::Context, pending: &[crate::app::Pending]) {
    if let Ok(mut slots) = pending_posters(ctx).0.lock() {
        slots.retain(|path, _| {
            pending
                .iter()
                .any(|item| matches!(item, crate::app::Pending::File(file) if file == path))
        });
    }
}

/// A play sign over a video's first frame, with its length below it.
fn paint_poster_marks(ui: &egui::Ui, rect: Rect, length: std::time::Duration, large: bool) {
    let side = if large { 56.0 } else { 22.0 };
    let disc = Rect::from_center_size(rect.center(), Vec2::splat(side));
    ui.painter()
        .circle_filled(disc.center(), side / 2.0, Color32::from_black_alpha(140));
    theme::paint_icon(ui, Icon::Play, disc, side * 0.45, Color32::WHITE);
    if !large || length.is_zero() {
        return;
    }
    let seconds = (length.as_secs_f64().round() as u32).max(1);
    let galley = ui.painter().layout_no_wrap(
        crate::util::duration(seconds),
        theme::medium(12.0),
        Color32::WHITE,
    );
    let chip = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.bottom() - galley.size().y - 16.0),
        galley.size() + vec2(12.0, 6.0),
    );
    ui.painter()
        .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
    ui.painter()
        .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
}

/// Fits a picture of `pixels` in `bounds`, keeping its shape, never larger
/// than its own size on screen.
fn fit_contain(pixels: Vec2, bounds: Vec2, pixels_per_point: f32) -> Vec2 {
    let pixels = pixels.max(Vec2::splat(1.0));
    let scale = (bounds.x / pixels.x)
        .min(bounds.y / pixels.y)
        .min(1.0 / pixels_per_point.max(0.1));
    pixels * scale
}

/// The chosen attachment, as large as `stage` allows.
fn pending_large(
    ui: &mut egui::Ui,
    palette: &Palette,
    item: &mut crate::app::Pending,
    index: usize,
    stage: Rect,
) {
    let ppp = ui.ctx().pixels_per_point();
    match item {
        crate::app::Pending::Picture {
            width,
            height,
            rgba,
            texture,
        } => {
            let id = pending_texture(ui.ctx(), index, *width, *height, rgba, texture);
            let size = fit_contain(vec2(*width as f32, *height as f32), stage.size(), ppp);
            egui::Image::from_texture((id, size))
                .corner_radius(6.0)
                .paint_at(ui, Rect::from_center_size(stage.center(), size));
        }
        crate::app::Pending::File(path) => {
            if crate::app::Pending::is_video_file(path) {
                match pending_poster(ui.ctx(), path) {
                    Some(Some((texture, length))) => {
                        let size = fit_contain(texture.size_vec2(), stage.size(), ppp);
                        let rect = Rect::from_center_size(stage.center(), size);
                        egui::Image::from_texture((texture.id(), size))
                            .corner_radius(6.0)
                            .paint_at(ui, rect);
                        paint_poster_marks(ui, rect, length, true);
                        return;
                    }
                    None => {
                        theme::paint_spinner(ui, stage, 28.0, palette.accent);
                        return;
                    }
                    Some(None) => {}
                }
            }
            if crate::app::Pending::is_picture_file(path) {
                let image = widgets::file_image(ui, path);
                match image.load_for_size(ui.ctx(), stage.size()) {
                    Ok(egui::load::TexturePoll::Ready { texture }) => {
                        let size = fit_contain(texture.size, stage.size(), ppp);
                        egui::Image::from_texture(texture)
                            .corner_radius(6.0)
                            .paint_at(ui, Rect::from_center_size(stage.center(), size));
                        return;
                    }
                    Ok(egui::load::TexturePoll::Pending { .. }) => {
                        theme::paint_spinner(ui, stage, 28.0, palette.accent);
                        return;
                    }
                    Err(_) => {}
                }
            }
            // Anything else goes as a document: its icon, name and size.
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let icon = Rect::from_center_size(stage.center() - vec2(0.0, 40.0), Vec2::splat(80.0));
            widgets::paint_file_badge(ui, palette, icon, &name);
            let line = widgets::line(
                ui,
                &name,
                theme::semibold(15.0),
                palette.text,
                (stage.width() - 32.0).max(40.0),
                2,
            );
            let at = egui::pos2(stage.center().x - line.size().x / 2.0, icon.bottom() + 12.0);
            line.paint(ui, at, palette.text);
            if let Ok(metadata) = std::fs::metadata(&*path) {
                ui.painter().text(
                    egui::pos2(stage.center().x, at.y + line.size().y + 14.0),
                    Align2::CENTER_CENTER,
                    crate::util::bytes(metadata.len()),
                    theme::regular(12.5),
                    palette.secondary,
                );
            }
        }
    }
}

/// An attachment's small picture in the strip, filling `rect`.
fn pending_thumbnail(
    ui: &mut egui::Ui,
    palette: &Palette,
    item: &mut crate::app::Pending,
    index: usize,
    rect: Rect,
) {
    match item {
        crate::app::Pending::Picture {
            width,
            height,
            rgba,
            texture,
        } => {
            let id = pending_texture(ui.ctx(), index, *width, *height, rgba, texture);
            egui::Image::from_texture((id, vec2(*width as f32, *height as f32)))
                .uv(cover_uv(vec2(*width as f32, *height as f32), rect.size()))
                .corner_radius(6.0)
                .paint_at(ui, rect);
        }
        crate::app::Pending::File(path) => {
            if crate::app::Pending::is_video_file(path)
                && let Some(Some((texture, length))) = pending_poster(ui.ctx(), path)
            {
                egui::Image::from_texture((texture.id(), texture.size_vec2()))
                    .uv(cover_uv(texture.size_vec2(), rect.size()))
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
                paint_poster_marks(ui, rect, length, false);
                return;
            }
            if crate::app::Pending::is_picture_file(path)
                && let Ok(egui::load::TexturePoll::Ready { texture }) =
                    widgets::file_image(ui, path).load_for_size(ui.ctx(), rect.size())
            {
                egui::Image::from_texture(texture)
                    .uv(cover_uv(texture.size, rect.size()))
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
                return;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let icon = Rect::from_center_size(rect.center() - vec2(0.0, 8.0), Vec2::splat(30.0));
            widgets::paint_file_badge(ui, palette, icon, &name);
            let line = widgets::line(
                ui,
                &name,
                theme::regular(10.5),
                palette.text,
                rect.width() - 4.0,
                1,
            );
            line.paint(
                ui,
                egui::pos2(rect.center().x - line.size().x / 2.0, rect.bottom() - 16.0),
                palette.text,
            );
        }
    }
}
