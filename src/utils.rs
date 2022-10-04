//
// mtpng - a multithreaded parallel PNG encoder in Rust
// utils.rs - misc bits
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

use ::std::io;
use ::std::io::{Error, ErrorKind, Write};

pub type IoResult = io::Result<()>;

pub fn invalid_data(payload: &str) -> Error {
    Error::new(ErrorKind::InvalidData, payload)
}

pub fn invalid_input(payload: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, payload)
}

pub fn other(payload: &str) -> Error {
    Error::new(ErrorKind::Other, payload)
}

pub fn write_be32<W: Write>(w: &mut W, val: u32) -> IoResult {
    let bytes = [
        (val >> 24 & 0xff) as u8,
        (val >> 16 & 0xff) as u8,
        (val >> 8 & 0xff) as u8,
        (val & 0xff) as u8,
    ];
    w.write_all(&bytes)
}

pub fn write_byte<W: Write>(w: &mut W, val: u8) -> IoResult {
    let bytes = [val];
    w.write_all(&bytes)
}
