//
// mtpng - a multithreaded parallel PNG encoder in Rust
// deflate.rs - wrapper for libz_sys suitable for making chunked deflate streams
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

use std::io::Write;
use std::io;

use std::convert::TryFrom;

use std::os::raw::*;

use miniz_oxide::deflate::core::{create_comp_flags_from_zip_params, TDEFLFlush, TDEFLStatus};

use super::utils::*;

pub struct Options {
    level: i32,
    window_bits: i32,
    strategy: i32,
}

#[repr(i32)]
#[derive(Copy, Clone)]
pub enum Strategy {
    Default = 0,
    Filtered = 1,
    HuffmanOnly = 2,
    RLE = 3,
    Fixed = 4,
}

impl TryFrom<u8> for Strategy {
    type Error = io::Error;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Strategy::Default),
            1 => Ok(Strategy::Filtered),
            2 => Ok(Strategy::HuffmanOnly),
            3 => Ok(Strategy::RLE),
            4 => Ok(Strategy::Fixed),
            _ => Err(invalid_input("Invalid strategy constant")),
        }
    }
}

impl Options {
    pub fn new() -> Options {
        Options {
            level: -1,
            window_bits: 15,
            strategy: 0,
        }
    }

    //
    // Compression level, 1 (fast) - 9 (high)
    //
    pub fn set_level(&mut self, level: i32) {
        self.level = level as c_int;
    }

    //
    // Default is 15 (32 KiB)
    // Set negative value for raw stream (no header/checksum)
    //
    pub fn set_window_bits(&mut self, bits: i32) {
        self.window_bits = bits as c_int;
    }

    pub fn set_strategy(&mut self, strategy: Strategy) {
        self.strategy = strategy as c_int;
    }
}

#[derive(Copy, Clone)]
pub enum Flush {
    // Only SyncFlush and Finish are used internally.

    //NoFlush = Z_NO_FLUSH as isize,
    //PartialFlush = Z_PARTIAL_FLUSH as isize,
    SyncFlush,
    //FullFlush = Z_FULL_FLUSH as isize,
    Finish,
}

pub struct Deflate<W: Write> {
    output: W,
    initialized: bool,
    stream: miniz_oxide::deflate::core::CompressorOxide,
}

impl<W: Write> Deflate<W> {
    pub fn new(options: Options, w: W) -> Deflate<W> {
        let flags =
            create_comp_flags_from_zip_params(options.level, options.window_bits, options.strategy);
        Deflate {
            output: w,
            initialized: false,
            stream: miniz_oxide::deflate::core::CompressorOxide::new(flags),
        }
    }

    pub fn init(&mut self) -> IoResult {
        if self.initialized {
            Ok(())
        } else {
            self.stream.reset();
            self.initialized = true;
            Ok(())
        }
    }

    pub fn set_dictionary(&mut self, _dict: &[u8]) -> IoResult {
        self.init()?;
        Ok(())
    }

    fn deflate(&mut self, data: &[u8], flush: Flush) -> IoResult {
        self.init()?;
        let f = match flush {
            Flush::SyncFlush => TDEFLFlush::Sync,
            Flush::Finish => TDEFLFlush::Finish,
        };
        let mut output = vec![0; ::core::cmp::max(data.len() / 2, 2)];

        let mut in_pos = 0;
        let mut out_pos = 0;
        loop {
            let (status, bytes_in, bytes_out) = miniz_oxide::deflate::core::compress(
                &mut self.stream,
                &data[in_pos..],
                &mut output[out_pos..],
                f,
            );

            out_pos += bytes_out;
            in_pos += bytes_in;

            match status {
                TDEFLStatus::Done => {
                    output.truncate(out_pos);
                    break;
                }
                TDEFLStatus::Okay => {
                    // We have too much space allocated.
                    if out_pos < output.len() {
                        output.truncate(out_pos);
                        break;
                    } else if bytes_in > 0 {
                        // We need more space, so resize the vector.

                        if output.len().saturating_sub(out_pos) < 30 {
                            output.resize(output.len() * 2, 0)
                        }
                        continue;
                    }
                    // This shouldn't be reached, but we break to avoid infinite loops.
                    break;
                }
                // Not supposed to happen unless there is a bug.
                _ => {
                    return Err(invalid_data("Bug compressing data."))
                }
            }
        }
        self.output.write_all(&output)?;
        Ok(())
    }

    pub fn write(&mut self, data: &[u8], flush: Flush) -> IoResult {
        self.init()?;
        self.deflate(data, flush)
    }

    //
    // Deallocate the zlib state and return the writer.
    //
    pub fn finish(self) -> io::Result<W> {
        Ok(self.output)
    }
}
