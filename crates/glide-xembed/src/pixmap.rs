#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Icon {
    pub width: i32,
    pub height: i32,
    pub argb: Vec<u8>,
}

impl Icon {
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.argb.chunks_exact(4).all(|pixel| pixel[0] == 0)
    }

    #[must_use]
    pub fn is_opaque_black(&self) -> bool {
        !self.argb.is_empty() && self.argb.chunks_exact(4).all(|pixel| pixel == [u8::MAX, 0, 0, 0])
    }

    #[must_use]
    pub fn is_unpainted_opaque(&self, depth: u8, damaged: bool) -> bool {
        depth != 32 && !damaged && self.is_opaque_black()
    }

    pub fn apply_shape_mask(&mut self, rects: &[(i16, i16, u16, u16)]) {
        let (Ok(w), Ok(h)) = (usize::try_from(self.width), usize::try_from(self.height)) else {
            return;
        };
        if w == 0 || h == 0 || self.argb.len() < w * h * 4 {
            return;
        }
        let mut visible = vec![false; w * h];
        for &(rx, ry, rw, rh) in rects {
            let x0 = usize::try_from(rx.max(0)).unwrap_or(0).min(w);
            let y0 = usize::try_from(ry.max(0)).unwrap_or(0).min(h);
            let x1 = usize::try_from((i32::from(rx) + i32::from(rw)).max(0)).unwrap_or(0).min(w);
            let y1 = usize::try_from((i32::from(ry) + i32::from(rh)).max(0)).unwrap_or(0).min(h);
            for y in y0..y1 {
                visible[y * w + x0..y * w + x1].fill(true);
            }
        }
        for (index, shown) in visible.into_iter().enumerate() {
            if !shown {
                self.argb[index * 4..index * 4 + 4].fill(0);
            }
        }
    }

    #[must_use]
    pub fn trim_to_square(self) -> Self {
        let (Ok(w), Ok(h)) = (usize::try_from(self.width), usize::try_from(self.height)) else {
            return self;
        };
        if w == 0 || h == 0 || self.argb.len() < w * h * 4 {
            return self;
        }
        let (mut min_x, mut min_y) = (w, h);
        let (mut max_x, mut max_y) = (0usize, 0usize);
        let mut any = false;
        for y in 0..h {
            for x in 0..w {
                if self.argb[(y * w + x) * 4] > 0 {
                    any = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if !any {
            return self;
        }
        let content_w = max_x - min_x + 1;
        let content_h = max_y - min_y + 1;
        let side = content_w.max(content_h);
        if side == w && side == h && min_x == 0 && min_y == 0 {
            return self;
        }
        let off_x = (side - content_w) / 2;
        let off_y = (side - content_h) / 2;
        let mut argb = vec![0u8; side * side * 4];
        let row_bytes = content_w * 4;
        for y in 0..content_h {
            let src = ((min_y + y) * w + min_x) * 4;
            let dst = ((off_y + y) * side + off_x) * 4;
            argb[dst..dst + row_bytes].copy_from_slice(&self.argb[src..src + row_bytes]);
        }
        Self {
            width: i32::try_from(side).unwrap_or(self.width),
            height: i32::try_from(side).unwrap_or(self.height),
            argb,
        }
    }
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let value = (u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
    u8::try_from(value.min(255)).unwrap_or(u8::MAX)
}

#[must_use]
pub fn from_zpixmap(data: &[u8], width: u16, height: u16, depth: u8) -> Option<Icon> {
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = usize::from(width) * usize::from(height);
    let source = data.get(..pixels * 4)?;
    let mut argb = Vec::with_capacity(pixels * 4);
    for pixel in source.chunks_exact(4) {
        let (blue, green, red) = (pixel[0], pixel[1], pixel[2]);
        let alpha = if depth == 32 { pixel[3] } else { u8::MAX };
        argb.extend_from_slice(&[
            alpha,
            unpremultiply(red, alpha),
            unpremultiply(green, alpha),
            unpremultiply(blue, alpha),
        ]);
    }
    Some(Icon { width: i32::from(width), height: i32::from(height), argb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_premultiplied_argb_window_becomes_straight_network_order_argb() {
        let data = [0x40, 0x20, 0x10, 0x80, 0, 0, 0, 0];
        let icon = from_zpixmap(&data, 2, 1, 32).expect("enough bytes");
        assert_eq!(icon.argb, [0x80, 0x20, 0x40, 0x80, 0, 0, 0, 0]);
        assert_eq!((icon.width, icon.height), (2, 1));
    }

    #[test]
    fn a_window_without_alpha_is_opaque() {
        let data = [1, 2, 3, 0];
        let icon = from_zpixmap(&data, 1, 1, 24).expect("enough bytes");
        assert_eq!(icon.argb, [0xff, 3, 2, 1]);
        assert!(!icon.is_blank());
    }

    #[test]
    fn a_short_image_is_refused_and_a_clear_one_is_blank() {
        assert_eq!(from_zpixmap(&[0; 7], 2, 1, 32), None);
        assert_eq!(from_zpixmap(&[], 0, 4, 32), None);
        let clear = from_zpixmap(&[0; 16], 2, 2, 32).expect("enough bytes");
        assert!(clear.is_blank(), "nothing drawn yet");
    }

    #[test]
    fn unpremultiplying_never_overflows() {
        assert_eq!(unpremultiply(200, 100), 255);
        assert_eq!(unpremultiply(50, 0), 0);
        assert_eq!(unpremultiply(255, 255), 255);
    }

    #[test]
    fn transparent_borders_are_cropped_and_recentered_into_a_square() {
        let mut argb = vec![0u8; 32 * 32 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let at = (y * 32 + x) * 4;
                argb[at..at + 4].copy_from_slice(&[0xff, 0xaa, 0xbb, 0xcc]);
            }
        }
        let wine = Icon { width: 32, height: 32, argb }.trim_to_square();
        assert_eq!((wine.width, wine.height), (16, 16));
        assert!(wine.argb.chunks_exact(4).all(|pixel| pixel == [0xff, 0xaa, 0xbb, 0xcc]));

        let mut rect = vec![0u8; 20 * 20 * 4];
        for y in 7..11 {
            for x in 5..11 {
                let at = (y * 20 + x) * 4;
                rect[at..at + 4].copy_from_slice(&[0xff, 10, 20, 30]);
            }
        }
        let centered = Icon { width: 20, height: 20, argb: rect }.trim_to_square();
        assert_eq!((centered.width, centered.height), (6, 6));
        for y in 0..6 {
            for x in 0..6 {
                let pixel = &centered.argb[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4];
                if (1..5).contains(&y) {
                    assert_eq!(pixel, &[0xff, 10, 20, 30]);
                } else {
                    assert_eq!(pixel, &[0, 0, 0, 0]);
                }
            }
        }

        let blank = Icon { width: 8, height: 8, argb: vec![0u8; 8 * 8 * 4] }.trim_to_square();
        assert_eq!((blank.width, blank.height), (8, 8));
        assert!(blank.is_blank());
    }

    #[test]
    fn shape_mask_clears_pixels_outside_bounding_rectangles_for_depth_24() {
        let data = [10u8, 20, 30, 0].repeat(16);
        let mut icon = from_zpixmap(&data, 4, 4, 24).expect("4x4 depth-24");
        icon.apply_shape_mask(&[(1, 1, 2, 2)]);
        assert!(!icon.is_blank());
        let trimmed = icon.trim_to_square();
        assert_eq!((trimmed.width, trimmed.height), (2, 2));
        assert!(trimmed.argb.chunks_exact(4).all(|pixel| pixel == [0xff, 30, 20, 10]));

        let mut empty = from_zpixmap(&data, 4, 4, 24).expect("4x4 depth-24");
        empty.apply_shape_mask(&[]);
        assert!(empty.is_blank());
    }

    #[test]
    fn unpainted_depth_24_black_frame_is_detected_until_damaged() {
        let black = from_zpixmap(&[0u8; 16], 2, 2, 24).expect("2x2 depth-24");
        assert!(black.is_opaque_black());
        assert!(black.is_unpainted_opaque(24, false));
        assert!(!black.is_unpainted_opaque(24, true));
        assert!(!black.is_unpainted_opaque(32, false));

        let mut painted_data = [0u8; 16];
        painted_data[0] = 42;
        let painted = from_zpixmap(&painted_data, 2, 2, 24).expect("2x2 depth-24");
        assert!(!painted.is_opaque_black());
        assert!(!painted.is_unpainted_opaque(24, false));
    }
}
