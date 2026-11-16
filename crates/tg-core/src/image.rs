use anyhow::{Context, Result, ensure};
use jpeg_encoder::{ColorType, Encoder};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
const MAX_SIDE: usize = 4096;
const SAVE_QUALITY: u8 = 95;

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
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, SAVE_QUALITY)
        .encode(&image.bgra, u16::try_from(image.width)?, u16::try_from(image.height)?, ColorType::Bgra)
        .map_err(|e| anyhow::anyhow!("JPEG encode failed: {e:?}"))?;
    Ok(jpeg)
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
}
