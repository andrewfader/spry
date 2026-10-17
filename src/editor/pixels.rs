//! `Pixels` is a rectangular grid of `Rgba`. It's the image inside every cel,
//! clipboard, floating selection and version thumbnail.
//!
//! Coordinates are signed so tools can step off the canvas while you drag.
//! Out-of-bounds reads give transparent and out-of-bounds writes do nothing,
//! which spares every tool from clipping on its own.

use super::color::Rgba;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Pixels {
    width: u32,
    height: u32,
    data: Vec<Rgba>,
}

/// An axis-aligned rectangle in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Rect { x, y, width, height }
    }

    /// The rectangle spanned by two corners, inclusive, in any order. This is
    /// what a drag from one pixel to another means.
    pub fn spanning(a: (i32, i32), b: (i32, i32)) -> Self {
        let (left, right) = (a.0.min(b.0), a.0.max(b.0));
        let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
        Rect::new(left, top, (right - left + 1) as u32, (bottom - top + 1) as u32)
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            Rect::new(left, top, 0, 0)
        } else {
            Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
        }
    }

    pub fn points(self) -> impl Iterator<Item = (i32, i32)> {
        (self.y..self.bottom()).flat_map(move |y| (self.x..self.right()).map(move |x| (x, y)))
    }
}

impl Pixels {
    pub fn new(width: u32, height: u32) -> Self {
        Self::filled(width, height, Rgba::TRANSPARENT)
    }

    pub fn filled(width: u32, height: u32, colour: Rgba) -> Self {
        Pixels { width, height, data: vec![colour; width as usize * height as usize] }
    }

    pub fn from_rgba_bytes(width: u32, height: u32, bytes: &[u8]) -> Option<Self> {
        if Some(bytes.len()) != (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)) {
            return None;
        }
        let data = bytes.chunks_exact(4).map(|c| Rgba([c[0], c[1], c[2], c[3]])).collect();
        Some(Pixels { width, height, data })
    }

    pub fn to_rgba_bytes(&self) -> Vec<u8> {
        self.data.iter().flat_map(|c| c.0).collect()
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        self.contains(x, y).then(|| y as usize * self.width as usize + x as usize)
    }

    pub fn get(&self, x: i32, y: i32) -> Rgba {
        self.index(x, y).map_or(Rgba::TRANSPARENT, |i| self.data[i])
    }

    pub fn set(&mut self, x: i32, y: i32, colour: Rgba) {
        if let Some(i) = self.index(x, y) {
            self.data[i] = colour;
        }
    }

    pub fn colours(&self) -> &[Rgba] {
        &self.data
    }

    pub fn colours_mut(&mut self) -> &mut [Rgba] {
        &mut self.data
    }

    pub fn is_blank(&self) -> bool {
        self.data.iter().all(|c| c.is_transparent())
    }

    /// The smallest rectangle holding every visible pixel, or `None` when the
    /// image is blank. Sprite-sheet trimming and auto-cropping use this.
    pub fn opaque_bounds(&self) -> Option<Rect> {
        let mut found: Option<(i32, i32, i32, i32)> = None;
        for (x, y) in self.bounds().points() {
            if !self.get(x, y).is_transparent() {
                let (l, t, r, b) = found.unwrap_or((x, y, x, y));
                found = Some((l.min(x), t.min(y), r.max(x), b.max(y)));
            }
        }
        found.map(|(l, t, r, b)| Rect::spanning((l, t), (r, b)))
    }

    /// Copies a region out into a new image. Parts of the region outside
    /// this image come out transparent.
    pub fn crop(&self, region: Rect) -> Pixels {
        let mut out = Pixels::new(region.width, region.height);
        for (x, y) in out.bounds().points() {
            out.set(x, y, self.get(region.x + x, region.y + y));
        }
        out
    }

    /// Writes every pixel of `source` at an offset, replacing what's there.
    pub fn paste(&mut self, source: &Pixels, at_x: i32, at_y: i32) {
        for (x, y) in source.bounds().points() {
            self.set(at_x + x, at_y + y, source.get(x, y));
        }
    }

    /// Like `paste`, but composites with alpha and skips transparent source
    /// pixels. This is how a floating selection lands on art.
    pub fn stamp(&mut self, source: &Pixels, at_x: i32, at_y: i32, opacity: u8) {
        for (x, y) in source.bounds().points() {
            let paint = source.get(x, y);
            if !paint.is_transparent() {
                let below = self.get(at_x + x, at_y + y);
                self.set(at_x + x, at_y + y, paint.over(below, opacity));
            }
        }
    }

    pub fn flipped_horizontally(&self) -> Pixels {
        self.remapped(self.width, self.height, |x, y| (self.width as i32 - 1 - x, y))
    }

    pub fn flipped_vertically(&self) -> Pixels {
        self.remapped(self.width, self.height, |x, y| (x, self.height as i32 - 1 - y))
    }

    pub fn rotated_clockwise(&self) -> Pixels {
        self.remapped(self.height, self.width, |x, y| (y, self.height as i32 - 1 - x))
    }

    pub fn rotated_counter_clockwise(&self) -> Pixels {
        self.remapped(self.height, self.width, |x, y| (self.width as i32 - 1 - y, x))
    }

    /// Builds a new image where each output pixel `(x, y)` takes its colour
    /// from `source_of(x, y)` in this one.
    fn remapped(&self, width: u32, height: u32, source_of: impl Fn(i32, i32) -> (i32, i32)) -> Pixels {
        let mut out = Pixels::new(width, height);
        for (x, y) in out.bounds().points() {
            let (sx, sy) = source_of(x, y);
            out.set(x, y, self.get(sx, sy));
        }
        out
    }

    /// Nearest-neighbour upscale: each pixel becomes a `factor`×`factor`
    /// block, which is the only honest way to enlarge pixel art.
    pub fn scaled_up(&self, factor: u32) -> Pixels {
        let factor = factor.max(1);
        let mut out = Pixels::new(self.width.saturating_mul(factor), self.height.saturating_mul(factor));
        for (x, y) in out.bounds().points() {
            out.set(x, y, self.get(x / factor as i32, y / factor as i32));
        }
        out
    }

    /// Resizes the canvas without resampling. The old image is placed at
    /// `(offset_x, offset_y)` in the new one.
    pub fn resized_canvas(&self, width: u32, height: u32, offset_x: i32, offset_y: i32) -> Pixels {
        let mut out = Pixels::new(width, height);
        out.paste(self, offset_x, offset_y);
        out
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({}×{})", self.width, self.height)
    }
}

/// On disk an image is `{ "w", "h", "rgba": base64 }`. The `.spry`
/// container is gzipped as a whole, so raw bytes compress about as well as
/// PNG and we skip a codec round trip.
#[derive(Serialize, Deserialize)]
struct PixelsOnDisk {
    w: u32,
    h: u32,
    rgba: String,
}

impl Serialize for Pixels {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let rgba = base64::engine::general_purpose::STANDARD.encode(self.to_rgba_bytes());
        PixelsOnDisk { w: self.width, h: self.height, rgba }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Pixels {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let disk = PixelsOnDisk::deserialize(deserializer)?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(disk.rgba).map_err(D::Error::custom)?;
        Pixels::from_rgba_bytes(disk.w, disk.h, &bytes)
            .ok_or_else(|| D::Error::custom("image data does not match its width and height"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba::rgb(255, 0, 0);

    fn l_shape() -> Pixels {
        // X.
        // XX
        let mut image = Pixels::new(2, 2);
        image.set(0, 0, RED);
        image.set(0, 1, RED);
        image.set(1, 1, RED);
        image
    }

    #[test]
    fn reads_outside_the_image_are_transparent_and_writes_are_ignored() {
        let mut image = Pixels::new(2, 2);
        image.set(-1, 5, RED);
        assert_eq!(image.get(-1, 5), Rgba::TRANSPARENT);
        assert!(image.is_blank());
    }

    #[test]
    fn rotating_clockwise_four_times_is_the_identity() {
        let image = l_shape();
        let spun = image.rotated_clockwise().rotated_clockwise().rotated_clockwise().rotated_clockwise();
        assert_eq!(spun, image);
    }

    #[test]
    fn rotating_clockwise_moves_the_top_left_to_the_top_right() {
        let mut image = Pixels::new(3, 2);
        image.set(0, 0, RED);
        let rotated = image.rotated_clockwise();
        assert_eq!((rotated.width(), rotated.height()), (2, 3));
        assert_eq!(rotated.get(1, 0), RED);
    }

    #[test]
    fn counter_clockwise_undoes_clockwise() {
        let image = l_shape();
        assert_eq!(image.rotated_clockwise().rotated_counter_clockwise(), image);
    }

    #[test]
    fn flips_mirror_across_the_centre() {
        let image = l_shape();
        assert_eq!(image.flipped_horizontally().get(1, 0), RED);
        assert_eq!(image.flipped_vertically().get(0, 0), RED);
        assert_eq!(image.flipped_vertically().get(1, 0), RED);
    }

    #[test]
    fn opaque_bounds_hug_the_visible_pixels() {
        let mut image = Pixels::new(10, 10);
        image.set(2, 3, RED);
        image.set(5, 7, RED);
        assert_eq!(image.opaque_bounds(), Some(Rect::new(2, 3, 4, 5)));
        assert_eq!(Pixels::new(4, 4).opaque_bounds(), None);
    }

    #[test]
    fn crop_then_paste_restores_the_region() {
        let mut image = Pixels::new(4, 4);
        image.set(1, 1, RED);
        let piece = image.crop(Rect::new(1, 1, 2, 2));
        let mut blank = Pixels::new(4, 4);
        blank.paste(&piece, 1, 1);
        assert_eq!(blank, image);
    }

    #[test]
    fn stamping_keeps_pixels_under_transparent_source() {
        let mut base = Pixels::filled(2, 1, Rgba::BLACK);
        let mut sticker = Pixels::new(2, 1);
        sticker.set(1, 0, RED);
        base.stamp(&sticker, 0, 0, 255);
        assert_eq!(base.get(0, 0), Rgba::BLACK);
        assert_eq!(base.get(1, 0), RED);
    }

    #[test]
    fn scaling_up_turns_each_pixel_into_a_block() {
        let scaled = l_shape().scaled_up(3);
        assert_eq!((scaled.width(), scaled.height()), (6, 6));
        assert_eq!(scaled.get(2, 2), RED);
        assert_eq!(scaled.get(3, 2), Rgba::TRANSPARENT);
        assert_eq!(scaled.get(5, 5), RED);
    }

    #[test]
    fn spanning_rect_is_inclusive_in_any_drag_direction() {
        assert_eq!(Rect::spanning((3, 4), (1, 1)), Rect::new(1, 1, 3, 4));
    }

    #[test]
    fn rect_intersection_can_be_empty() {
        let a = Rect::new(0, 0, 4, 4);
        assert_eq!(a.intersect(&Rect::new(2, 2, 4, 4)), Rect::new(2, 2, 2, 2));
        assert!(a.intersect(&Rect::new(10, 10, 1, 1)).is_empty());
    }

    #[test]
    fn serialises_losslessly() {
        let image = l_shape();
        let json = serde_json::to_string(&image).unwrap();
        assert_eq!(serde_json::from_str::<Pixels>(&json).unwrap(), image);
    }

    #[test]
    fn rejects_image_data_of_the_wrong_length() {
        let bogus = r#"{"w":4,"h":4,"rgba":"AAAA"}"#;
        assert!(serde_json::from_str::<Pixels>(bogus).is_err());
    }
}
