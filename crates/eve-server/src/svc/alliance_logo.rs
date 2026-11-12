//! Imports a public image and prepares it for the legacy alliance logo slot.

use std::io::Cursor;
use std::net::IpAddr;
use std::time::Duration;

use image::{io::Reader, DynamicImage, ImageFormat, RgbaImage};
use reqwest::{header::LOCATION, redirect::Policy, Url};
use tokio::sync::Semaphore;

const LOGO_SIZE: u32 = 128;
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_REDIRECTS: usize = 3;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(12);
const INVALID_URL: &str = "Enter a public HTTP or HTTPS image URL (up to 2,048 characters).";
const DOWNLOAD_FAILED: &str = "The logo could not be downloaded. Use a direct link to a public image.";
const IMAGE_TOO_LARGE: &str = "The logo image must be 4 MB or smaller.";
const INVALID_IMAGE: &str = "Use a PNG, JPEG, GIF or WebP image up to 4,096 pixels on each side.";

static IMPORTS: Semaphore = Semaphore::const_new(4);

pub async fn import(source: &str) -> Result<Vec<u8>, &'static str> {
    if source.len() > 2048 || source.chars().any(char::is_control) {
        return Err(INVALID_URL);
    }
    let url = Url::parse(source).map_err(|_| INVALID_URL)?;
    public_endpoint(&url)?;
    let permit = IMPORTS.try_acquire().map_err(|_| "Too many logo imports are in progress. Try again shortly.")?;
    let bytes = tokio::time::timeout(DOWNLOAD_TIMEOUT, download(url)).await
        .map_err(|_| DOWNLOAD_FAILED)??;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepare(&bytes)
    }).await.map_err(|_| INVALID_IMAGE)?
}

fn public_endpoint(url: &Url) -> Result<(&str, u16), &'static str> {
    let port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return Err(INVALID_URL),
    };
    if url.port_or_known_default() != Some(port) || !url.username().is_empty() || url.password().is_some() {
        return Err(INVALID_URL);
    }
    let host = url.host_str().ok_or(INVALID_URL)?.trim_matches(['[', ']']);
    if host.parse::<IpAddr>().is_ok_and(|ip| !public_ip(ip)) {
        return Err(INVALID_URL);
    }
    Ok((host, port))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_documentation()
                && !matches!((a, b, c), (0 | 224..=255, _, _) | (100, 64..=127, _) | (192, 0, 0)
                    | (192, 88, 99) | (198, 18 | 19, _))
        }
        IpAddr::V6(ip) => {
            // Global unicast only, excluding special protocol and documentation ranges.
            let [a, b, ..] = ip.segments();
            matches!(a, 0x2000..=0x3fff) && !matches!(a, 0x2002 | 0x3fff)
                && !(a == 0x2001 && (b < 0x0200 || b == 0x0db8))
        }
    }
}

async fn download(mut url: Url) -> Result<Vec<u8>, &'static str> {
    for redirect in 0..=MAX_REDIRECTS {
        let (host, port) = public_endpoint(&url)?;
        let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await.map_err(|_| DOWNLOAD_FAILED)?
            .filter(|address| public_ip(address.ip())).collect();
        if addresses.is_empty() {
            return Err(INVALID_URL);
        }
        // Pin the validated answers to this request and recheck every redirect.
        let client = reqwest::Client::builder().no_proxy().redirect(Policy::none())
            .resolve_to_addrs(host, &addresses).build().map_err(|_| DOWNLOAD_FAILED)?;
        let mut response = client.get(url.clone()).send().await.map_err(|_| DOWNLOAD_FAILED)?;
        if response.status().is_redirection() && redirect < MAX_REDIRECTS {
            let location = response.headers().get(LOCATION).and_then(|value| value.to_str().ok()).ok_or(DOWNLOAD_FAILED)?;
            url = url.join(location).map_err(|_| INVALID_URL)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(DOWNLOAD_FAILED);
        }
        if response.content_length().is_some_and(|size| size > MAX_IMAGE_BYTES as u64) {
            return Err(IMAGE_TOO_LARGE);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| DOWNLOAD_FAILED)? {
            if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(IMAGE_TOO_LARGE);
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err(DOWNLOAD_FAILED)
}

fn prepare(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut reader = Reader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| INVALID_IMAGE)?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP)) {
        return Err(INVALID_IMAGE);
    }
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    reader.limits(limits);
    let mut source = reader.decode().map_err(|_| INVALID_IMAGE)?.into_rgba8();
    strip_checkerboard_background(&mut source);
    let fitted = DynamicImage::ImageRgba8(source).thumbnail(LOGO_SIZE, LOGO_SIZE).into_rgba8();
    let (width, height) = fitted.dimensions();
    let mut logo = RgbaImage::new(LOGO_SIZE, LOGO_SIZE);
    image::imageops::replace(&mut logo, &fitted, i64::from((LOGO_SIZE - width) / 2), i64::from((LOGO_SIZE - height) / 2));
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(logo).write_to(&mut output, ImageFormat::Png).map_err(|_| INVALID_IMAGE)?;
    Ok(output.into_inner())
}

/// Some image sites save the transparency preview as an opaque checkerboard.
/// Remove a regular light checkerboard only where it connects to the border.
fn strip_checkerboard_background(image: &mut RgbaImage) {
    let (width, height) = image.dimensions();
    if width < 8 || height < 8 || image.pixels().any(|pixel| pixel[3] != 255) {
        return;
    }

    let light_gray = |pixel: image::Rgba<u8>| pixel[0] >= 200 && pixel[0] == pixel[1] && pixel[0] == pixel[2];
    let first = *image.get_pixel(0, 0);
    let Some(tile) = (1..=64.min(width - 1)).find(|&x| *image.get_pixel(x, 0) != first) else { return };
    let second = *image.get_pixel(tile, 0);
    if !light_gray(first) || !light_gray(second) || !(10..=50).contains(&first[0].abs_diff(second[0]))
        || width < tile * 4 || height < tile * 4
    {
        return;
    }
    let background_at = |x: u32, y: u32| if (x / tile + y / tile) % 2 == 0 { first } else { second };
    let border: Vec<_> = (0..width).flat_map(|x| [(x, 0), (x, height - 1)])
        .chain((1..height - 1).flat_map(|y| [(0, y), (width - 1, y)])).collect();
    let matching = border.iter().filter(|&&(x, y)| *image.get_pixel(x, y) == background_at(x, y)).count();
    if matching * 10 < border.len() * 9 {
        return;
    }

    let mut pending = Vec::new();
    let visit = |x, y, image: &mut RgbaImage, pending: &mut Vec<_>| {
        if *image.get_pixel(x, y) == background_at(x, y) {
            image.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            pending.push((x, y));
        }
    };
    for (x, y) in border {
        visit(x, y, image, &mut pending);
    }
    while let Some((x, y)) = pending.pop() {
        if x > 0 { visit(x - 1, y, image, &mut pending); }
        if y > 0 { visit(x, y - 1, image, &mut pending); }
        if x + 1 < width { visit(x + 1, y, image, &mut pending); }
        if y + 1 < height { visit(x, y + 1, image, &mut pending); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_wide_logo_to_the_stock_slot() {
        let image = RgbaImage::from_fn(512, 256, |x, _| {
            let alpha = match x {
                0..=127 => 0,
                128..=255 => 96,
                _ => 255,
            };
            image::Rgba([10, 20, 30, alpha])
        });
        let mut original = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image).write_to(&mut original, ImageFormat::Png).unwrap();
        let fitted = prepare(original.get_ref()).unwrap();
        let logo = image::load_from_memory(&fitted).unwrap().into_rgba8();
        assert_eq!(logo.dimensions(), (128, 128));
        assert_eq!(logo.get_pixel(64, 31)[3], 0);
        assert_eq!(logo.get_pixel(64, 32).0, [10, 20, 30, 255]);
        assert_eq!(logo.get_pixel(64, 95)[3], 255);
        assert_eq!(logo.get_pixel(64, 96)[3], 0);
        assert_eq!(logo.get_pixel(16, 64)[3], 0, "transparent pixels in the source stay transparent");
        assert_eq!(logo.get_pixel(48, 64)[3], 96, "semitransparent pixels retain their opacity");
        assert!(prepare(b"<html>Not an image</html>").is_err());
    }

    #[test]
    fn rejects_local_destinations_and_non_web_urls() {
        for url in ["file:///logo.png", "http://2130706433/logo.png", "http://169.254.169.254/logo.png",
            "http://[::ffff:127.0.0.1]/logo.png", "http://[fd00::1]/logo.png", "http://10.0.0.5/logo.png",
            "http://example.com:5432/logo.png", "https://user:pass@example.com/logo.png"] {
            assert!(public_endpoint(&Url::parse(url).unwrap()).is_err(), "accepted {url}");
        }
        assert_eq!(public_endpoint(&Url::parse("https://example.com/logo.png").unwrap()), Ok(("example.com", 443)));
    }

    #[test]
    fn removes_the_checkerboard_without_keying_out_white_art() {
        let mut source = RgbaImage::from_fn(60, 50, |x, y| {
            let shade = if (x / 5 + y / 5) % 2 == 0 { 255 } else { 230 };
            image::Rgba([shade, shade, shade, 255])
        });
        for y in 15..35 {
            for x in 20..40 {
                source.put_pixel(x, y, image::Rgba([30, 60, 90, 255]));
            }
        }
        source.put_pixel(28, 22, image::Rgba([255, 255, 255, 255]));
        strip_checkerboard_background(&mut source);
        assert_eq!(source.pixels().filter(|pixel| pixel[3] == 0).count(), 2600);
        assert_eq!(source.get_pixel(28, 22).0, [255, 255, 255, 255]);

        let mut white = RgbaImage::from_pixel(60, 50, image::Rgba([255, 255, 255, 255]));
        strip_checkerboard_background(&mut white);
        assert!(white.pixels().all(|pixel| pixel[3] == 255));
    }
}
