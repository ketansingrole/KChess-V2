//! The picture's pixels: decoded, any transparency laid over white, a large one scaled down.

use image::DynamicImage;
use image::imageops::FilterType;

/// Pictures larger than this on their longer side are scaled down first: a board filling such a
/// picture still has squares of 200 pixels, and every later step walks each pixel.
const MAX_SIDE: u32 = 1600;
/// The widest or tallest picture decoded at all.
const MAX_DIMENSION: u32 = 16_384;
/// The most memory decoding one picture may take.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// A picture as colours, row by row from the top left, each channel 0 to 255.
pub(crate) struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<[f32; 3]>,
}

impl Picture {
    /// The picture in `bytes` (PNG, JPEG, GIF, WebP, BMP or TIFF), or `None` when it is none of
    /// them or too large to decode.
    pub fn decode(bytes: &[u8]) -> Option<Picture> {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(MAX_DECODE_BYTES);
        reader.limits(limits);
        reader.decode().ok().map(|image| Picture::of(&image))
    }

    /// `image`'s pixels, over white where it is transparent, scaled down to `MAX_SIDE`.
    pub fn of(image: &DynamicImage) -> Picture {
        let scaled;
        let image = if image.width().max(image.height()) > MAX_SIDE {
            scaled = image.resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle);
            &scaled
        } else {
            image
        };
        let rgba = image.to_rgba8();
        let rgb = rgba
            .pixels()
            .map(|pixel| {
                let [r, g, b, a] = pixel.0;
                let alpha = f32::from(a) / 255.;
                let over_white = |channel: u8| f32::from(channel) * alpha + 255. * (1. - alpha);
                [over_white(r), over_white(g), over_white(b)]
            })
            .collect();
        Picture {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            rgb,
        }
    }

    /// Each pixel's brightness (Rec. 601), 0 to 255.
    pub fn luma(&self) -> Vec<f32> {
        self.rgb.iter().map(|&pixel| luma(pixel)).collect()
    }
}

/// How bright a colour is, 0 to 255.
pub(crate) fn luma([r, g, b]: [f32; 3]) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// How far apart two colours are.
pub(crate) fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [dr, dg, db] = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (dr * dr + dg * dg + db * db).sqrt()
}

/// `values`' median, `None` when there are none. Reorders them.
pub(crate) fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let middle = values.len() / 2;
    let (_, value, _) = values.select_nth_unstable_by(middle, f32::total_cmp);
    Some(*value)
}

/// The colour whose every channel is the channels' median, `None` when there are none.
pub(crate) fn median_colour(colours: impl Iterator<Item = [f32; 3]>) -> Option<[f32; 3]> {
    let mut channels: [Vec<f32>; 3] = Default::default();
    for colour in colours {
        for (channel, value) in channels.iter_mut().zip(colour) {
            channel.push(value);
        }
    }
    let [r, g, b] = &mut channels;
    Some([median(r)?, median(g)?, median(b)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transparent_picture_lies_over_white_and_a_large_one_is_scaled_down() {
        let mut image = image::RgbaImage::new(3200, 100);
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let picture = Picture::of(&DynamicImage::ImageRgba8(image));
        assert_eq!((picture.width, picture.height), (1600, 50));
        assert_eq!(picture.rgb[picture.width * 10 + 800], [255.; 3]);
        assert!(Picture::decode(b"not a picture").is_none());
    }

    #[test]
    fn medians_and_distances() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [3., 1., 2.]), Some(2.));
        assert_eq!(
            median_colour([[1., 9., 0.], [2., 8., 0.], [3., 7., 0.]].into_iter()),
            Some([2., 8., 0.])
        );
        assert_eq!(distance([0., 3., 0.], [4., 0., 0.]), 5.);
        assert_eq!(luma([255.; 3]).round(), 255.);
    }
}
