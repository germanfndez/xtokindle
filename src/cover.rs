//! Kindle covers: turns wide banners into a portrait cover.
//!
//! Kindle shows covers in portrait and crops anything wider. We letterbox the
//! banner into a portrait canvas so the whole image stays visible.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{Rgb, RgbImage};

/// Cover size recommended by Amazon (1:1.6).
pub const COVER_WIDTH: u32 = 1600;
pub const COVER_HEIGHT: u32 = 2560;

/// Images at or below this width/height ratio are already portrait enough.
const MAX_PORTRAIT_RATIO: f32 = 0.8;
const JPEG_QUALITY: u8 = 88;

/// Letterboxes a wide image into a 1600x2560 JPEG.
///
/// Returns `None` when the original should be kept: the bytes are not a
/// decodable image, or the image is already portrait.
pub fn make_kindle_cover(bytes: &[u8]) -> Option<Vec<u8>> {
    let source = image::load_from_memory(bytes).ok()?.to_rgb8();
    let (width, height) = source.dimensions();
    if width as f32 / height as f32 <= MAX_PORTRAIT_RATIO {
        return None;
    }

    // Scale to the canvas width; if that is too tall, fit by height instead.
    let mut new_width = COVER_WIDTH;
    let mut new_height = (height * COVER_WIDTH / width).max(1);
    if new_height > COVER_HEIGHT {
        new_height = COVER_HEIGHT;
        new_width = (width * COVER_HEIGHT / height).max(1);
    }
    let scaled = imageops::resize(&source, new_width, new_height, FilterType::Lanczos3);

    let mut canvas = RgbImage::from_pixel(COVER_WIDTH, COVER_HEIGHT, edge_color(&source));
    let x = (COVER_WIDTH - new_width) / 2;
    let y = (COVER_HEIGHT - new_height) / 2;
    imageops::replace(&mut canvas, &scaled, i64::from(x), i64::from(y));

    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&canvas)
        .ok()?;
    Some(out.into_inner())
}

/// Average color of the image border (top and bottom rows, left and right columns).
fn edge_color(img: &RgbImage) -> Rgb<u8> {
    let (width, height) = img.dimensions();
    let mut sums = [0u64; 3];
    let mut count = 0u64;
    for (x, y, pixel) in img.enumerate_pixels() {
        if x == 0 || y == 0 || x == width - 1 || y == height - 1 {
            for (sum, channel) in sums.iter_mut().zip(pixel.0) {
                *sum += u64::from(channel);
            }
            count += 1;
        }
    }
    Rgb(sums.map(|sum| (sum / count) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageFormat;

    const BANNER: Rgb<u8> = Rgb([200, 30, 30]);
    const EDGE: Rgb<u8> = Rgb([20, 40, 200]);

    /// A banner of `w`x`h` with a `EDGE`-colored 2px frame and a `BANNER` center.
    fn framed(w: u32, h: u32) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, EDGE);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                img.put_pixel(x, y, BANNER);
            }
        }
        img
    }

    fn encode(img: &RgbImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn cover_of(img: &RgbImage) -> RgbImage {
        let jpeg = make_kindle_cover(&encode(img, ImageFormat::Png)).expect("should convert");
        image::load_from_memory(&jpeg).unwrap().to_rgb8()
    }

    /// JPEG is lossy, so compare colors with a tolerance.
    fn close(a: &Rgb<u8>, b: &Rgb<u8>) -> bool {
        a.0.iter().zip(b.0).all(|(x, y)| x.abs_diff(y) <= 12)
    }

    #[test]
    fn edge_color_of_a_uniform_image_is_that_color() {
        let img = RgbImage::from_pixel(10, 6, Rgb([10, 20, 30]));
        assert_eq!(edge_color(&img), Rgb([10, 20, 30]));
    }

    #[test]
    fn edge_color_ignores_the_center() {
        assert_eq!(edge_color(&framed(20, 10)), EDGE);
    }

    #[test]
    fn edge_color_averages_different_edges() {
        let mut img = RgbImage::from_pixel(4, 4, Rgb([0, 0, 0]));
        for x in 0..4 {
            img.put_pixel(x, 0, Rgb([100, 100, 100]));
        }
        // Border has 12 pixels: 4 at 100 and 8 at 0 -> 33.
        assert_eq!(edge_color(&img), Rgb([33, 33, 33]));
    }

    #[test]
    fn wide_banner_becomes_a_portrait_cover() {
        let cover = cover_of(&framed(1920, 768));
        assert_eq!(cover.dimensions(), (COVER_WIDTH, COVER_HEIGHT));
    }

    #[test]
    fn banner_sits_in_the_middle_with_edge_color_around() {
        let cover = cover_of(&framed(1920, 768));
        assert!(close(
            cover.get_pixel(COVER_WIDTH / 2, COVER_HEIGHT / 2),
            &BANNER
        ));
        assert!(close(cover.get_pixel(COVER_WIDTH / 2, 50), &EDGE));
        assert!(close(
            cover.get_pixel(COVER_WIDTH / 2, COVER_HEIGHT - 50),
            &EDGE
        ));
    }

    #[test]
    fn works_with_jpeg_input() {
        let jpeg = encode(&framed(1000, 400), ImageFormat::Jpeg);
        let out = make_kindle_cover(&jpeg).expect("should convert");
        let cover = image::load_from_memory(&out).unwrap();
        assert_eq!((cover.width(), cover.height()), (COVER_WIDTH, COVER_HEIGHT));
    }

    #[test]
    fn very_wide_image_is_letterboxed() {
        let cover = cover_of(&framed(3000, 300));
        assert_eq!(cover.dimensions(), (COVER_WIDTH, COVER_HEIGHT));
        assert!(close(
            cover.get_pixel(COVER_WIDTH / 2, COVER_HEIGHT / 2),
            &BANNER
        ));
        assert!(close(cover.get_pixel(COVER_WIDTH / 2, 100), &EDGE));
    }

    #[test]
    fn square_image_is_processed_and_centered() {
        let cover = cover_of(&framed(400, 400));
        assert_eq!(cover.dimensions(), (COVER_WIDTH, COVER_HEIGHT));
        assert!(close(
            cover.get_pixel(COVER_WIDTH / 2, COVER_HEIGHT / 2),
            &BANNER
        ));
        assert!(close(cover.get_pixel(COVER_WIDTH / 2, 100), &EDGE));
    }

    #[test]
    fn portrait_image_is_kept() {
        let portrait = encode(&framed(800, 1280), ImageFormat::Png);
        assert!(make_kindle_cover(&portrait).is_none());
    }

    #[test]
    fn garbage_bytes_are_kept() {
        assert!(make_kindle_cover(b"not an image").is_none());
    }
}
