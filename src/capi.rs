//
// mtpng - a multithreaded parallel PNG encoder in Rust
// capi.rs - C API implementation
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

use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;

use std::convert::TryFrom;

use std::io;
use std::io::Write;

use std::ptr;

use std::ffi::CStr;
use std::os::raw::c_char;

use libc::{c_int, c_void, size_t};

use super::ColorType;
use super::CompressionLevel;
use super::Header;
use super::Mode::{Adaptive, Fixed};
use super::Strategy;

use super::encoder::Encoder;
use super::encoder::Options;

use super::filter::Filter;

use super::utils::invalid_input;
use super::utils::other;

#[repr(C)]
pub enum CResult {
    Ok = 0,
    Err = 1,
}

impl From<Result<(), io::Error>> for CResult {
    fn from(result: Result<(), io::Error>) -> CResult {
        match result {
            Ok(()) => CResult::Ok,
            Err(_) => CResult::Err,
        }
    }
}

/*
pub type CReadFunc = unsafe extern "C"
    fn(*const c_void, *mut u8, size_t) -> size_t;
*/

pub type CWriteFunc = unsafe extern "C" fn(*const c_void, *const u8, size_t) -> size_t;

pub type CFlushFunc = unsafe extern "C" fn(*const c_void) -> bool;

/*

//
// Adapter for Read trait to use C callback.
//
pub struct CReader {
    read_func: CReadFunc,
    user_data: *mut c_void,
}

impl CReader {
    fn new(read_func: CReadFunc,
           user_data: *mut c_void)
    -> CReader
    {
        CReader {
            read_func: read_func,
            user_data: user_data,
        }
    }
}

impl Read for CReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let ret = unsafe {
            (self.read_func)(self.user_data,
                             &mut buf[0],
                             buf.len())
        };
        if ret == buf.len() {
            Ok(ret)
        } else {
            Err(other("mtpng read callback returned failure"))
        }
    }
}
*/

//
// Adapter for Write trait to use C callbacks.
//
pub struct CWriter {
    write_func: CWriteFunc,
    flush_func: CFlushFunc,
    user_data: *mut c_void,
}

impl CWriter {
    fn new(write_func: CWriteFunc, flush_func: CFlushFunc, user_data: *mut c_void) -> CWriter {
        CWriter {
            write_func: write_func,
            flush_func: flush_func,
            user_data: user_data,
        }
    }
}

impl Write for CWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let ret = unsafe { (self.write_func)(self.user_data, &buf[0], buf.len()) };
        if ret == buf.len() {
            Ok(ret)
        } else {
            Err(other("mtpng write callback returned failure"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let ret = unsafe { (self.flush_func)(self.user_data) };
        if ret {
            Ok(())
        } else {
            Err(other("mtpng flush callback returned failure"))
        }
    }
}

// Cheat on the lifetimes?
type CEncoder = Encoder<'static, CWriter>;

pub type PThreadPool = *mut ThreadPool;
pub type PEncoderOptions = *mut Options<'static>;
pub type PEncoder = *mut CEncoder;
pub type PHeader = *mut Header;

#[no_mangle]
pub unsafe extern "C" fn mtpng_threadpool_new(
    pp_pool: *mut PThreadPool,
    threads: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_pool.is_null() {
            return Err(invalid_input("pp_pool must not be null"));
        }
        if !(*pp_pool).is_null() {
            return Err(invalid_input("*pp_pool must be null"));
        }
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|err| other(&err.to_string()))?;
        *pp_pool = Box::into_raw(Box::new(pool));
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_threadpool_release(pp_pool: *mut PThreadPool) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_pool.is_null() {
            return Err(invalid_input("pp_pool must not be null"));
        }
        if (*pp_pool).is_null() {
            return Err(invalid_input("*pp_pool must not be null"));
        }
        drop(Box::from_raw(*pp_pool));
        *pp_pool = ptr::null_mut();
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_new(pp_options: *mut PEncoderOptions) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_options.is_null() {
            return Err(invalid_input("pp_options must not be null"));
        }
        if !(*pp_options).is_null() {
            return Err(invalid_input("*pp_options must be null"));
        }
        *pp_options = Box::into_raw(Box::new(Options::new()));
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_release(
    pp_options: *mut PEncoderOptions,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_options.is_null() {
            return Err(invalid_input("pp_header must not be null"));
        }
        if (*pp_options).is_null() {
            return Err(invalid_input("*pp_header must not be null"));
        }
        drop(Box::from_raw(*pp_options));
        *pp_options = ptr::null_mut();
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_set_thread_pool(
    p_options: PEncoderOptions,
    p_pool: PThreadPool,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_options.is_null() {
            return Err(invalid_input("p_options must not be null"));
        }
        (*p_options).set_thread_pool(&*p_pool)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_set_filter(
    p_options: PEncoderOptions,
    filter_mode: c_int,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_options.is_null() {
            return Err(invalid_input("p_options must not be null"));
        }
        if filter_mode > u8::max_value() as c_int {
            return Err(invalid_input("Invalid filter mode"));
        }
        let mode = if filter_mode < 0 {
            Adaptive
        } else {
            Fixed(Filter::try_from(filter_mode as u8)?)
        };
        (*p_options).set_filter_mode(mode)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_set_strategy(
    p_options: PEncoderOptions,
    strategy_mode: c_int,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_options.is_null() {
            return Err(invalid_input("p_options must not be null"));
        }
        if strategy_mode > u8::max_value() as c_int {
            return Err(invalid_input("Invalid strategy mode"));
        }
        let mode = if strategy_mode < 0 {
            Adaptive
        } else {
            Fixed(Strategy::try_from(strategy_mode as u8)?)
        };
        (*p_options).set_strategy_mode(mode)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_set_compression_level(
    p_options: PEncoderOptions,
    compression_level: c_int,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_options.is_null() {
            return Err(invalid_input("p_options must not be null"));
        }
        if compression_level < 0 || compression_level > 9 {
            return Err(invalid_input("Invalid compression level"));
        }
        let level = CompressionLevel::try_from(compression_level as u8)?;
        (*p_options).set_compression_level(level)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_options_set_chunk_size(
    p_options: PEncoderOptions,
    chunk_size: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_options.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        (*p_options).set_chunk_size(chunk_size)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_header_new(pp_header: *mut PHeader) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_header.is_null() {
            return Err(invalid_input("pp_header must not be null"));
        }
        if !(*pp_header).is_null() {
            return Err(invalid_input("*pp_header must be null"));
        }
        *pp_header = Box::into_raw(Box::new(Header::new()));
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_header_release(pp_header: *mut PHeader) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_header.is_null() {
            return Err(invalid_input("pp_header must not be null"));
        }
        if (*pp_header).is_null() {
            return Err(invalid_input("*pp_header must not be null"));
        }
        drop(Box::from_raw(*pp_header));
        *pp_header = ptr::null_mut();
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_header_set_size(
    p_header: PHeader,
    width: u32,
    height: u32,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_header.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        (*p_header).set_size(width, height)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_header_set_color(
    p_header: PHeader,
    color_type: c_int,
    depth: u8,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_header.is_null() {
            return Err(invalid_input("p_header must not be null"));
        }
        if color_type < 0 || color_type > u8::max_value() as c_int {
            return Err(invalid_input("Invalid color type"));
        }
        let color = ColorType::try_from(color_type as u8)?;
        (*p_header).set_color(color, depth)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_new(
    pp_encoder: *mut PEncoder,
    write_func: Option<CWriteFunc>,
    flush_func: Option<CFlushFunc>,
    user_data: *mut c_void,
    p_options: PEncoderOptions,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_encoder.is_null() {
            return Err(invalid_input("pp_encoder must not be null"));
        }
        if !(*pp_encoder).is_null() {
            return Err(invalid_input("*pp_encoder must be null"));
        }
        let writer = match (write_func, flush_func) {
            (Some(wf), Some(ff)) => CWriter::new(wf, ff, user_data),
            _ => return Err(invalid_input("write_func and flush_func must not be null")),
        };
        let default = Options::<'static>::new();
        let options = if p_options.is_null() {
            &default
        } else {
            &*p_options
        };
        let encoder = Encoder::new(writer, options);
        *pp_encoder = Box::into_raw(Box::new(encoder));
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_release(pp_encoder: *mut PEncoder) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_encoder.is_null() {
            return Err(invalid_input("pp_encoder must not be null"));
        }
        if (*pp_encoder).is_null() {
            return Err(invalid_input("*pp_encoder must not be null"));
        }
        drop(Box::from_raw(*pp_encoder));
        *pp_encoder = ptr::null_mut();
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_write_header(
    p_encoder: PEncoder,
    p_header: PHeader,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_encoder.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        if p_header.is_null() {
            return Err(invalid_input("p_header must not be null"));
        }
        (*p_encoder).write_header(&*p_header)?;
        Ok(())
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_write_palette(
    p_encoder: PEncoder,
    p_bytes: *const u8,
    len: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_encoder.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        if p_bytes.is_null() {
            return Err(invalid_input("p_bytes must not be null"));
        }
        let slice = ::std::slice::from_raw_parts(p_bytes, len);
        (*p_encoder).write_palette(slice)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_write_transparency(
    p_encoder: PEncoder,
    p_bytes: *const u8,
    len: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_encoder.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        if p_bytes.is_null() {
            return Err(invalid_input("p_bytes must not be null"));
        }
        let slice = ::std::slice::from_raw_parts(p_bytes, len);
        (*p_encoder).write_transparency(slice)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_write_chunk(
    p_encoder: PEncoder,
    p_tag: *const c_char,
    p_bytes: *const u8,
    len: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_encoder.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        if p_tag.is_null() {
            return Err(invalid_input("p_tag must not be null"));
        }
        if p_bytes.is_null() {
            return Err(invalid_input("p_bytes must not be null"));
        }
        let tag = CStr::from_ptr(p_tag).to_bytes();
        let slice = ::std::slice::from_raw_parts(p_bytes, len);
        (*p_encoder).write_chunk(tag, slice)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_write_image_rows(
    p_encoder: PEncoder,
    p_bytes: *const u8,
    len: size_t,
) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if p_encoder.is_null() {
            return Err(invalid_input("p_encoder must not be null"));
        }
        if p_bytes.is_null() {
            return Err(invalid_input("p_bytes must not be null"));
        }
        let slice = ::std::slice::from_raw_parts(p_bytes, len);
        (*p_encoder).write_image_rows(slice)
    }())
}

#[no_mangle]
pub unsafe extern "C" fn mtpng_encoder_finish(pp_encoder: *mut PEncoder) -> CResult {
    CResult::from(|| -> io::Result<()> {
        if pp_encoder.is_null() {
            return Err(invalid_input("pp_encoder must not be null"));
        }
        if (*pp_encoder).is_null() {
            return Err(invalid_input("*pp_encoder must not be null"));
        }

        // Take ownership back from C...
        let b_encoder = Box::from_raw(*pp_encoder);
        *pp_encoder = ptr::null_mut();

        // And finish it out.
        b_encoder.finish()?;
        Ok(())
    }())
}
