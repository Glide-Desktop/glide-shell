use std::collections::HashMap;
use std::time::Instant;

use glide_paint::TextAlign;
use glide_shell::raster::{Canvas, ShellRenderer};
use glide_theme::motion::{self, ease_out as ease};
use glide_theme::{Color, TextStyle};
use glide_ui::Reveal;
use smithay::{
    backend::renderer::element::{
        Kind,
        memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
    },
    utils::{Logical, Point, Scale, Size},
};

use super::{Slot, Topbar};
use crate::i18n::{Key, t};

const HEIGHT: i32 = 24;
const PAD_X: i32 = glide_theme::spacing::SP2;
const GAP: i32 = glide_theme::spacing::SP2;
const KEY_GAP: i32 = 6;
const RADIUS: f32 = glide_theme::shape::R2;
const LABEL: TextStyle = TextStyle { size_px: 12.0, line_height_px: 16.0, weight: 450, ..TextStyle::CAP };
const KEYCAP: TextStyle = TextStyle { size_px: 10.5, line_height_px: 16.0, weight: 450, ..TextStyle::NUM };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    Idle,
    Since(Instant),
}

#[derive(Debug)]
pub(super) struct QsTip {
    hover: Option<Slot>,
    wait: Wait,
    shown: Option<Slot>,
    reveal: Reveal,
    cache: HashMap<(Slot, i32), (u64, MemoryRenderBuffer)>,
}

impl Default for QsTip {
    fn default() -> Self {
        Self {
            hover: None,
            wait: Wait::Idle,
            shown: None,
            reveal: Reveal::closed().with_ease(ease),
            cache: HashMap::new(),
        }
    }
}

pub(super) fn text(slot: Slot) -> Option<(Key, Option<&'static str>)> {
    match slot {
        Slot::QsScreenshot => Some((Key::QsTipScreenshot, Some("Print"))),
        Slot::QsSettings => Some((Key::QsTipSettings, None)),
        Slot::QsLock => Some((Key::QsTipLock, Some("Super+L"))),
        Slot::QsPowerMenu => Some((Key::QsTipPower, None)),
        _ => None,
    }
}

impl QsTip {
    pub(super) fn hover(&mut self, slot: Option<Slot>) {
        let slot = slot.filter(|slot| text(*slot).is_some());
        if slot != self.hover {
            self.hover = slot;
            self.wait = slot.map_or(Wait::Idle, |_| Wait::Since(glide_ui::clock::now()));
        }
    }

    pub(super) fn settle(&mut self) {
        let duration = motion::t1();
        let due =
            matches!(self.wait, Wait::Since(at) if glide_ui::clock::since(at) >= glide_theme::component::TOOLTIP_DELAY);
        match self.hover.filter(|_| due) {
            Some(slot) => {
                self.shown = Some(slot);
                self.reveal.open(duration);
            }
            None => {
                self.reveal.close(duration);
            }
        }
        self.reveal.settle(duration);
        if !self.reveal.is_open() && !self.reveal.is_visible(duration) {
            self.shown = None;
        }
    }

    pub(super) fn is_animating(&self) -> bool {
        self.reveal.is_animating(motion::t1())
    }

    pub(super) fn next_wake(&self) -> Option<Instant> {
        match (self.hover, self.wait) {
            (Some(slot), Wait::Since(at)) if self.shown != Some(slot) => {
                Some(at + glide_theme::component::TOOLTIP_DELAY)
            }
            _ => None,
        }
    }

    pub(super) fn clear(&mut self) {
        self.cache.clear();
    }
}

fn width(painter: &crate::paint::Painter, label: &str, keycap: Option<&str>) -> i32 {
    let label_w = painter.text_width_styled(label, &LABEL).ceil() as i32;
    let key_w = keycap.map_or(0, |keycap| KEY_GAP + painter.text_width_styled(keycap, &KEYCAP).ceil() as i32);
    PAD_X * 2 + label_w + key_w
}

fn draw(
    painter: &crate::paint::Painter, (label, keycap): (&str, Option<&str>),
    (fill, line, ink, faint): (Color, Color, Color, Color), scale: i32,
) -> Option<MemoryRenderBuffer> {
    let w = width(painter, label, keycap);
    let mut c = Canvas::popover((w, HEIGHT), scale, RADIUS, fill, line)?;
    let label_w = painter.text_width_styled(label, &LABEL).ceil() as i32;
    c.text(painter, (PAD_X, PAD_X + label_w, 0, HEIGHT), TextAlign::Left, label, ink, &LABEL);
    if let Some(keycap) = keycap {
        let left = PAD_X + label_w + KEY_GAP;
        c.text(painter, (left, w - PAD_X, 0, HEIGHT), TextAlign::Left, keycap, faint, &KEYCAP);
    }
    Some(c.finish())
}

pub(super) fn push<R: ShellRenderer>(
    topbar: &mut Topbar, renderer: &mut R, painter: &crate::paint::Painter, (scale, output_scale): (Scale<f64>, i32),
    elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) {
    let Some(slot) = topbar.qs_tip.shown else {
        return;
    };
    let (Some((key, keycap)), Some(anchor)) = (text(slot), topbar.slot_rect(slot)) else {
        return;
    };
    let progress = topbar.qs_tip.reveal.value(motion::t1());
    let alpha = motion::assist(progress) as f32;
    if alpha <= 0.0 {
        return;
    }
    let label = t(key);
    let stamp =
        glide_theme::Revision::new().u64(topbar.theme.revision()).u64(crate::paint::painter_generation()).finish();
    let theme = &topbar.theme;
    let colours = (theme.popover_bg, theme.line, theme.label, theme.faint);
    let fresh = topbar.qs_tip.cache.get(&(slot, output_scale)).is_some_and(|(at, _)| *at == stamp);
    if !fresh && let Some(buffer) = draw(painter, (label, keycap), colours, output_scale) {
        topbar.qs_tip.cache.insert((slot, output_scale), (stamp, buffer));
    }
    let Some((_, buffer)) = topbar.qs_tip.cache.get(&(slot, output_scale)) else {
        return;
    };
    let w = width(painter, label, keycap);
    let popup = topbar.popup_rect.unwrap_or(anchor);
    let x = (anchor.loc.x + (anchor.size.w - w) / 2).min(popup.loc.x + popup.size.w - w);
    let rise = ((1.0 - progress) * f64::from(glide_theme::component::POPOVER_RISE)).round() as i32;
    let at: Point<i32, Logical> = Point::from((x, anchor.loc.y + anchor.size.h + GAP - rise));
    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        at.to_f64().to_physical(scale),
        buffer,
        Some(alpha.min(1.0)),
        None,
        Some(Size::from((w, HEIGHT))),
        Kind::Unspecified,
    ) {
        elements.push(element);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_footer_button_explains_itself_after_the_tooltip_delay() {
        let mut tip = QsTip::default();
        tip.hover(Some(Slot::QsVolumeTrack));
        assert_eq!(tip.hover, None, "only the footer buttons have tips");
        tip.hover(Some(Slot::QsLock));
        tip.settle();
        assert_eq!(tip.shown, None, "not before the delay");
        assert!(!tip.is_animating(), "the dwell pumps no frames");
        let Wait::Since(started) = tip.wait else { panic!("the dwell is timed") };
        assert_eq!(tip.next_wake(), Some(started + glide_theme::component::TOOLTIP_DELAY), "a timer wakes it");
        let earlier = glide_ui::clock::now().checked_sub(glide_theme::component::TOOLTIP_DELAY).expect("uptime");
        tip.wait = Wait::Since(earlier);
        tip.settle();
        assert_eq!(tip.shown, Some(Slot::QsLock));
        assert_eq!(tip.next_wake(), None, "a shown tip needs no wake");
        assert_eq!(text(Slot::QsLock), Some((Key::QsTipLock, Some("Super+L"))));
        assert_eq!(text(Slot::QsScreenshot).and_then(|(_, keycap)| keycap), Some("Print"));
        let painter = crate::paint::Painter::new(14.0, None);
        let theme = crate::TopbarTheme::default();
        let colours = (theme.popover_bg, theme.line, theme.label, theme.faint);
        assert!(draw(&painter, (t(Key::QsTipLock), Some("Super+L")), colours, 2).is_some());
    }
}
