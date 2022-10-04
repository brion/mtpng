//
// mtpng - a multithreaded parallel PNG encoder in Rust
// lib.rs - library crate main file & public structs
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

//! mtpng - a multithreaded parallel PNG encoder in Rust

#[cfg(feature = "capi")]
pub mod capi;

mod adler32;
#[cfg(feature = "zlib")]
mod deflate;
#[cfg(all(feature = "miniz", not(feature = "zlib")))]
mod deflate_minz;
pub mod encoder;
mod filter;
mod utils;
mod writer;

#[cfg(all(feature = "miniz", not(feature = "zlib")))]
use deflate_minz as deflate;

pub type Strategy = deflate::Strategy;
pub type Filter = filter::Filter;

use std::convert::TryFrom;
use std::io;

use utils::invalid_input;

/// Wrapper for filter and compression modes.
#[derive(Copy, Clone)]
pub enum Mode<T> {
    /// Automatic selection based on file contents
    Adaptive,
    /// Fixed value
    Fixed(T),
}

/// PNG color types.
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum ColorType {
    /// Single brightness channel.
    Greyscale = 0,
    /// Red, green, and blue channels.
    Truecolor = 2,
    /// Single channel of palette indices.
    IndexedColor = 3,
    /// Brightness and alpha channels.
    GreyscaleAlpha = 4,
    /// Red, green, blue, and alpha channels.
    TruecolorAlpha = 6,
}
use ColorType::*;

impl TryFrom<u8> for ColorType {
    type Error = io::Error;

    /// Validate and produce a ColorType from one of the PNG header constants.
    ///
    /// Will return an error on invalid input.
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Greyscale),
            2 => Ok(Truecolor),
            3 => Ok(IndexedColor),
            4 => Ok(GreyscaleAlpha),
            6 => Ok(TruecolorAlpha),
            _ => Err(invalid_input("Invalid color type")),
        }
    }
}

impl ColorType {
    /// Check if the given bit depth is valid for this color type.
    ///
    /// See [the PNG standard](https://www.w3.org/TR/PNG/#table111) for valid types.
    pub fn is_depth_valid(self, depth: u8) -> bool {
        match self {
            Greyscale => matches!(depth, 1 | 2 | 4 | 8 | 16),
            GreyscaleAlpha | Truecolor | TruecolorAlpha => matches!(depth, 8 | 16),
            IndexedColor => matches!(depth, 1 | 2 | 4 | 8),
        }
    }

    /// Calculate the number of channels per pixel.
    pub fn channels(self) -> usize {
        match self {
            Greyscale => 1,
            Truecolor => 3,
            IndexedColor => 1,
            GreyscaleAlpha => 2,
            TruecolorAlpha => 4,
        }
    }
}

/// PNG header compression method representation.
///
/// There is only one method defined, which is Deflate.
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum CompressionMethod {
    /// Use zlib deflate compression, the default.
    Deflate = 0,
}

/// PNG header filter method representation.
///
/// Currently only Standard is supported. This may be expanded to support APNG in future.
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum FilterMethod {
    /// Use PNG standard filter types.
    Standard = 0,
}

/// PNG header interlace method representation.
///
/// Currently only Standard is supported; Adam7 interlacing will throw an error if used.
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum InterlaceMethod {
    /// No interlacing.
    ///
    /// Rows proceed from top to bottom and are the same length.
    Standard = 0,
    /// Adam7 interlacing.
    ///
    /// Not yet supported.
    Adam7 = 1,
}

/// PNG header representation.
///
/// You must create one of these with image metadata when encoding,
/// and can reuse the header for multiple encodings if desired.
#[derive(Copy, Clone)]
pub struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color_type: ColorType,
    compression_method: CompressionMethod,
    filter_method: FilterMethod,
    interlace_method: InterlaceMethod,
}

impl Header {
    /// Create a new Header struct with default settings.
    ///
    /// This will be 1x1 pixels, TruecolorAlpha 8-bit, with
    /// the standard compression, filter, and interlace methods.
    /// You can mutate the state using the set_* methods.
    pub fn new() -> Header {
        Header {
            width: 1,
            height: 1,
            depth: 8,
            color_type: ColorType::TruecolorAlpha,
            compression_method: CompressionMethod::Deflate,
            filter_method: FilterMethod::Standard,
            interlace_method: InterlaceMethod::Standard,
        }
    }

    /// Get the pixel width of the image.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the pixel height of the image.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the color depth of the image in bits.
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// Get the color type of the image.
    pub fn color_type(&self) -> ColorType {
        self.color_type
    }

    /// Get the compression method defined for the image.
    pub fn compression_method(&self) -> CompressionMethod {
        self.compression_method
    }

    /// Get the filter method defined for the image.
    pub fn filter_method(&self) -> FilterMethod {
        self.filter_method
    }

    /// Get the interlace method defined for the image.
    pub fn interlace_method(&self) -> InterlaceMethod {
        self.interlace_method
    }

    /// Calculate the bytes per pixel, for PNG filtering purposes.
    ///
    /// If the bit depth is < 8, this will clamp at 1.
    pub fn bytes_per_pixel(&self) -> usize {
        self.color_type.channels() * if self.depth > 8 { 2 } else { 1 }
    }

    /// Calculate the stride in bytes for the encoded pixel rows.
    ///
    /// Will panic on arithmetic overflow if given pathologically long rows.
    pub fn stride(&self) -> usize {
        let bits_per_pixel = self.color_type.channels() * self.depth as usize;

        // Very long line lengths can overflow usize on 32-bit.
        // If we got this far, let it panic in the unwrap().
        let stride_bits = bits_per_pixel.checked_mul(self.width as usize).unwrap();

        // And round up to nearest byte.
        let stride_bytes = stride_bits >> 3;
        let remainder = stride_bits & 3;
        if remainder > 0 {
            stride_bytes + 1
        } else {
            stride_bytes
        }
    }

    /// Set the pixel dimensions of the image.
    ///
    /// Returns error if width or height are 0.
    ///
    /// Warning: it's possible to make combinations of width and color type
    /// that cannot fit in memory on 32-bit systems. These are not detected
    /// here, but will panic when stride() is called.
    pub fn set_size(&mut self, width: u32, height: u32) -> io::Result<()> {
        if width == 0 {
            Err(invalid_input("width cannot be 0"))
        } else if height == 0 {
            Err(invalid_input("height canno tbe 0"))
        } else {
            self.width = width;
            self.height = height;
            Ok(())
        }
    }

    /// Set the color type and depth of the image.
    ///
    /// Returns error if depth is invalid for the given color type.
    ///
    /// Warning: it's possible to make combinations of width and color type
    /// that cannot fit in memory on 32-bit systems. These are not detected
    /// here, but will panic when stride() is called.
    pub fn set_color(&mut self, color_type: ColorType, depth: u8) -> io::Result<()> {
        if !color_type.is_depth_valid(depth) {
            Err(invalid_input("invalid color depth for this color type"))
        } else {
            self.color_type = color_type;
            self.depth = depth;
            Ok(())
        }
    }

    /// Set the compression method.
    ///
    /// This is not very useful, as only deflate is supported.
    pub fn set_compression_method(
        &mut self,
        compression_method: CompressionMethod,
    ) -> io::Result<()> {
        self.compression_method = compression_method;
        Ok(())
    }

    /// Set the filter method.
    ///
    /// Currently only Standard is supported.
    pub fn set_filter_method(&mut self, filter_method: FilterMethod) -> io::Result<()> {
        self.filter_method = filter_method;
        Ok(())
    }

    /// Set the interlace method.
    ///
    /// Currently only Standard is supported; requesting Adam7 will return an error.
    pub fn set_interlace_method(&mut self, interlace_method: InterlaceMethod) -> io::Result<()> {
        match interlace_method {
            InterlaceMethod::Standard => {}
            InterlaceMethod::Adam7 => return Err(invalid_input("Adam7 interlacing not yet")),
        }
        self.interlace_method = interlace_method;
        Ok(())
    }
}

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

/// Representation of deflate compression level.
#[derive(Copy, Clone)]
pub enum CompressionLevel {
    /// Fast but poor compression (zlib level 1).
    Fast,
    /// Good balance of speed and compression (zlib level 6).
    Default,
    /// Best compression but slow (zlib level 9).
    High,
}

impl TryFrom<u8> for CompressionLevel {
    type Error = io::Error;

    /// Validate and convert u8 to CompressionLevel.
    ///
    /// Will return an error on invalid input.
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            1 => Ok(CompressionLevel::Fast),
            6 => Ok(CompressionLevel::Default),
            9 => Ok(CompressionLevel::High),
            _ => Err(invalid_input("Compression level not supported")),
        }
    }
}
