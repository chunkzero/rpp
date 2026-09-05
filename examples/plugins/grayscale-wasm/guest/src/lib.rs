wit_bindgen::generate!({
    world: "grayscale",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn grayscale(input: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut decoder = png::Decoder::new(input.as_slice());
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder
            .read_info()
            .map_err(|error| format!("decode: {error}"))?;
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut pixels)
            .map_err(|error| format!("decode: {error}"))?;
        pixels.truncate(info.buffer_size());

        let (channels, has_alpha) = match info.color_type {
            png::ColorType::Rgb => (3, false),
            png::ColorType::Rgba => (4, true),
            png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha => return Ok(input),
            other => return Err(format!("unsupported color type {other:?}")),
        };
        if info.bit_depth != png::BitDepth::Eight {
            return Err(format!("unsupported bit depth {:?}", info.bit_depth));
        }

        let out_channels = if has_alpha { 2 } else { 1 };
        let mut out = Vec::with_capacity(pixels.len() / channels * out_channels);
        for pixel in pixels.chunks_exact(channels) {
            let luma =
                (pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000;
            out.push(luma as u8);
            if has_alpha {
                out.push(pixel[3]);
            }
        }

        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, info.width, info.height);
            encoder.set_color(if has_alpha {
                png::ColorType::GrayscaleAlpha
            } else {
                png::ColorType::Grayscale
            });
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|error| format!("encode: {error}"))?;
            writer
                .write_image_data(&out)
                .map_err(|error| format!("encode: {error}"))?;
        }
        Ok(encoded)
    }
}

export!(Component);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_rgb_transparency() {
        let mut input = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut input, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_trns(vec![0, 255, 0, 0, 0, 0]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        }
        let output = Component::grayscale(input).unwrap();
        let mut reader = png::Decoder::new(output.as_slice()).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
        assert_eq!(&pixels[..info.buffer_size()], &[76, 0, 149, 255]);
    }
}
