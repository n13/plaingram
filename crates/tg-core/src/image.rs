use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder as _, ImageReader, Rgb, RgbImage};
use jpeg_encoder::{ColorType, Encoder};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::model::Upload;

const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
const MAX_SIDE: usize = 4096;
const JPEG_QUALITY: u8 = 95;
/// The long side of Telegram's largest photo size; bigger photos are scaled down to it before upload.
const UPLOAD_MAX_SIDE: u32 = 2560;

#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Decodes JPEG only, with zune-jpeg built without SIMD so the crate is `forbid(unsafe_code)`.
pub fn decode_jpeg(bytes: &[u8]) -> Result<DecodedImage> {
    ensure!(bytes.starts_with(&JPEG_MAGIC), "image is not a JPEG");
    let options = DecoderOptions::default()
        .set_max_width(MAX_SIDE)
        .set_max_height(MAX_SIDE)
        .set_strict_mode(true)
        .jpeg_set_out_colorspace(ColorSpace::BGRA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let bgra = decoder.decode().map_err(|e| anyhow::anyhow!("JPEG decode failed: {e:?}"))?;
    let info = decoder.info().context("JPEG decoder returned no image info")?;
    Ok(DecodedImage { width: info.width.into(), height: info.height.into(), bgra })
}

/// Encodes decoded pixels as a new JPEG, so a saved photo holds no byte from its sender. jpeg-encoder is built
/// without SIMD, so the crate is `forbid(unsafe_code)`.
pub(crate) fn encode_jpeg(image: &DecodedImage) -> Result<Vec<u8>> {
    encode(&image.bgra, image.width, image.height, ColorType::Bgra)
}

fn encode(pixels: &[u8], width: u32, height: u32, color: ColorType) -> Result<Vec<u8>> {
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, JPEG_QUALITY)
        .encode(pixels, u16::try_from(width)?, u16::try_from(height)?, color)
        .map_err(|e| anyhow::anyhow!("JPEG encode failed: {e:?}"))?;
    Ok(jpeg)
}

/// Writes the user's `photo` to `out` as Telegram gets it, and returns its size: turned upright as the camera
/// recorded, at most [`UPLOAD_MAX_SIDE`], transparency on white, and a new JPEG, so no metadata such as location or
/// camera leaves the computer.
pub(crate) fn prepare_upload(photo: &Upload, out: &Path) -> Result<(u32, u32)> {
    let mut image = match photo {
        Upload::File(file) => decode_file(file, out)?,
        Upload::Pasted(bytes) => decode_upright(bytes).context("cannot read the pasted image as a photo")?,
    };
    if image.width().max(image.height()) > UPLOAD_MAX_SIDE {
        image = image.resize(UPLOAD_MAX_SIDE, UPLOAD_MAX_SIDE, FilterType::CatmullRom);
    }
    let rgb = on_white(image);
    fs::write(out, encode(rgb.as_raw(), rgb.width(), rgb.height(), ColorType::Rgb)?)?;
    Ok(rgb.dimensions())
}

/// Decodes with the image crate, or on macOS, for formats it lacks such as HEIC, after `sips` converts to JPEG at `out`.
#[cfg_attr(not(target_os = "macos"), expect(unused_variables, reason = "only macOS converts through a file"))]
fn decode_file(file: &Path, out: &Path) -> Result<DynamicImage> {
    let unreadable = || format!("cannot read {} as a photo", file.display());
    let decoded = decode_upright(&fs::read(file)?);
    #[cfg(target_os = "macos")]
    let decoded = match decoded {
        Err(image::ImageError::Unsupported(_)) => {
            let converted = std::process::Command::new("/usr/bin/sips")
                .args(["-s", "format", "jpeg"])
                .arg(file)
                .arg("--out")
                .arg(out)
                .output()?;
            ensure!(
                converted.status.success(),
                "{}: {}",
                unreadable(),
                String::from_utf8_lossy(&converted.stderr).trim()
            );
            decode_upright(&fs::read(out)?)
        }
        decoded => decoded,
    };
    decoded.with_context(unreadable)
}

/// Applies the orientation the camera recorded, since re-encoding drops the metadata that holds it.
fn decode_upright(bytes: &[u8]) -> image::ImageResult<DynamicImage> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn on_white(image: DynamicImage) -> RgbImage {
    let rgba = image.into_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend = |c: u8| ((u16::from(c) * u16::from(a) + 255 * u16::from(255 - a)) / 255) as u8;
        Rgb([blend(r), blend(g), blend(b)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u16, height: u16, rgb: [u8; 3]) -> Vec<u8> {
        let pixels: Vec<u8> = rgb.repeat(usize::from(width) * usize::from(height));
        let mut out = Vec::new();
        Encoder::new(&mut out, 95).encode(&pixels, width, height, ColorType::Rgb).unwrap();
        out
    }

    #[test]
    fn decodes_to_bgra() {
        let image = decode_jpeg(&jpeg(16, 8, [250, 10, 10])).unwrap();
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(image.bgra.len(), 16 * 8 * 4);
        let [b, g, r, a] = image.bgra[..4] else { unreachable!() };
        assert!(r > 230 && g < 30 && b < 30 && a == 255, "{:?}", &image.bgra[..4]);
    }

    #[test]
    fn re_encoding_keeps_the_picture_and_drops_everything_else() {
        const PLANTED: &[u8] = b"planted by the sender";
        let mut original = jpeg(16, 8, [250, 10, 10]);
        let comment = [&[0xFF, 0xFE, 0, PLANTED.len() as u8 + 2], PLANTED].concat();
        original.splice(2..2, comment);
        let saved = encode_jpeg(&decode_jpeg(&original).unwrap()).unwrap();
        assert!(!saved.windows(PLANTED.len()).any(|w| w == PLANTED), "only pixels survive re-encoding");
        let image = decode_jpeg(&saved).unwrap();
        assert_eq!((image.width, image.height), (16, 8));
        let [b, g, r, _] = image.bgra[..4] else { unreachable!() };
        assert!(r > 230 && g < 30 && b < 30, "{:?}", &image.bgra[..4]);
    }

    #[test]
    fn rejects_non_jpeg() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert!(decode_jpeg(png).is_err());
        assert!(decode_jpeg(b"RIFF\0\0\0\0WEBPVP8 ").is_err());
        assert!(decode_jpeg(&[]).is_err());
    }

    #[test]
    fn rejects_truncated_jpeg() {
        let bytes = jpeg(64, 64, [0, 0, 255]);
        assert!(decode_jpeg(&bytes[..bytes.len() / 3]).is_err());
    }

    #[test]
    fn rejects_oversized_jpeg() {
        assert!(decode_jpeg(&jpeg(MAX_SIDE as u16 + 1, 1, [0, 0, 0])).is_err());
    }

    /// 40x20, red on the left and blue on the right, tagged the way a phone tags a portrait photo: rotate 90° clockwise.
    fn portrait_jpeg() -> Vec<u8> {
        const ROTATE_CLOCKWISE: u8 = 6;
        let pixels: Vec<u8> =
            (0..40 * 20).flat_map(|i| if i % 40 < 20 { [250, 10, 10] } else { [10, 10, 250] }).collect();
        let exif = [
            b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0".as_slice(),
            &[ROTATE_CLOCKWISE, 0, 0],
            &[0; 4],
        ];
        let mut out = Vec::new();
        let mut encoder = Encoder::new(&mut out, 95);
        encoder.add_app_segment(1, exif.concat()).unwrap();
        encoder.encode(&pixels, 40, 20, ColorType::Rgb).unwrap();
        out
    }

    fn test_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("plaingram-upload-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Prepares `bytes` as an upload from a file named `name`, and returns the size and the JPEG sent.
    fn upload(name: &str, bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
        let (file, out) = (test_dir().join(name), test_dir().join(format!("{name}.out.jpg")));
        fs::write(&file, bytes).unwrap();
        let size = prepare_upload(&Upload::File(file), &out).unwrap();
        (size, fs::read(out).unwrap())
    }

    /// Red on top, blue below, and no metadata left to rotate it again.
    fn assert_upright(size: (u32, u32), jpeg: &[u8]) {
        assert_eq!(size, (20, 40), "turned to portrait");
        assert!(!jpeg.windows(4).any(|w| w == b"Exif"), "metadata is gone");
        let image = decode_jpeg(jpeg).unwrap();
        let red = |row: usize| image.bgra[row * 20 * 4 + 10 * 4 + 2] > 200;
        assert!(red(5) && !red(35), "the left side, red, is now on top");
    }

    #[test]
    fn uploads_are_turned_upright_before_their_metadata_is_dropped() {
        let (size, jpeg) = upload("portrait.jpg", &portrait_jpeg());
        assert_upright(size, &jpeg);
    }

    #[test]
    fn pasted_images_upload_like_files() {
        let out = test_dir().join("pasted.out.jpg");
        let size = prepare_upload(&Upload::Pasted(portrait_jpeg()), &out).unwrap();
        assert_upright(size, &fs::read(&out).unwrap());
        assert!(prepare_upload(&Upload::Pasted(b"<svg/>".to_vec()), &out).is_err(), "only formats it decodes");
    }

    #[test]
    fn uploads_shrink_to_telegrams_largest_size_and_never_grow() {
        assert_eq!(upload("wide.jpg", &jpeg(5120, 100, [0, 0, 0])).0, (2560, 50));
        assert_eq!(upload("small.jpg", &jpeg(300, 200, [0, 0, 0])).0, (300, 200));
    }

    #[test]
    fn transparent_png_uploads_turn_white() {
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(image::RgbaImage::new(8, 8))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode_jpeg(&upload("clear.png", &png).1).unwrap();
        assert!(image.bgra[..3].iter().all(|&c| c > 245), "{:?}", &image.bgra[..4]);
    }

    /// iPhone photos are HEIC, which only macOS decodes; sips writes HEIC too, keeping the rotation tag.
    #[cfg(target_os = "macos")]
    #[test]
    fn heic_uploads_are_converted_by_macos_and_turned_upright() {
        let (source, heic) = (test_dir().join("portrait-source.jpg"), test_dir().join("portrait.heic"));
        fs::write(&source, portrait_jpeg()).unwrap();
        let status = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "heic"])
            .arg(&source)
            .arg("--out")
            .arg(&heic)
            .output()
            .unwrap();
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        let (size, jpeg) = upload("portrait.heic", &fs::read(&heic).unwrap());
        assert_upright(size, &jpeg);
    }
}
