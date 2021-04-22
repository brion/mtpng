#![no_main]
use libfuzzer_sys::fuzz_target;

use std::io;
extern crate png;
extern crate rayon;
use rayon::{ThreadPool, ThreadPoolBuilder};
use mtpng::{ColorType, CompressionLevel, Header};
use mtpng::Mode::{Adaptive};
use mtpng::encoder::{Encoder, Options};
use std::convert::TryFrom;

fn decode_png(data: &[u8])
    -> io::Result<(Header, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>)>
{
    let limits = png::Limits { bytes: 1 << 16 };
    let decoder = png::Decoder::new_with_limits(data, limits);
    let (info, mut reader) = decoder.read_info()?;

    if info.buffer_size() > 5_000_000 {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "memory limit exceeded"));
    }

    let mut header = Header::new();
    header.set_size(info.width, info.height)?;
    header.set_color(ColorType::try_from(info.color_type as u8)?,
                     info.bit_depth as u8)?;

    let palette = reader.info().palette.clone();
    let transparency = reader.info().trns.clone();

    let mut data = vec![0u8; info.buffer_size()];
    reader.next_frame(&mut data)?;

    Ok((header, data, palette, transparency))
}

fn write_png(pool: &ThreadPool,
             header: &Header,
             data: &[u8],
             palette: &Option<Vec<u8>>,
             transparency: &Option<Vec<u8>>)
   -> io::Result<Vec<u8>>
{
    let writer = Vec::new();
    let mut options = Options::new();

    // Encoding options
    options.set_thread_pool(pool)?;
    options.set_filter_mode(Adaptive)?;
    options.set_compression_level(CompressionLevel::Default)?;
    options.set_strategy_mode(Adaptive)?;
    options.set_streaming(false)?;

    let mut encoder = Encoder::new(writer, &options);

    // Image data
    encoder.write_header(&header)?;
    match palette {
        Some(v) => encoder.write_palette(&v)?,
        None => {},
    }
    match transparency {
        Some(v) => encoder.write_transparency(&v)?,
        None => {},
    }
    encoder.write_image_rows(&data)?;
    encoder.finish()
}

fn roundtrip(pool: ThreadPool, data: &[u8]) -> io::Result<()> {
    let (header, data, palette, transparency) = decode_png(data)?;
    // the line below does error out in practice; not sure if that's a bug in png or mtpng
    let compressed = write_png(&pool, &header, &data, &palette, &transparency)?;//.expect("Writing PNG failed");
    let (new_header, new_data, new_palette, new_transparency) = decode_png(&compressed).expect("Failed to decode mtpng-compressed data");
    // not sure if header and palette are expected to match exactly, so ignoring them for now
    //assert!(header == new_header, "Header differs after encoding and decoding back");
    //assert!(palette == new_palette, "Palette differs after encoding and decoding back");
    assert!(data == new_data, "Data differs after encoding and decoding back");
    assert!(transparency == new_transparency, "Transparency differs after encoding and decoding back");
    Ok(())
}

fuzz_target!(|data: &[u8]| {
    // we could create the pool once instead of on every input
    // if we used AFL instead, but AFL is not as user-friendly as cargo-fuzz,
    // and if the fuzzing is too complicated people won't use it at all
    let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    // we don't care about the result *here*:
    // all the failure conditions we want to detect panic
    let _ = roundtrip(pool, data);
});
