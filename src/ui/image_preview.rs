//! A downloaded image shown over the window, nearly as tall as it.
//!
//! As with an expanded video, this is not another window: the picture grows
//! over a dimmed chat. Hovering it shows three buttons along its top edge,
//! as the expanded video shows its controls: 100% (or back to fitting), copy,
//! and open in another app. A click beside the picture, or Esc, closes it.

use crate::i18n::tr;
use egui::{Color32, Layout, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};

/// Space kept free around the image, as around an expanded video.
const MARGIN: f32 = 16.0;

/// Diameter of the round buttons over the picture, and the space between
/// their centres.
const BUTTON: f32 = 28.0;
const BUTTON_STEP: f32 = 36.0;

/// What the picture reported back from inside the scroll area.
struct Shown {
    /// Where the picture was drawn, in screen coordinates.
    rect: Rect,
    /// A click landed beside the picture.
    beside: bool,
    /// Where a double-click on the picture landed.
    double_click: Option<egui::Pos2>,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(preview) = app.image_preview.clone() else {
        return;
    };
    let palette = app.palette;
    let screen = ctx.content_rect().shrink(MARGIN);
    let response = egui::Modal::new(egui::Id::new("image-preview"))
        .frame(egui::Frame::NONE)
        .backdrop_color(Color32::from_black_alpha(200))
        .show(ctx, |ui| {
            let (area, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            let canvas = area.size().max(Vec2::ZERO);
            // Registered with the image cache like every other draw site, so a
            // sweep never releases the picture while it is on screen.
            let image = crate::ui::widgets::file_image(ui, preview.path());
            match image.load_for_size(ctx, canvas) {
                Ok(egui::load::TexturePoll::Ready { texture }) => {
                    let size = display_size(texture.size, canvas, preview.is_fit(), preview.zoom());
                    if preview.is_fit()
                        && texture.size.x > 0.0
                        && let Some(state) = &mut app.image_preview
                    {
                        state.set_fit_scale(size.x / texture.size.x);
                    }
                    let trackpad = app.scroll_from_trackpad();
                    // Read before the scroll area, which would otherwise take the
                    // wheel. The zoom itself is applied by `App` after the frame.
                    let zoom = zoom_input(ui, area, trackpad).and_then(|(factor, pointer)| {
                        let mut next = app.image_preview.clone()?;
                        next.zoom_by(factor);
                        let zoomed = display_size(texture.size, canvas, next.is_fit(), next.zoom());
                        Some((factor, pointer, zoomed))
                    });
                    let output = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                            egui::ScrollArea::both()
                                .id_salt("image-preview-scroll")
                                .auto_shrink([false, false])
                                // egui drags only on touch screens by default.
                                .scroll_source(egui::scroll_area::ScrollSource {
                                    drag: egui::scroll_area::DragScroll::Always,
                                    ..Default::default()
                                })
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .on_drag_cursor(egui::CursorIcon::Grabbing)
                                .show(ui, |ui| picture(app, ui, &image, canvas.max(size), size))
                        })
                        .inner;
                    let shown = &output.inner;
                    // Stored under the id the scroll area reads, from inside the
                    // scope, and after it, as it clamps its offset to
                    // this frame's size; the next frame lays out the zoomed size
                    // with the pointed-at pixel still under the pointer.
                    if let Some((factor, pointer, zoomed)) = zoom {
                        let mut scroll = output.state;
                        scroll.offset = crate::image_preview::anchored_offset(
                            canvas,
                            size,
                            zoomed,
                            output.state.offset,
                            pointer,
                            pointer,
                        );
                        scroll.store(ctx, output.id);
                        app.actions.push(Action::ZoomImageBy(factor));
                    }
                    // 100% opens with the chosen point in the middle: the
                    // double-clicked one, or the middle of the picture for the
                    // button. The offset is stored for the next frame, which
                    // lays out the new size.
                    let actual_size = |anchor: Vec2, actions: &mut Vec<Action>| {
                        let mut scroll = output.state;
                        scroll.offset = crate::image_preview::anchored_offset(
                            canvas,
                            size,
                            texture.size,
                            output.state.offset,
                            anchor,
                            canvas / 2.0,
                        );
                        scroll.store(ctx, output.id);
                        actions.push(Action::ImageActualSize);
                    };
                    if let Some(pos) = shown.double_click {
                        if preview.is_fit() {
                            actual_size(pos - area.min, &mut app.actions);
                        } else {
                            app.actions.push(Action::FitImage);
                        }
                    }
                    if shown.beside {
                        app.actions.push(Action::CloseImagePreview);
                    }
                    let visible = shown.rect.intersect(area);
                    if visible.is_positive() {
                        let anchor = visible.center() - area.min;
                        let buttons = Buttons {
                            fit: preview.is_fit(),
                            path: preview.path(),
                            locale: app.locale,
                        };
                        match buttons.show(ui, visible, ui.rect_contains_pointer(visible)) {
                            Some(Action::ImageActualSize) => {
                                actual_size(anchor, &mut app.actions);
                            }
                            Some(action) => app.actions.push(action),
                            None => {}
                        }
                    }
                }
                Ok(egui::load::TexturePoll::Pending { .. }) => {
                    let disc = Rect::from_center_size(area.center(), Vec2::splat(48.0));
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE);
                }
                Err(_) => {
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(area)
                            .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
                        |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space((area.height() / 2.0 - 30.0).max(0.0));
                                crate::ui::widgets::rich_text(
                                    ui,
                                    tr("This image could not be displayed in ZapFast."),
                                    theme::regular(14.0),
                                    Color32::WHITE,
                                );
                                ui.add_space(8.0);
                                if theme::soft_button(
                                    ui,
                                    &palette,
                                    None,
                                    tr("Open externally"),
                                    false,
                                )
                                .clicked()
                                {
                                    app.actions
                                        .push(Action::OpenFile(preview.path().to_owned()));
                                }
                            });
                        },
                    );
                }
            }
        });
    if response.should_close() {
        app.actions.push(Action::CloseImagePreview);
    }
}

/// Lays out the picture at `size` in the middle of `content`, with its context
/// menu, and says where it went and what the pointer did.
fn picture(
    app: &mut App,
    ui: &mut egui::Ui,
    image: &egui::Image<'static>,
    content: Vec2,
    size: Vec2,
) -> Shown {
    let palette = app.palette;
    let path = app
        .image_preview
        .as_ref()
        .map(|preview| preview.path().to_owned())
        .unwrap_or_default();
    // The whole content, under the picture, so a click on the picture itself
    // is the picture's. Neither takes keyboard focus: Tab goes straight to
    // the buttons.
    let area = Rect::from_min_size(ui.cursor().min, content);
    let beside = ui.allocate_rect(area, Sense::CLICK);
    // Placed by hand: in a justified layout the picture's response would
    // stretch over the whole content and take the clicks beside it.
    let rect = Rect::from_center_size(area.center(), size);
    image.paint_at(ui, rect);
    let image_response = ui.interact(rect, ui.id().with("image-preview-picture"), Sense::CLICK);
    let copy_label = crate::i18n::gettext(app.locale, "Copy image");
    let save_label = crate::i18n::gettext(app.locale, "Save as…");
    let open_label = crate::i18n::gettext(app.locale, "Open in another app");
    let menu_width =
        crate::ui::widgets::menu_width(ui, &[&copy_label, &save_label, &open_label], true)
            .max(180.0);
    egui::Popup::context_menu(&image_response)
        .width(menu_width)
        .frame(crate::ui::widgets::menu_frame(&palette))
        .show(|ui| {
            if crate::ui::widgets::menu_item(ui, &palette, Some(Icon::Copy), &copy_label) {
                app.actions.push(Action::CopyImage(path.clone()));
            }
            if crate::ui::widgets::menu_item(ui, &palette, Some(Icon::Download), &save_label) {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image.png")
                    .to_owned();
                app.actions.push(Action::SaveAttachmentAs {
                    path: path.clone(),
                    name,
                });
            }
            if crate::ui::widgets::menu_item(ui, &palette, Some(Icon::ExternalLink), &open_label) {
                app.actions.push(Action::OpenFile(path.clone()));
            }
        });
    let double_click = image_response
        .interact_pointer_pos()
        .filter(|_| image_response.double_clicked());
    Shown {
        rect,
        beside: beside.clicked(),
        double_click,
    }
}

/// The round buttons along the top of the picture, as over an expanded video.
struct Buttons<'a> {
    fit: bool,
    path: &'a std::path::Path,
    locale: crate::i18n::Locale,
}

impl Buttons<'_> {
    /// Places the buttons at the top right of `picture` and returns the action
    /// of the one clicked. They are drawn while the pointer is over the
    /// picture, or while one of them has keyboard focus.
    fn show(&self, ui: &mut egui::Ui, picture: Rect, hovered: bool) -> Option<Action> {
        let copy_hint = format!(
            "{} ({})",
            crate::i18n::gettext(self.locale, "Copy image"),
            super::keys::label("Ctrl+C"),
        );
        let (zoom_icon, zoom_hint, zoom_action) = if self.fit {
            (Icon::ZoomIn, tr("Zoom to 100%"), Action::ImageActualSize)
        } else {
            (Icon::ZoomOut, tr("Fit to the window (0)"), Action::FitImage)
        };
        // Left to right: zoom, copy, open.
        let buttons = [
            ("image-zoom", zoom_icon, zoom_hint.to_owned(), zoom_action),
            (
                "image-copy",
                Icon::Copy,
                copy_hint,
                Action::CopyImage(self.path.to_owned()),
            ),
            (
                "image-open",
                Icon::ExternalLink,
                crate::i18n::gettext(self.locale, "Open in another app").into_owned(),
                Action::OpenFile(self.path.to_owned()),
            ),
        ];
        let count = buttons.len() as f32;
        let placed: Vec<_> = buttons
            .into_iter()
            .enumerate()
            .map(|(index, (id, icon, hint, action))| {
                let x = picture.right() - 20.0 - (count - 1.0 - index as f32) * BUTTON_STEP;
                let rect =
                    Rect::from_center_size(pos2(x, picture.top() + 20.0), Vec2::splat(BUTTON));
                // Global ids: there is one preview, and the tour finds them.
                let response = ui
                    .interact(rect, egui::Id::new(id), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &hint)
                });
                (icon, hint, action, response)
            })
            .collect();
        if !hovered && !placed.iter().any(|(.., response)| response.has_focus()) {
            return None;
        }
        let mut clicked = None;
        for (icon, hint, action, response) in placed {
            let fill = if response.has_focus() || response.hovered() {
                Color32::from_black_alpha(210)
            } else {
                Color32::from_black_alpha(150)
            };
            ui.painter()
                .circle_filled(response.rect.center(), BUTTON / 2.0, fill);
            if response.has_focus() {
                ui.painter().circle_stroke(
                    response.rect.center(),
                    BUTTON / 2.0,
                    egui::Stroke::new(1.5, Color32::WHITE),
                );
            }
            theme::paint_icon(ui, icon, response.rect, 15.0, Color32::WHITE);
            if response.on_hover_text(hint).clicked() {
                clicked = Some(action);
            }
        }
        clicked
    }
}

/// Zoom factor the wheel or a pinch asks for over the preview area, with the
/// anchor point relative to the picture area. Each wheel notch is one header
/// zoom step. A plain mouse wheel zooms instead of scrolling, so its delta is
/// taken from the scroll area. Windows and X11 report touchpads as wheel
/// lines, so two fingers zoom there; only scrolling reported in points
/// (macOS, Wayland) keeps moving the picture.
fn zoom_input(ui: &mut egui::Ui, area: Rect, trackpad: bool) -> Option<(f32, Vec2)> {
    let (touch, hover) = ui.input(|input| {
        (
            input.multi_touch().map(|touch| touch.center_pos),
            input.pointer.hover_pos(),
        )
    });
    let anchor = touch.or(hover)?;
    if !area.contains(anchor) || touch.is_none() && !ui.rect_contains_pointer(area) {
        return None;
    }
    let (zoom_speed, line_speed) = ui.ctx().options(|options| {
        let input = &options.input_options;
        (input.scroll_zoom_speed, input.line_scroll_speed)
    });
    let step = crate::image_preview::PreviewState::ZOOM_STEP;
    let factor = ui.input_mut(|input| {
        let pinch = input.multi_touch().is_some()
            || input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Zoom(_)));
        let mut factor = input.zoom_delta();
        if !pinch {
            // Without a pinch the zoom came from Ctrl/Cmd+wheel (Windows
            // touchpad pinches included), to which egui applies its own curve,
            // exp(scroll_zoom_speed * points). It keeps the modifiers from the
            // start of the gesture, so this checks the source, not the keys.
            factor = factor.powf(step.ln() / (zoom_speed * line_speed));
        }
        // Shift and Alt turn the wheel sideways; Ctrl pressed during a plain
        // notch must not hand the rest of it to the scroll area.
        if !trackpad
            && !input.modifiers.shift
            && !input.modifiers.alt
            && input.smooth_scroll_delta.y != 0.0
        {
            factor *= step.powf(input.smooth_scroll_delta.y / line_speed);
            input.smooth_scroll_delta.y = 0.0;
        }
        factor
    });
    (factor != 1.0).then_some((factor, anchor - area.min))
}

/// Size the image is drawn at from the texture's intrinsic pixel dimensions:
/// fitted into the canvas, or scaled by the preview's zoom factor. Zoom is
/// applied here only. The size hint passed when loading does not change the
/// texture: egui decodes raster formats (all the preview accepts) once at full
/// resolution and reports the source size, whatever size is asked for.
fn display_size(original: Vec2, canvas: Vec2, fit: bool, zoom: f32) -> Vec2 {
    let (width, height) = if fit {
        crate::image_preview::fit_size(original.x, original.y, canvas.x, canvas.y)
    } else {
        crate::image_preview::zoomed_size(original.x, original.y, zoom)
    };
    vec2(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_images_keep_aspect_ratio_inside_the_canvas() {
        assert_eq!(
            display_size(vec2(1600.0, 1200.0), vec2(800.0, 700.0), true, 1.0),
            vec2(800.0, 600.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), true, 1.0),
            vec2(320.0, 240.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), false, 2.0),
            vec2(640.0, 480.0)
        );
    }
}
