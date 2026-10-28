//! `app-icon`: draws the KChess app icon and writes every size and format the app needs.
//!
//! The icon is original artwork made of a few geometric shapes (a rook on a rounded tile), so it
//! ships under the project's licence. Outputs: `apps/gpui/build/icon.{svg,png,ico,icns}`,
//! `icon-{light,dark}.png` (the macOS-style inset tiles the Dock shows), `apps/gpui/assets/icon.png`
//! and `icon-splash.png`, and the inline SVG of the sign-in page
//! (`crates/kchess-domain/src/records/oauth.rs`). The `.icns` needs macOS' `iconutil`.

use std::path::Path;

use image::{ImageBuffer, Rgba, RgbaImage, imageops};
use tiny_skia::{FillRule, Mask, PathBuilder, Transform};

use crate::util::{Result, root, run, say};

/// The coordinate space the shapes are drawn in.
const DESIGN: f64 = 1024.0;
/// Draw large, then shrink, for smooth edges.
const SUPERSAMPLE: u32 = 3;

enum Shape {
    /// A rounded rectangle `(x0, y0, x1, y1)` with its corner radius.
    Rect([f64; 4], f64),
    Poly(&'static [f64]),
}

const ROOK: [Shape; 8] = [
    Shape::Rect([282.0, 745.0, 742.0, 800.0], 18.0), // base plate
    Shape::Rect([312.0, 690.0, 712.0, 752.0], 16.0), // foot
    Shape::Poly(&[382.0, 395.0, 642.0, 395.0, 682.0, 700.0, 342.0, 700.0]), // body, wider at the bottom
    Shape::Rect([322.0, 350.0, 702.0, 402.0], 12.0),                        // collar
    Shape::Rect([332.0, 300.0, 692.0, 352.0], 0.0), // band joining the battlements
    Shape::Rect([332.0, 250.0, 422.0, 352.0], 7.0), // left merlon
    Shape::Rect([467.0, 250.0, 557.0, 352.0], 7.0), // middle merlon
    Shape::Rect([602.0, 250.0, 692.0, 352.0], 7.0), // right merlon
];
/// How much of the tile the rook fills, and the rook's centre in DESIGN space.
const ROOK_SCALE: f64 = 1.12;
const ROOK_CENTER: (f64, f64) = (512.0, 525.0);

struct Theme {
    tile: ([u8; 3], [u8; 3]),
    rook: ([u8; 3], [u8; 3]),
    edge: [u8; 3],
}

/// A light tile with a dark rook, and the reverse for dark mode.
const LIGHT: Theme = Theme {
    tile: ([0xF7, 0xDC, 0x90], [0xD8, 0xA2, 0x3F]),
    rook: ([0x3B, 0x2D, 0x20], [0x1E, 0x16, 0x10]),
    edge: [0xB9, 0x85, 0x24],
};
const DARK: Theme = Theme {
    tile: ([0x7B, 0x5D, 0x30], [0x35, 0x26, 0x0F]),
    rook: ([0xFC, 0xF2, 0xD8], [0xE6, 0xD0, 0x9B]),
    edge: [0x1E, 0x14, 0x08],
};

/// Continuous corners extend farther along each tile edge; a superellipse makes the transition
/// into straight edges smooth.
const CORNER: f64 = 0.30;
const CORNER_POWER: f64 = 2.6;
/// The reference macOS icon's 824px tile on a 1024px canvas.
const MACOS_INSET: f64 = 100.0 / 1024.0;

fn hex(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

/// The continuous-corner outline shared by the raster and SVG icons.
fn tile_outline(size: f64) -> Vec<(f64, f64)> {
    let end = size - 1.0;
    let radius = end * CORNER;
    let mut points = Vec::new();
    for (cx, cy, start) in [
        (end - radius, radius, -90.0f64),
        (end - radius, end - radius, 0.0),
        (radius, end - radius, 90.0),
        (radius, radius, 180.0),
    ] {
        for step in 0..65 {
            let angle = (start + f64::from(step) * 90.0 / 64.0).to_radians();
            let (x, y) = (angle.cos(), angle.sin());
            points.push((
                cx + radius * x.abs().powf(2.0 / CORNER_POWER).copysign(x),
                cy + radius * y.abs().powf(2.0 / CORNER_POWER).copysign(y),
            ));
        }
    }
    points
}

/// A DESIGN-space point moved so the rook is centred and scaled within the tile.
fn place(x: f64, y: f64) -> (f64, f64) {
    (
        DESIGN / 2.0 + (x - ROOK_CENTER.0) * ROOK_SCALE,
        DESIGN / 2.0 + (y - ROOK_CENTER.1) * ROOK_SCALE,
    )
}

fn polygon(points: &[(f64, f64)]) -> Option<tiny_skia::Path> {
    let mut path = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    path.move_to(first.0 as f32, first.1 as f32);
    for (x, y) in rest {
        path.line_to(*x as f32, *y as f32);
    }
    path.close();
    path.finish()
}

fn rounded_rect(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Option<tiny_skia::Path> {
    let (x0, y0, x1, y1, r) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32, r as f32);
    if r <= 0.0 {
        return tiny_skia::Rect::from_ltrb(x0, y0, x1, y1).map(PathBuilder::from_rect);
    }
    // Quarter circles as cubic Béziers.
    let k = 0.552_284_8 * r;
    let mut path = PathBuilder::new();
    path.move_to(x0 + r, y0);
    path.line_to(x1 - r, y0);
    path.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
    path.line_to(x1, y1 - r);
    path.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    path.line_to(x0 + r, y1);
    path.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
    path.line_to(x0, y0 + r);
    path.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
    path.close();
    path.finish()
}

/// Coverage (0–255) of `paths` on a `size` square.
fn coverage(size: u32, paths: &[tiny_skia::Path]) -> Result<Vec<u8>> {
    let mut mask = Mask::new(size, size).ok_or("Cannot draw the icon.")?;
    for path in paths {
        mask.fill_path(path, FillRule::Winding, true, Transform::identity());
    }
    Ok(mask.data().to_vec())
}

fn rook_paths(size: u32) -> Vec<tiny_skia::Path> {
    let k = f64::from(size) / DESIGN;
    ROOK.iter()
        .filter_map(|shape| match shape {
            Shape::Poly(coords) => {
                let points: Vec<(f64, f64)> = coords
                    .chunks(2)
                    .map(|pair| {
                        let (x, y) = place(pair[0], pair[1]);
                        (x * k, y * k)
                    })
                    .collect();
                polygon(&points)
            }
            Shape::Rect([x0, y0, x1, y1], r) => {
                let (a, b) = (place(*x0, *y0), place(*x1, *y1));
                rounded_rect(a.0 * k, a.1 * k, b.0 * k, b.1 * k, r * ROOK_SCALE * k)
            }
        })
        .collect()
}

/// A separable Gaussian blur of a coverage mask.
fn blur(mask: &[u8], size: usize, sigma: f32) -> Vec<u8> {
    let radius = (sigma * 3.0).ceil() as isize;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let pass = |source: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0; size * size];
        for y in 0..size {
            for x in 0..size {
                let mut sum = 0.0;
                for (offset, weight) in (-radius..=radius).zip(&weights) {
                    let (sx, sy) = if horizontal {
                        (x as isize + offset, y as isize)
                    } else {
                        (x as isize, y as isize + offset)
                    };
                    if sx >= 0 && sy >= 0 && (sx as usize) < size && (sy as usize) < size {
                        sum += source[sy as usize * size + sx as usize] * weight;
                    }
                }
                out[y * size + x] = sum / total;
            }
        }
        out
    };
    let source: Vec<f32> = mask.iter().map(|&v| f32::from(v)).collect();
    let blurred = pass(&pass(&source, true), false);
    blurred
        .iter()
        .map(|&v| v.round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// A mask eroded by `width` pixels (PIL's `MinFilter`), as a square neighbourhood minimum.
fn erode(mask: &[u8], size: usize, width: usize) -> Vec<u8> {
    let pass = |source: &[u8], horizontal: bool| -> Vec<u8> {
        let mut out = vec![0; size * size];
        for y in 0..size {
            for x in 0..size {
                let mut min = 255u8;
                for d in 0..=2 * width {
                    let offset = d as isize - width as isize;
                    let (sx, sy) = if horizontal {
                        (x as isize + offset, y as isize)
                    } else {
                        (x as isize, y as isize + offset)
                    };
                    let value =
                        if sx >= 0 && sy >= 0 && (sx as usize) < size && (sy as usize) < size {
                            source[sy as usize * size + sx as usize]
                        } else {
                            0
                        };
                    min = min.min(value);
                }
                out[y * size + x] = min;
            }
        }
        out
    };
    pass(&pass(mask, true), false)
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t)
}

/// Straight-alpha "over" of a colour with coverage `alpha` (0–1) onto `pixel`.
fn over(pixel: &mut [f32; 4], color: [f32; 3], alpha: f32) {
    let out_alpha = alpha + pixel[3] * (1.0 - alpha);
    if out_alpha <= 0.0 {
        return;
    }
    for i in 0..3 {
        pixel[i] = (color[i] * alpha + pixel[i] * pixel[3] * (1.0 - alpha)) / out_alpha;
    }
    pixel[3] = out_alpha;
}

/// The icon tile (rounded square, gradient, rook with a soft shadow) at `size` pixels.
fn render_tile(theme: &Theme, size: u32) -> Result<RgbaImage> {
    let big = size * SUPERSAMPLE;
    let n = big as usize;
    let outline = tile_outline(f64::from(big));
    let tile_mask = coverage(big, &polygon(&outline).into_iter().collect::<Vec<_>>())?;
    let rook = coverage(big, &rook_paths(big))?;
    let shadow = blur(&rook, n, big as f32 * 0.012);
    let shift = (big as f32 * 0.014) as usize;
    let width = (big / 256).max(2) as usize;
    let inner = erode(&tile_mask, n, width);
    let mut pixels = vec![[0.0f32; 4]; n * n];
    for y in 0..n {
        let t = y as f32 / (n - 1) as f32;
        let tile_color = lerp(theme.tile.0, theme.tile.1, t);
        let rook_color = lerp(theme.rook.0, theme.rook.1, t);
        for x in 0..n {
            let at = y * n + x;
            let tile_alpha = f32::from(tile_mask[at]) / 255.0;
            let mut pixel = [tile_color[0], tile_color[1], tile_color[2], tile_alpha];
            // The shadow sits below the rook, inside the tile.
            if y >= shift {
                let cast = f32::from(shadow[(y - shift) * n + x]) / 255.0 * 0.38 * tile_alpha;
                over(&mut pixel, [20.0, 10.0, 0.0], cast);
            }
            over(&mut pixel, rook_color, f32::from(rook[at]) / 255.0);
            // A thin darker edge keeps the tile readable on light backgrounds.
            let edge = f32::from(tile_mask[at].saturating_sub(inner[at])) / 255.0 * 0.55;
            over(&mut pixel, theme.edge.map(f32::from), edge);
            pixels[at] = pixel;
        }
    }
    let large: RgbaImage = ImageBuffer::from_fn(big, big, |x, y| {
        let p = pixels[y as usize * n + x as usize];
        Rgba([
            p[0].round() as u8,
            p[1].round() as u8,
            p[2].round() as u8,
            (p[3] * 255.0).round() as u8,
        ])
    });
    Ok(imageops::resize(
        &large,
        size,
        size,
        imageops::FilterType::Lanczos3,
    ))
}

/// The tile on a transparent square; `inset` leaves the margin macOS icons have.
fn framed(theme: &Theme, size: u32, inset: f64) -> Result<RgbaImage> {
    let inner = (f64::from(size) * (1.0 - 2.0 * inset)).round() as u32;
    let tile = render_tile(theme, inner)?;
    let mut canvas = RgbaImage::new(size, size);
    let offset = (f64::from(size) * inset).round() as i64;
    imageops::overlay(&mut canvas, &tile, offset, offset);
    Ok(canvas)
}

/// The light icon as an SVG, from the same shapes.
pub fn svg() -> String {
    let mut shapes = Vec::new();
    for shape in &ROOK {
        match shape {
            Shape::Poly(coords) => {
                let points: Vec<String> = coords
                    .chunks(2)
                    .map(|pair| {
                        let (x, y) = place(pair[0], pair[1]);
                        format!("{x:.1},{y:.1}")
                    })
                    .collect();
                shapes.push(format!("    <polygon points=\"{}\"/>", points.join(" ")));
            }
            Shape::Rect([x0, y0, x1, y1], r) => {
                let (a, b) = (place(*x0, *y0), place(*x1, *y1));
                shapes.push(format!(
                    "    <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"{:.1}\"/>",
                    a.0,
                    a.1,
                    b.0 - a.0,
                    b.1 - a.1,
                    r * ROOK_SCALE
                ));
            }
        }
    }
    let outline: Vec<String> = tile_outline(DESIGN)
        .iter()
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="512" height="512">
  <title>KChess</title>
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{}"/>
      <stop offset="1" stop-color="{}"/>
    </linearGradient>
    <linearGradient id="rook" gradientUnits="userSpaceOnUse" x1="0" y1="{:.0}" x2="0" y2="{:.0}">
      <stop offset="0" stop-color="{}"/>
      <stop offset="1" stop-color="{}"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="14" stdDeviation="12" flood-color="#140a00" flood-opacity="0.38"/>
    </filter>
  </defs>
  <polygon points="{}" fill="url(#tile)"/>
  <g fill="url(#rook)" filter="url(#shadow)">
{}
  </g>
</svg>
"##,
        hex(LIGHT.tile.0),
        hex(LIGHT.tile.1),
        place(0.0, 250.0).1,
        place(0.0, 800.0).1,
        hex(LIGHT.rook.0),
        hex(LIGHT.rook.1),
        outline.join(" "),
        shapes.join("\n")
    )
}

/// The SVG as the sign-in page inlines it: no namespace, title, size or line breaks.
pub fn inline_svg() -> String {
    let text = svg()
        .replace(" xmlns=\"http://www.w3.org/2000/svg\"", "")
        .replace("<title>KChess</title>", "")
        .replace(" width=\"512\" height=\"512\"", "");
    text.lines().map(str::trim).collect::<Vec<_>>().join("")
}

fn save(image: &RgbaImage, path: &Path) -> Result {
    image
        .save(path)
        .map_err(|cause| format!("Cannot write {}: {cause}", path.display()))
}

fn write_ico(master: &RgbaImage, path: &Path) -> Result {
    let frames: Vec<image::codecs::ico::IcoFrame<'static>> = [16u32, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&size| {
            let resized = imageops::resize(master, size, size, imageops::FilterType::Lanczos3);
            image::codecs::ico::IcoFrame::as_png(
                resized.as_raw(),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|cause| cause.to_string())
        })
        .collect::<Result<_>>()?;
    let file = std::fs::File::create(path).map_err(|cause| cause.to_string())?;
    image::codecs::ico::IcoEncoder::new(file)
        .encode_images(&frames)
        .map_err(|cause| cause.to_string())
}

fn write_icns(master: &RgbaImage, path: &Path) -> Result {
    if !cfg!(target_os = "macos") {
        say("Skipped icon.icns (needs macOS iconutil).");
        return Ok(());
    }
    let dir = tempfile::tempdir().map_err(|cause| cause.to_string())?;
    let iconset = dir.path().join("icon.iconset");
    std::fs::create_dir_all(&iconset).map_err(|cause| cause.to_string())?;
    for base in [16u32, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            let resized = imageops::resize(master, size, size, imageops::FilterType::Lanczos3);
            save(
                &resized,
                &iconset.join(format!("icon_{base}x{base}{suffix}.png")),
            )?;
        }
    }
    let (from, to) = (
        iconset.to_string_lossy().to_string(),
        path.to_string_lossy().to_string(),
    );
    run("iconutil", &["-c", "icns", &from, "-o", &to])
}

/// Replace the sign-in page's inline icon with `icon`.
fn write_page_icon(icon: &str) -> Result {
    let path = root().join("crates/kchess-domain/src/records/oauth.rs");
    let text = std::fs::read_to_string(&path).map_err(|cause| cause.to_string())?;
    let start = text
        .find("const APP_ICON_SVG: &str = r##\"")
        .ok_or("oauth.rs has no APP_ICON_SVG.")?;
    let body = start + "const APP_ICON_SVG: &str = r##\"".len();
    let end = body
        + text[body..]
            .find("\"##;")
            .ok_or("APP_ICON_SVG is not closed.")?;
    let next = format!("{}{icon}{}", &text[..body], &text[end..]);
    std::fs::write(&path, next).map_err(|cause| cause.to_string())
}

pub fn write() -> Result {
    let build = root().join("apps/gpui/build");
    let assets = root().join("apps/gpui/assets");
    std::fs::write(build.join("icon.svg"), svg()).map_err(|cause| cause.to_string())?;
    write_page_icon(&inline_svg())?;
    // Edge to edge: the window icon, the startup screen, Linux.
    let full = framed(&LIGHT, 512, 0.0)?;
    save(&full, &build.join("icon.png"))?;
    save(&full, &assets.join("icon.png"))?;
    save(
        &framed(&LIGHT, 512, MACOS_INSET)?,
        &build.join("icon-light.png"),
    )?;
    save(
        &framed(&DARK, 512, MACOS_INSET)?,
        &build.join("icon-dark.png"),
    )?;
    // The startup screen shows it at 72 points; this is its 2x raster.
    save(&framed(&LIGHT, 144, 0.0)?, &assets.join("icon-splash.png"))?;
    write_ico(&framed(&LIGHT, 256, 0.0)?, &build.join("icon.ico"))?;
    write_icns(
        &framed(&LIGHT, 1024, MACOS_INSET)?,
        &build.join("icon.icns"),
    )?;
    say("Icons written.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-in page's icon is this drawing: regenerating changes nothing.
    #[test]
    fn the_sign_in_pages_icon_is_the_drawn_one() {
        let page =
            std::fs::read_to_string(root().join("crates/kchess-domain/src/records/oauth.rs"))
                .unwrap();
        assert!(page.contains(&format!(
            "const APP_ICON_SVG: &str = r##\"{}\"##;",
            inline_svg()
        )));
    }

    #[test]
    fn the_svg_matches_the_committed_one() {
        let committed = std::fs::read_to_string(root().join("apps/gpui/build/icon.svg")).unwrap();
        assert_eq!(committed, svg());
    }
}
