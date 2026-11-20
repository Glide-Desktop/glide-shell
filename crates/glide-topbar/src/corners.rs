use glide_theme::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Split {
    Body,
    Chevron,
}

impl Split {
    pub(super) const fn tag(self) -> u8 {
        match self {
            Self::Body => 1,
            Self::Chevron => 2,
        }
    }

    pub(super) fn radii(self, outer: f32, inner: f32) -> [f32; 4] {
        match self {
            Self::Body => [outer, inner, inner, outer],
            Self::Chevron => [inner, outer, outer, inner],
        }
    }
}

fn distance(x: f32, y: f32, (w, h): (f32, f32), radii: [f32; 4]) -> f32 {
    let (dx, dy) = (x - w / 2.0, y - h / 2.0);
    let radius = match (dx >= 0.0, dy >= 0.0) {
        (false, false) => radii[0],
        (true, false) => radii[1],
        (true, true) => radii[2],
        (false, true) => radii[3],
    }
    .min(w / 2.0)
    .min(h / 2.0);
    let qx = dx.abs() - (w / 2.0 - radius);
    let qy = dy.abs() - (h / 2.0 - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

pub(super) fn coverage(x: f32, y: f32, size: (f32, f32), radii: [f32; 4], scale: f32, ring: Option<f32>) -> f32 {
    let d = distance(x, y, size, radii);
    let inside = (0.5 - d * scale).clamp(0.0, 1.0);
    match ring {
        None => inside,
        Some(width) => (inside - (0.5 - (d + width) * scale).clamp(0.0, 1.0)).max(0.0),
    }
}

pub(super) fn paint(
    buf: &mut [u8], (width, height, scale): (i32, i32, i32), radii: [f32; 4], colour: Color, ring: Option<f32>,
) {
    let (columns, rows) = (width * scale, height * scale);
    let factor = scale as f32;
    let size = (width as f32, height as f32);
    let [red, green, blue, opacity] = colour.0;
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    for row in 0..rows {
        for column in 0..columns {
            let point = ((column as f32 + 0.5) / factor, (row as f32 + 0.5) / factor);
            let cover = coverage(point.0, point.1, size, radii, factor, ring);
            if cover <= 0.0 {
                continue;
            }
            let alpha = opacity * cover;
            let Ok(at) = usize::try_from((row * columns + column) * 4) else {
                continue;
            };
            let Some(pixel) = buf.get_mut(at..at + 4) else {
                continue;
            };
            pixel.copy_from_slice(&[byte(blue * alpha), byte(green * alpha), byte(red * alpha), byte(alpha)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_half_keeps_its_outer_corners_round_and_its_inner_ones_tight() {
        let radii = Split::Body.radii(8.0, 4.0);
        let size = (100.0, 56.0);
        assert!(coverage(1.0, 1.0, size, radii, 1.0, None) < 0.1, "the outer top-left corner is cut by 8 px");
        assert!(coverage(98.5, 1.5, size, radii, 1.0, None) > 0.5, "the inner top-right corner is cut by only 4 px");
        assert!(coverage(50.0, 28.0, size, radii, 1.0, None) >= 1.0);
        assert_eq!(Split::Chevron.radii(8.0, 4.0), [4.0, 8.0, 8.0, 4.0]);
        assert!(coverage(50.0, 28.0, size, radii, 1.0, Some(2.0)) <= 0.0, "a ring leaves the middle empty");
        assert!(coverage(50.0, 1.0, size, radii, 1.0, Some(2.0)) > 0.5, "and covers its edge");
    }
}
