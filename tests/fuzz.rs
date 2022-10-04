use std::{borrow::Cow, ffi::OsStr, fs::File, io, panic::catch_unwind, path::PathBuf};

use mtpng::{
    encoder::{Encoder, Options},
    ColorType, CompressionLevel, Filter, Header, Mode, Strategy,
};
use rand::{prelude::StdRng, Rng};

fn png_files(path: &'static str, filter: bool) -> impl Iterator<Item = PathBuf> {
    walkdir::WalkDir::new(path)
        .into_iter()
        .map(std::result::Result::unwrap)
        .filter(|entry| entry.path().is_file())
        .filter(move |entry| !filter || entry.path().extension() == Some(OsStr::new("png")))
        .map(walkdir::DirEntry::into_path)
}

fn write_png(
    filename: &str,
    options: &Options,
    header: &Header,
    data: &[u8],
    palette: &Option<Cow<[u8]>>,
    transparency: &Option<Cow<[u8]>>,
) -> io::Result<()> {
    let writer = File::create(filename)?;

    let mut encoder = Encoder::new(writer, options);

    // Image data
    encoder.write_header(header)?;
    match palette {
        Some(v) => encoder.write_palette(v)?,
        None => {}
    }
    match transparency {
        Some(v) => encoder.write_transparency(v)?,
        None => {}
    }
    encoder.write_image_rows(data)?;
    encoder.finish()?;

    Ok(())
}

#[ignore]
#[test]
pub fn fuzz() {
    println!("started fuzzing");
    let files: Vec<PathBuf> = png_files("pngsuite", true).collect();

    let seed = [
        92, 1, 0, 130, 211, 8, 21, 70, 74, 4, 9, 5, 0, 23, 0, 3, 20, 25, 6, 5, 229, 30, 0, 34, 218,
        0, 40, 7, 5, 2, 7, 0,
    ];

    let mut random: StdRng = rand::SeedableRng::from_seed(seed);

    let start_index = 0;
    let compression_level = [
        CompressionLevel::Default,
        CompressionLevel::Fast,
        CompressionLevel::High,
    ];
    let filter_mode = [
        Mode::Adaptive,
        Mode::Fixed(Filter::None),
        Mode::Fixed(Filter::Average),
        Mode::Fixed(Filter::Paeth),
        Mode::Fixed(Filter::Sub),
        Mode::Fixed(Filter::Up),
    ];
    let strategy = [
        Mode::Adaptive,
        Mode::Fixed(Strategy::Default),
        Mode::Fixed(Strategy::Filtered),
        Mode::Fixed(Strategy::HuffmanOnly),
        Mode::Fixed(Strategy::RLE),
    ];

    for fuzz_index in 0..1024_u64 * 2048 * 4 {
        let file_1_name = &files[random.gen_range(0..files.len())];
        let mutation_point = random.gen::<f32>().powi(3);
        let mutation = random.gen::<u8>();

        if fuzz_index >= start_index {
            use png::Decoder;
            use png::Transformations;
            let result = catch_unwind(move || {
                let mut options = Options::new();
                options
                    .set_compression_level(
                        compression_level[mutation as usize % compression_level.len()],
                    )
                    .unwrap();
                options
                    .set_filter_mode(filter_mode[mutation as usize % filter_mode.len()])
                    .unwrap();
                options
                    .set_strategy_mode(strategy[mutation as usize % strategy.len()])
                    .unwrap();

                let mut decoder = Decoder::new(File::open(file_1_name).unwrap());
                decoder.set_transformations(Transformations::IDENTITY);

                let mut reader = decoder.read_info().unwrap();
                let mut data = vec![0u8; reader.output_buffer_size()];
                let info = reader.next_frame(&mut data).unwrap();

                let mut header = Header::new();
                header.set_size(info.width, info.height).unwrap();
                header
                    .set_color(
                        ColorType::try_from(info.color_type as u8).unwrap(),
                        info.bit_depth as u8,
                    )
                    .unwrap();

                let palette = reader.info().palette.clone();
                let transparency = reader.info().trns.clone();

                let index = ((mutation_point * data.len() as f32) as usize + 4) % data.len();
                data[index] = mutation;

                let seed = seed
                    .iter()
                    .map(|num| num.to_string())
                    .collect::<Vec<String>>()
                    .join("-");

                write_png(
                    &format!("tests/images/fuzzed/fuzz_{}_{}.png", fuzz_index, seed),
                    &options,
                    &header,
                    &data,
                    &palette,
                    &transparency,
                )
                .unwrap()
            });
            if let Err(e) = result {
                println!(" PANIC! [{:?}]", e);
            }
        }
    }
}
