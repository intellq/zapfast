//! A loaded video shown over the window, nearly as tall as it.
//!
//! Not the operating system's full screen: the window stays as it is, and
//! the video fills most of it over a dimmed chat. The controls are the ones
//! the message shows, and the same corner that expanded it brings it back.

use egui::{Color32, Rect, Sense, Vec2, pos2};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};
use crate::video::State;

/// Space kept free around the video.
const MARGIN: f32 = 16.0;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(message) = app.video_expanded.clone() else {
        return;
    };
    // The video stopped, ended, or another one started.
    let (Some(status), Some(path)) = (
        app.video.status(&message),
        app.video.loaded().map(|(_, path)| path.to_owned()),
    ) else {
        app.actions.push(Action::CollapseVideo);
        return;
    };
    app.video.saw(&message);
    let palette = app.palette;
    let screen = ctx.content_rect();
    let area = screen.shrink(MARGIN);
    let response = egui::Modal::new(egui::Id::new("video-view"))
        .frame(egui::Frame::NONE)
        .backdrop_color(Color32::from_black_alpha(200))
        .show(ctx, |ui| {
            // Around the picture, a click returns the video to its message.
            let (rect, around) = ui.allocate_exact_size(area.size(), Sense::click());
            let picture = match status.frame.as_ref() {
                Some(frame) => {
                    let picture = super::conversation::fit_within(frame.size_vec2(), rect);
                    ui.painter().rect_filled(picture, 6.0, Color32::BLACK);
                    super::conversation::paint_texture(
                        ui,
                        picture,
                        frame.id(),
                        Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)),
                        6.0,
                    );
                    picture
                }
                None => rect,
            };
            if status.state == State::Loading && status.frame.is_none() {
                let disc = Rect::from_center_size(picture.center(), Vec2::splat(48.0));
                theme::paint_spinner(ui, disc, 24.0, Color32::WHITE);
            }
            if around.clicked() {
                app.actions.push(Action::CollapseVideo);
            }
            // A click on the picture plays or pauses, as in the message.
            if ui
                .interact(picture, ui.id().with("video-picture"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.actions.push(Action::PlayVideo {
                    message: message.clone(),
                    path: path.clone(),
                });
            }
            if status.state == State::Paused
                || (status.state != State::Loading && ui.rect_contains_pointer(picture))
            {
                let controls = super::conversation::VideoControls {
                    palette: &palette,
                    locale: app.locale,
                    video: &app.video,
                    message: &message,
                    path: &path,
                };
                super::conversation::video_controls(
                    ui,
                    &controls,
                    picture,
                    &status,
                    &mut app.actions,
                );
                let button = Rect::from_center_size(
                    pos2(picture.right() - 20.0, picture.top() + 20.0),
                    Vec2::splat(28.0),
                );
                ui.painter()
                    .circle_filled(button.center(), 14.0, Color32::from_black_alpha(150));
                theme::paint_icon(ui, Icon::Minimize, button, 15.0, Color32::WHITE);
                if ui
                    .interact(button, ui.id().with("video-collapse"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(crate::i18n::gettext(app.locale, "Back to the message").as_ref())
                    .clicked()
                {
                    app.actions.push(Action::CollapseVideo);
                }
            }
        });
    if response.backdrop_response.clicked() {
        app.actions.push(Action::CollapseVideo);
    }
}
