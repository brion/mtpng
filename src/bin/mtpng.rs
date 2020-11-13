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
use std::io::{Error, ErrorKind};
use std::time::SystemTime;

// CLI options
use clap::{Arg, App, ArgMatches};

use rayon::{ThreadPool, ThreadPoolBuilder};

use mtpng::{ColorType, CompressionLevel, Header};
use mtpng::Mode::{Adaptive, Fixed};
use mtpng::encoder::{Encoder, Options};
use mtpng::Strategy;
use mtpng::Filter;

pub fn err(payload: &str) -> Error
{
    Error::new(ErrorKind::Other, payload)
}

fn read_png(filename: &str)
    -> io::Result<(Header, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>)>
{
    use png::Decoder;
    use png::Transformations;

    let mut decoder = Decoder::new(File::open(filename)?);
    decoder.set_transformations(Transformations::IDENTITY);

    let (info, mut reader) = decoder.read_info()?;

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
             args: &ArgMatches,
             filename: &str,
             header: &Header,
             data: &[u8],
             palette: &Option<Vec<u8>>,
             transparency: &Option<Vec<u8>>)
   -> io::Result<()>
{
    let writer = File::create(filename)?;
    let mut options = Options::new();

    // Encoding options
    options.set_thread_pool(pool)?;

    match args.value_of("chunk-size") {
        None    => {},
        Some(s) => {
            let n = s.parse::<usize>().map_err(|_e| err("Invalid chunk size"))?;
            options.set_chunk_size(n)?;
        },
    }

    match args.value_of("filter") {
        None             => {},
        Some("adaptive") => options.set_filter_mode(Adaptive)?,
        Some("none")     => options.set_filter_mode(Fixed(Filter::None))?,
        Some("up")       => options.set_filter_mode(Fixed(Filter::Up))?,
        Some("sub")      => options.set_filter_mode(Fixed(Filter::Sub))?,
        Some("average")  => options.set_filter_mode(Fixed(Filter::Average))?,
        Some("paeth")    => options.set_filter_mode(Fixed(Filter::Paeth))?,
        _                => return Err(err("Unsupported filter type")),
    }

    match args.value_of("level") {
        None            => {},
        Some("default") => options.set_compression_level(CompressionLevel::Default)?,
        Some("1")       => options.set_compression_level(CompressionLevel::Fast)?,
        Some("9")       => options.set_compression_level(CompressionLevel::High)?,
        _               => return Err(err("Unsupported compression level (try default, 1, or 9)")),
    }

    match args.value_of("strategy") {
        None             => {},
        Some("auto")     => options.set_strategy_mode(Adaptive)?,
        Some("default")  => options.set_strategy_mode(Fixed(Strategy::Default))?,
        Some("filtered") => options.set_strategy_mode(Fixed(Strategy::Filtered))?,
        Some("huffman")  => options.set_strategy_mode(Fixed(Strategy::HuffmanOnly))?,
        Some("rle")      => options.set_strategy_mode(Fixed(Strategy::RLE))?,
        Some("fixed")    => options.set_strategy_mode(Fixed(Strategy::Fixed))?,
        _                => return Err(err("Invalid compression strategy mode")),
    }

    match args.value_of("streaming") {
        None        => {},
        Some("yes") => options.set_streaming(true)?,
        Some("no")  => options.set_streaming(false)?,
        _           => return Err(err("Invalid streaming mode, try yes or no."))
    }

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
    encoder.finish()?;

    Ok(())
}

fn doit(args: ArgMatches) -> io::Result<()> {
    let threads = match args.value_of("threads") {
        None    => 0, // Means default
        Some(s) => {
            s.parse::<usize>().map_err(|_e| err("invalid threads"))?
        },
    };

    let pool = ThreadPoolBuilder::new().num_threads(threads)
                                       .build()
                                       .map_err(|e| err(&e.to_string()))?;
    eprintln!("Using {} threads", pool.current_num_threads());

    let reps = match args.value_of("repeat") {
        Some(s) => {
            s.parse::<usize>().map_err(|_e| err("invalid repeat"))?
        },
        None => 1,
    };

    // input and output are guaranteed to be present
    let infile = args.value_of("input").unwrap();
    let outfile = args.value_of("output").unwrap();

    println!("{} -> {}", infile, outfile);
    let (header, data, palette, transparency) = read_png(&infile)?;

    for _i in 0 .. reps {
        let start_time =  SystemTime::now();
        write_png(&pool, &args, &outfile, &header, &data, &palette, &transparency)?;
        let delta = start_time.elapsed().unwrap();

        println!("Done in {} ms", delta.as_millis());
    }

    Ok(())
}

pub fn main() {
    let matches = App::new("mtpng parallel PNG encoder")
        .version("0.1.0")
        .author("Brion Vibber <brion@pobox.com>")
        .about("Re-encodes PNG images using multiple CPU cores to exercise the mtpng library.")
        .arg(Arg::with_name("chunk-size")
            .long("chunk-size")
            .value_name("bytes")
            .help("Divide image into chunks of at least this given size.")
            .takes_value(true))
        .arg(Arg::with_name("filter")
            .long("filter")
            .value_name("filter")
            .help("Set a fixed filter: one of none, sub, up, average, or paeth."))
        .arg(Arg::with_name("level")
            .long("level")
            .value_name("level")
            .help("Set deflate compression level, from 1-9."))
        .arg(Arg::with_name("strategy")
            .long("strategy")
            .value_name("strategy")
            .help("Deflate strategy: one of filtered, huffman, rle, or fixed."))
        .arg(Arg::with_name("streaming")
            .long("streaming")
            .value_name("streaming")
            .help("Use streaming output mode; trades off file size for lower latency and memory usage"))
        .arg(Arg::with_name("threads")
            .long("threads")
            .value_name("threads")
            .help("Override default number of threads."))
        .arg(Arg::with_name("repeat")
            .long("repeat")
            .value_name("n")
            .help("Run conversion n times, as load benchmarking helper."))
        .arg(Arg::with_name("input")
            .help("Input filename, must be another PNG.")
            .required(true)
            .index(1))
        .arg(Arg::with_name("output")
            .help("Output filename.")
            .required(true)
            .index(2))
        .get_matches();

    match doit(matches) {
        Ok(()) => {},
        Err(e) => eprintln!("Error: {}", e),
    }
}