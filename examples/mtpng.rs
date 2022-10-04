//
// mtpng - a multithreaded parallel PNG encoder in Rust
// mtpng.rs - CLI utility for testing and Rust API example
//
// Copyright (c) 2018 Brion Vibber
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//

use std::convert::TryFrom;
use std::fs::File;
use std::io;
use std::io::{Error, ErrorKind, Write};

// CLI options
use clap::{App, Arg, ArgMatches};

// For reading an existing file
use rayon::{ThreadPool, ThreadPoolBuilder};

// For timing!
use time::OffsetDateTime;

// Hey that's us!
extern crate mtpng;
use mtpng::encoder::{Encoder, Options};
use mtpng::Filter;
use mtpng::Mode::{Adaptive, Fixed};
use mtpng::Strategy;
use mtpng::{ColorType, CompressionLevel, Header};

pub fn err(payload: &str) -> Error {
    Error::new(ErrorKind::Other, payload)
}

fn expand(src: &[u8]) -> io::Result<Vec<u8>>
{
    let mut v = Vec::new();
    v.write_all(src)?;
    Ok(v)
}

struct Image {
    header: Header,
    data: Vec<u8>,
    palette: Option<Vec<u8>>,
    transparency: Option<Vec<u8>>,
}

fn read_png(filename: &str)
    -> io::Result<Image>
{
    use png::Decoder;
    use png::Transformations;

    let mut decoder = Decoder::new(File::open(filename)?);
    decoder.set_transformations(Transformations::IDENTITY);

    let mut reader = decoder.read_info()?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data)?;

    let mut header = Header::new();
    header.set_size(info.width, info.height)?;
    header.set_color(
        ColorType::try_from(info.color_type as u8)?,
        info.bit_depth as u8,
    )?;

    let info = reader.info();
    // Very ugly hack to run from the Cow and ownership
    let palette = info
        .palette
        .clone()
        .map(|x| x.iter().cloned().collect::<Vec<u8>>());
    let transparency = info
        .trns
        .clone()
        .map(|x| x.iter().cloned().collect::<Vec<u8>>());

    Ok(Image {
        header,
        data,
        palette,
        transparency
    })
}

fn write_png(pool: &ThreadPool,
             args: &ArgMatches,
             filename: &str,
             image: &Image)
   -> io::Result<()>
{
    let writer = File::create(filename)?;
    let mut options = Options::new();

    // Encoding options
    options.set_thread_pool(pool)?;

    match args.value_of("chunk-size") {
        None => {}
        Some(s) => {
            let n = s.parse::<usize>().map_err(|_e| err("Invalid chunk size"))?;
            options.set_chunk_size(n)?;
        }
    }

    match args.value_of("filter") {
        None => {}
        Some("adaptive") => options.set_filter_mode(Adaptive)?,
        Some("none") => options.set_filter_mode(Fixed(Filter::None))?,
        Some("up") => options.set_filter_mode(Fixed(Filter::Up))?,
        Some("sub") => options.set_filter_mode(Fixed(Filter::Sub))?,
        Some("average") => options.set_filter_mode(Fixed(Filter::Average))?,
        Some("paeth") => options.set_filter_mode(Fixed(Filter::Paeth))?,
        _ => return Err(err("Unsupported filter type")),
    }

    match args.value_of("level") {
        None => {}
        Some("default") => options.set_compression_level(CompressionLevel::Default)?,
        Some("1") => options.set_compression_level(CompressionLevel::Fast)?,
        Some("9") => options.set_compression_level(CompressionLevel::High)?,
        _ => return Err(err("Unsupported compression level (try default, 1, or 9)")),
    }

    match args.value_of("strategy") {
        None => {}
        Some("auto") => options.set_strategy_mode(Adaptive)?,
        Some("default") => options.set_strategy_mode(Fixed(Strategy::Default))?,
        Some("filtered") => options.set_strategy_mode(Fixed(Strategy::Filtered))?,
        Some("huffman") => options.set_strategy_mode(Fixed(Strategy::HuffmanOnly))?,
        Some("rle") => options.set_strategy_mode(Fixed(Strategy::RLE))?,
        Some("fixed") => options.set_strategy_mode(Fixed(Strategy::Fixed))?,
        _ => return Err(err("Invalid compression strategy mode")),
    }

    match args.value_of("streaming") {
        None => {}
        Some("yes") => options.set_streaming(true)?,
        Some("no") => options.set_streaming(false)?,
        _ => return Err(err("Invalid streaming mode, try yes or no.")),
    }

    let mut encoder = Encoder::new(writer, &options);

    // Image data
    encoder.write_header(&image.header)?;
    match &image.palette {
        Some(v) => encoder.write_palette(v)?,
        None => {},
    }
    match &image.transparency {
        Some(v) => encoder.write_transparency(v)?,
        None => {},
    }
    encoder.write_image_rows(&image.data)?;
    encoder.finish()?;

    Ok(())
}

fn doit(args: ArgMatches) -> io::Result<()> {
    let threads = match args.value_of("threads") {
        None => 0, // Means default
        Some(s) => s.parse::<usize>().map_err(|_e| err("invalid threads"))?,
    };

    let pool = ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| err(&e.to_string()))?;
    eprintln!("Using {} threads", pool.current_num_threads());

    let reps = match args.value_of("repeat") {
        Some(s) => s.parse::<usize>().map_err(|_e| err("invalid repeat"))?,
        None => 1,
    };

    // input and output are guaranteed to be present
    let infile = args.value_of("input").unwrap();
    let outfile = args.value_of("output").unwrap();

    println!("{} -> {}", infile, outfile);
    let image = read_png(infile)?;

    for _i in 0..reps {
        let start_time = OffsetDateTime::now_utc();
        write_png(&pool, &args, outfile, &image)?;
        let delta = OffsetDateTime::now_utc() - start_time;

        println!("Done in {} ms", (delta.as_seconds_f64() * 1000.0).round());
    }

    Ok(())
}

pub fn main() {
    let matches = clap::Command::new("mtpng parallel PNG encoder")
        .version("0.4.0")
        .author("Brion Vibber <brion@pobox.com>")
        .about("Re-encodes PNG images using multiple CPU cores to exercise the mtpng library.")
        .arg(Arg::new("chunk-size")
            .long("chunk-size")
            .value_name("bytes")
            .help("Divide image into chunks of at least this given size.")
            .takes_value(true))
        .arg(Arg::new("filter")
            .long("filter")
            .value_name("filter")
            .help("Set a fixed filter: one of none, sub, up, average, or paeth."))
        .arg(Arg::new("level")
            .long("level")
            .value_name("level")
            .help("Set deflate compression level, from 1-9."))
        .arg(Arg::new("strategy")
            .long("strategy")
            .value_name("strategy")
            .help("Deflate strategy: one of filtered, huffman, rle, or fixed."))
        .arg(Arg::new("streaming")
            .long("streaming")
            .value_name("streaming")
            .help("Use streaming output mode; trades off file size for lower latency and memory usage"))
        .arg(Arg::new("threads")
            .long("threads")
            .value_name("threads")
            .help("Override default number of threads."))
        .arg(Arg::new("repeat")
            .long("repeat")
            .value_name("n")
            .help("Run conversion n times, as load benchmarking helper."))
        .arg(Arg::new("input")
            .help("Input filename, must be another PNG.")
            .required(true)
            .index(1))
        .arg(Arg::new("output")
            .help("Output filename.")
            .required(true)
            .index(2))
        .get_matches();

    match doit(matches) {
        Ok(()) => {}
        Err(e) => eprintln!("Error: {}", e),
    }
}
