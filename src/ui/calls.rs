//! The call log: every chat's call rows in one list, shown in place of the
//! chat list, and the pieces the call rows in a conversation share with it.

use crate::i18n::tr;
use egui::{Frame, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::archive::LoggedCall;
use crate::model::{Action, Chat, ChatKind, Content, Message};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::widgets;

/// The icon of a call row: a camera for video, people for a group call.
pub fn icon(content: &Content) -> Icon {
    match content {
        Content::Call { video: true, .. } => Icon::Video,
        Content::Call { group: Some(_), .. } => Icon::Users,
        _ => Icon::Phone,
    }
}

/// Whether ZapFast can call back the other side of a logged call: a voice
/// call between two people, with a contact who can be called from here.
pub fn can_call_back(content: &Content, chat: &Chat) -> bool {
    matches!(
        content,
        Content::Call {
            video: false,
            group: None,
            ..
        }
    ) && chat.kind == ChatKind::Direct
        && !chat.blocked
        && crate::calls::capabilities().voice
}

/// The menu of a call row: call back, and delete the row here.
pub fn row_menu(
    ui: &mut egui::Ui,
    palette: &Palette,
    chat: &Chat,
    message: &Message,
    actions: &mut Vec<Action>,
) {
    if can_call_back(&message.content, chat)
        && widgets::menu_item(ui, palette, Some(Icon::Phone), tr("Call back"))
    {
        actions.push(Action::StartCall(chat.id.clone()));
    }
    if widgets::menu_item(ui, palette, Some(Icon::Trash), tr("Delete from call log")) {
        actions.push(Action::RemoveCall {
            chat: chat.id.clone(),
            id: message.id.clone(),
        });
    }
}

/// The button to the call log, beside the archive's, with the number of
/// missed calls since the log was last opened.
/// It is there once a call is logged, as the archive's is once something is
/// archived.
pub fn button(app: &mut App, ui: &mut egui::Ui) {
    if app.show_archived || app.locked_folder || app.show_calls || app.call_log.is_empty() {
        return;
    }
    let palette = app.palette;
    let missed = app.missed_calls();
    let hint = if missed > 0 {
        crate::i18n::ngettext(
            app.locale,
            "Calls ({} missed)",
            "Calls ({} missed)",
            missed as u32,
        )
        .replace("{}", &missed.to_string())
    } else {
        tr("Calls").to_owned()
    };
    let response = theme::icon_button(
        ui,
        Icon::Phone,
        18.0,
        palette.secondary,
        palette.text,
        &hint,
    )
    .tab_stop(Stop::Calls);
    if missed > 0 {
        let label = if missed > 9 {
            "9+".to_owned()
        } else {
            missed.to_string()
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, theme::semibold(9.0), palette.on_accent);
        let width = (galley.size().x + 6.0).max(14.0);
        let rect = Rect::from_center_size(
            response.rect.right_top() + vec2(-3.0, 4.0),
            vec2(width, 14.0),
        );
        ui.painter().rect_filled(rect, 7.0, palette.danger);
        ui.painter().galley(
            rect.center() - galley.size() / 2.0,
            galley,
            palette.on_accent,
        );
    }
    if response.clicked() {
        app.actions.push(Action::ShowCalls(true));
    }
}

/// Whether the call log screen shows only missed calls.
fn missed_only_id() -> egui::Id {
    egui::Id::new("call-log-missed-only")
}

/// The call log in place of the chat list: its header, the All and Missed
/// chips, and one row per call, newest first.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let missed_only = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(missed_only_id()))
        .unwrap_or(false);
    let inset = if theme::macos_chrome(ui.ctx()) {
        (theme::traffic_light_inset(ui.ctx()) - 14.0).max(0.0)
    } else {
        0.0
    };
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(inset);
                if theme::icon_button(
                    ui,
                    Icon::ArrowLeft,
                    18.0,
                    palette.secondary,
                    palette.text,
                    tr("Back to chats"),
                )
                .tab_stop(Stop::Back)
                .clicked()
                {
                    app.actions.push(Action::ShowCalls(false));
                }
                theme::text(ui, tr("Calls"), theme::bold(20.0), palette.text);
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                let missed = app.missed_calls();
                if widgets::filter_chip(ui, &palette, tr("All"), 0, !missed_only)
                    .tab_stop(Stop::All)
                    .clicked()
                {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(missed_only_id(), false));
                }
                if widgets::filter_chip(ui, &palette, tr("Missed calls"), missed, missed_only)
                    .tab_stop(Stop::Unread)
                    .clicked()
                {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(missed_only_id(), true));
                }
            });
        });
    let calls: Vec<LoggedCall> = app
        .listed_calls()
        .into_iter()
        .filter(|call| !missed_only || missed(call))
        .cloned()
        .collect();
    if calls.is_empty() {
        let (title, body) = if missed_only {
            (tr("No missed calls"), tr("Choose All to see every call."))
        } else {
            (
                tr("No calls yet"),
                tr("Calls made or received here or on your phone appear here."),
            )
        };
        widgets::empty_state(ui, &palette, Icon::Phone, title, body);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("call-log")
        .auto_shrink([false, false])
        .show_rows(ui, theme::ROW_HEIGHT, calls.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for call in &calls[range] {
                row(app, ui, call);
            }
        });
}

/// Whether a logged call rang unanswered here.
fn missed(call: &LoggedCall) -> bool {
    matches!(&call.content, Content::Call { outgoing: false, status, .. } if status.missed())
}

/// One call: who, which way and how it went, when, and a phone button to
/// call back. A click opens the chat at the call's row; the right button
/// offers the row's menu.
fn row(app: &mut App, ui: &mut egui::Ui, call: &LoggedCall) {
    let palette = app.palette;
    let Some(chat) = app.chat(&call.chat).cloned() else {
        return;
    };
    let Content::Call { outgoing, .. } = &call.content else {
        return;
    };
    let title = app.chat_title(&chat);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
    let back = can_call_back(&call.content, &chat);
    let button = Rect::from_center_size(
        pos2(rect.right() - 30.0, rect.center().y),
        Vec2::splat(30.0),
    );
    let call_back = back.then(|| {
        ui.interact(
            button,
            egui::Id::new(("call-log-back", &call.chat, &call.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr("Call back"))
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &title,
            &chat.id,
            picture.as_deref(),
        );
        let left = rect.left() + 76.0;
        let right = if back {
            button.left() - 8.0
        } else {
            rect.right() - 14.0
        };
        let stamp = crate::util::chat_stamp(app.locale, call.timestamp);
        let stamp_line = widgets::line(ui, &stamp, theme::regular(12.0), palette.dim, 120.0, 1);
        let stamp_width = stamp_line.size().x;
        stamp_line.paint(
            ui,
            pos2(right - stamp_width, rect.top() + 16.0),
            palette.dim,
        );
        let name_color = if missed(call) {
            palette.danger
        } else {
            palette.text
        };
        widgets::line(
            ui,
            &title,
            theme::medium(14.5),
            name_color,
            right - stamp_width - 8.0 - left,
            1,
        )
        .paint(ui, pos2(left, rect.top() + 14.0), name_color);
        // Which way, the kind of call, then how it went.
        let detail_color = if missed(call) {
            palette.danger
        } else {
            palette.dim
        };
        let arrow = Rect::from_min_size(pos2(left, rect.top() + 39.0), Vec2::splat(14.0));
        theme::paint_icon(
            ui,
            if *outgoing {
                Icon::ArrowUpRight
            } else {
                Icon::ArrowDownLeft
            },
            arrow,
            14.0,
            detail_color,
        );
        let kind = Rect::from_min_size(pos2(arrow.right() + 4.0, arrow.top()), Vec2::splat(14.0));
        theme::paint_icon(ui, icon(&call.content), kind, 14.0, detail_color);
        // The icons say which way and what kind; the line says how it went,
        // with the length under the time.
        let Content::Call {
            status,
            seconds,
            group,
            ..
        } = &call.content
        else {
            return;
        };
        let mut detail = status.short_label(app.locale, *outgoing);
        if let Some(people) = group.filter(|people| *people > 0) {
            detail.push_str(" · ");
            detail.push_str(
                &crate::i18n::ngettext(app.locale, "{} participant", "{} participants", people)
                    .replace("{}", &people.to_string()),
            );
        }
        let length = seconds.map(crate::util::duration);
        let mut detail_right = right;
        if let Some(length) = length {
            let length_line =
                widgets::line(ui, &length, theme::regular(12.0), palette.dim, 120.0, 1);
            let width = length_line.size().x;
            length_line.paint(ui, pos2(right - width, rect.top() + 39.0), palette.dim);
            detail_right = right - width - 8.0;
        }
        widgets::line(
            ui,
            &detail,
            theme::regular(13.0),
            detail_color,
            detail_right - kind.right() - 6.0,
            1,
        )
        .paint(
            ui,
            pos2(kind.right() + 6.0, rect.top() + 38.0),
            detail_color,
        );
        if let Some(call_back) = &call_back {
            let fill = if call_back.hovered() {
                palette.surface_hover
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().circle_filled(button.center(), 15.0, fill);
            theme::paint_icon(ui, Icon::Phone, button, 18.0, palette.accent);
        }
    }
    if call_back.is_some_and(|call_back| call_back.clicked()) {
        app.actions.push(Action::StartCall(chat.id.clone()));
    } else if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: chat.id.clone(),
            message: call.id.clone(),
        });
    }
    let message = Message {
        id: call.id.clone(),
        chat: call.chat.clone(),
        sender: String::new(),
        sender_name: None,
        from_me: *outgoing,
        timestamp: call.timestamp,
        history_order: None,
        content: call.content.clone(),
        status: crate::model::Delivery::None,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: false,
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    };
    response.context_menu(|ui| {
        row_menu(ui, &palette, &chat, &message, &mut app.actions);
    });
}
