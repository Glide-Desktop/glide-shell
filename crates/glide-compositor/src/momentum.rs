use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use glide_theme::motion;
use smithay::{
    backend::input::{AxisSource, InputBackend, InputEvent, InputTime},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
};

use crate::{GlideState, input::Scroll};

const WINDOW: u64 = 100_000;
const PAUSE: u64 = 50_000;
const GLIDE: f64 = 0.4;
const LONGEST: Duration = Duration::from_millis(1200);
const SLOWEST: f64 = 150.0;
const FASTEST: f64 = 8000.0;

#[derive(Debug, Default)]
pub struct Tracker {
    samples: VecDeque<(u64, [f64; 2])>,
}

impl Tracker {
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn push(&mut self, micros: u64, delta: [f64; 2]) {
        while self.samples.front().is_some_and(|&(at, _)| micros.saturating_sub(at) > WINDOW) {
            self.samples.pop_front();
        }
        self.samples.push_back((micros, delta));
    }

    #[must_use]
    pub fn velocity(&self, lifted: u64) -> Option<[f64; 2]> {
        let &(last, _) = self.samples.back()?;
        let &(first, _) = self.samples.front()?;
        if lifted.saturating_sub(last) > PAUSE || last <= first {
            return None;
        }
        let seconds = (last - first) as f64 / 1_000_000.0;
        let moved = self.samples.iter().skip(1).fold([0.0, 0.0], |sum, (_, d)| [sum[0] + d[0], sum[1] + d[1]]);
        let velocity = moved.map(|axis| axis / seconds);
        let speed = velocity[0].hypot(velocity[1]);
        if !speed.is_finite() || speed < SLOWEST {
            return None;
        }
        let capped = (FASTEST / speed).min(1.0);
        Some(velocity.map(|axis| axis * capped))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Coast {
    started: Instant,
    release: motion::Release,
    distance: [f64; 2],
    sent: [f64; 2],
}

impl Coast {
    #[must_use]
    pub fn new(velocity: [f64; 2], now: Instant) -> Option<Self> {
        let release = motion::Release::new(1.0, 1.0 / GLIDE, motion::scaled(LONGEST));
        if release.duration.is_zero() || release.slope <= 0.0 {
            return None;
        }
        let travel = release.duration.as_secs_f64() / release.slope;
        Some(Self { started: now, release, distance: velocity.map(|axis| axis * travel), sent: [0.0, 0.0] })
    }

    pub fn step(&mut self, now: Instant) -> ([f64; 2], bool) {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f64();
        let t = (elapsed / self.release.duration.as_secs_f64()).min(1.0);
        let progress = self.release.at(t);
        let at = self.distance.map(|axis| axis * progress);
        let delta = [at[0] - self.sent[0], at[1] - self.sent[1]];
        self.sent = at;
        (delta, t >= 1.0)
    }

    #[must_use]
    pub fn moving(&self) -> [bool; 2] {
        self.distance.map(|axis| axis != 0.0)
    }
}

#[derive(Debug)]
struct Run {
    coast: Coast,
    target: Option<WlSurface>,
    factor: f64,
}

#[derive(Debug, Default)]
pub struct Momentum {
    tracker: Tracker,
    run: Option<Run>,
    coasts: bool,
}

impl GlideState {
    pub(crate) fn momentum_takes(&mut self, scroll: &Scroll) -> bool {
        if scroll.source != AxisSource::Finger {
            self.end_coast(true);
            self.session.momentum.tracker.clear();
            return false;
        }
        if !scroll.stops_any() {
            self.end_coast(false);
            self.session.momentum.tracker.push(scroll.time.micros(), scroll.amounts());
            return false;
        }
        let momentum = &mut self.session.momentum;
        let velocity = momentum.tracker.velocity(scroll.time.micros());
        momentum.tracker.clear();
        if !self.input_config.touchpad_momentum || !momentum.coasts {
            return false;
        }
        let Some(coast) = velocity.and_then(|velocity| Coast::new(velocity, Instant::now())) else {
            return false;
        };
        let target = self.pointer_surface();
        self.session.momentum.run = Some(Run { coast, target, factor: scroll.factor });
        self.queue_animation_redraw();
        true
    }

    pub(crate) fn note_scroll_coasts(&mut self, scroll: &Scroll, coasts: bool) {
        if scroll.source == AxisSource::Finger && !scroll.stops_any() {
            self.session.momentum.coasts = coasts;
        }
    }

    pub fn momentum_pending(&mut self) -> bool {
        let Some(run) = &self.session.momentum.run else {
            return false;
        };
        if self.screen_locked() || self.pointer_surface() != run.target {
            self.session.momentum.run = None;
            return false;
        }
        true
    }

    pub fn tick_momentum_on(&mut self, output: &smithay::output::Output) {
        if !self.momentum_pending() {
            return;
        }
        let pointer = self.seat.get_pointer().map(|pointer| pointer.current_location()).unwrap_or_default();
        if !self.space.output_geometry(output).is_some_and(|area| area.to_f64().contains(pointer)) {
            return;
        }
        let Some(run) = self.session.momentum.run.as_mut() else {
            return;
        };
        let (delta, done) = run.coast.step(glide_ui::clock::now());
        let factor = run.factor;
        let moving = run.coast.moving();
        if done {
            self.session.momentum.run = None;
        }
        let raw = delta.map(|axis| (axis != 0.0).then_some(axis));
        if raw.iter().any(Option::is_some) {
            self.route_scroll(&Scroll::finger(InputTime::now(), raw, factor));
        }
        if done {
            self.route_scroll(&Scroll::finger(InputTime::now(), moving.map(|on| on.then_some(0.0)), factor));
        }
    }

    pub fn end_coast(&mut self, stop: bool) {
        let Some(run) = self.session.momentum.run.take() else {
            return;
        };
        if stop && !self.screen_locked() && self.pointer_surface() == run.target {
            let (moving, factor) = (run.coast.moving(), run.factor);
            self.route_scroll(&Scroll::finger(InputTime::now(), moving.map(|on| on.then_some(0.0)), factor));
        }
    }

    pub fn end_coast_on<I: InputBackend>(&mut self, event: &InputEvent<I>) {
        if matches!(
            event,
            InputEvent::Keyboard { .. }
                | InputEvent::PointerButton { .. }
                | InputEvent::TouchDown { .. }
                | InputEvent::GestureHoldBegin { .. }
                | InputEvent::GestureSwipeBegin { .. }
                | InputEvent::GesturePinchBegin { .. }
        ) {
            self.end_coast(true);
        }
    }

    pub fn end_coast_before(&mut self, next: Option<&WlSurface>) {
        if self.session.momentum.run.as_ref().is_some_and(|run| run.target.as_ref() != next) {
            self.session.momentum.run = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u64 = 8_000;

    fn swipe(tracker: &mut Tracker, step: f64, frames: u64) -> u64 {
        for frame in 0..frames {
            tracker.push(frame * FRAME, [0.0, step]);
        }
        (frames - 1) * FRAME
    }

    #[test]
    fn a_steady_swipe_measures_its_own_speed() {
        let mut tracker = Tracker::default();
        let last = swipe(&mut tracker, 12.0, 20);
        let velocity = tracker.velocity(last + FRAME).expect("fast enough to coast");
        assert!((velocity[1] - 1500.0).abs() < 1.0, "{velocity:?}");
        assert!(velocity[0].abs() < f64::EPSILON);
    }

    #[test]
    fn fingers_that_rested_before_lifting_do_not_coast() {
        let mut tracker = Tracker::default();
        let last = swipe(&mut tracker, 12.0, 20);
        assert!(tracker.velocity(last + PAUSE + 1).is_none());
    }

    #[test]
    fn a_slow_drag_and_a_single_sample_do_not_coast() {
        let mut tracker = Tracker::default();
        let last = swipe(&mut tracker, 0.5, 20);
        assert!(tracker.velocity(last).is_none());
        let mut single = Tracker::default();
        single.push(0, [0.0, 40.0]);
        assert!(single.velocity(0).is_none());
    }

    #[test]
    fn only_the_last_tenth_of_a_second_counts() {
        let mut tracker = Tracker::default();
        let last = swipe(&mut tracker, 30.0, 40);
        for frame in 1..=20 {
            tracker.push(last + frame * FRAME, [0.0, 4.0]);
        }
        let velocity = tracker.velocity(last + 20 * FRAME).expect("still moving");
        assert!((velocity[1] - 500.0).abs() < 1.0, "{velocity:?}");
    }

    #[test]
    fn a_flick_is_capped() {
        let mut tracker = Tracker::default();
        let last = swipe(&mut tracker, 400.0, 10);
        let velocity = tracker.velocity(last).expect("a flick");
        assert!((velocity[1] - FASTEST).abs() < 1e-6, "{velocity:?}");
    }

    #[test]
    fn a_coast_leaves_at_the_finger_speed_and_comes_to_rest() {
        let start = Instant::now();
        let mut coast = Coast::new([0.0, 1500.0], start).expect("animations are on in tests");
        let frame = Duration::from_millis(8);
        let (first, _) = coast.step(start + frame);
        let speed = first[1] / frame.as_secs_f64();
        assert!((speed - 1500.0).abs() < 60.0, "left at {speed} px/s");
        let mut total = first[1];
        let mut previous = first[1];
        let mut now = start + frame;
        loop {
            now += frame;
            let (delta, done) = coast.step(now);
            assert!(delta[1] <= previous + 1e-9, "the coast never speeds up");
            assert!(delta[0].abs() < f64::EPSILON);
            previous = delta[1];
            total += delta[1];
            if done {
                break;
            }
        }
        assert!(previous < 0.5, "it lands at rest, last step {previous}");
        assert!((total - coast.distance[1]).abs() < 1e-6);
        assert_eq!(coast.moving(), [false, true]);
    }
}
