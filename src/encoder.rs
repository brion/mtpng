//
// mtpng - a multithreaded parallel PNG encoder in Rust
// encoder.rs - implements the public encoder interface & internals
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

use std::collections::VecDeque;

use std::io::Write;
use std::io::{self, ErrorKind};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::adler32;

use super::ColorType;
use super::CompressionLevel;
use super::Header;
use super::Mode;
use super::Mode::{Adaptive, Fixed};
use super::Strategy;

use super::filter::AdaptiveFilter;
use super::filter::Filter;
use super::writer::Writer;

use super::deflate;
use super::deflate::Deflate;
use super::deflate::Flush;

use super::utils::*;

/// Options setup struct for the PNG encoder.
/// May be modified and reused.
#[derive(Copy, Clone)]
pub struct Options<'a> {
    chunk_size: usize,
    compression_level: CompressionLevel,
    strategy_mode: Mode<Strategy>,
    filter_mode: Mode<Filter>,
    streaming: bool,
    thread_pool: Option<&'a ThreadPool>,
}

impl<'a> Options<'a> {
    /// Create a new Options struct using default options:
    /// * chunk_size: 256 KiB
    /// * compression_level: Default
    /// * strategy_mode: Adaptive
    /// * filter_mode: Adaptive
    /// * streaming: off
    /// * thread_pool: global default
    ///
    /// The compression, strategy, and filtering use the same
    /// defaults as libpng.
    pub fn new() -> Options<'a> {
        Options {
            //
            // A chunk size of 256 KiB gives compression results very similar
            // to a single stream when otherwise using defaults.
            //
            chunk_size: 256 * 1024,

            //
            // Same defaults as libpng.
            //
            compression_level: CompressionLevel::Default,
            strategy_mode: Adaptive,
            filter_mode: Adaptive,

            //
            // Streaming mode can produce lower latency to first bytes hitting
            // output on large files, at the cost of size -- several extra
            // 32-bit words per chunk, which adds up.
            //
            // Leaving off will buffer compressed image data into memory until
            // the end is reached.
            //
            streaming: false,

            //
            // Use the global thread pool.
            //
            thread_pool: None,
        }
    }

    /// Use a custom Rayon ThreadPool instance instead of the global pool.
    pub fn set_thread_pool(&mut self, thread_pool: &'a ThreadPool) -> IoResult {
        self.thread_pool = Some(thread_pool);
        Ok(())
    }

    /// Set the size in bytes of chunks used for distributing data to threads.
    /// The actual chunk size used will be a multiple of row lengths approximating
    /// the requested size.
    ///
    /// Chunk size must be at least 32 KiB.
    pub fn set_chunk_size(&mut self, chunk_size: usize) -> IoResult {
        if chunk_size < 32768 {
            Err(invalid_input("chunk size must be at least 32768"))
        } else {
            self.chunk_size = chunk_size;
            Ok(())
        }
    }

    /// Set the deflate compression level.
    /// Currently supported are Fast (equivalent to gzip -1),
    /// Default (gzip -6), and High (gzip -9).
    pub fn set_compression_level(&mut self, level: CompressionLevel) -> IoResult {
        self.compression_level = level;
        Ok(())
    }

    /// Set the pixel filtering mode. By default it will use Adaptive,
    /// which tries all filter modes and a heuristic to guess which will
    /// compress better on a line-by-line basis.
    ///
    /// The same logic and heuristic are used as in libpng,
    /// which often does well but can pick poorly on some images.
    /// Fixed<*> may be used to override the mode for the whole image,
    /// which sometimes produces better results than the heuristic.
    pub fn set_filter_mode(&mut self, filter_mode: Mode<Filter>) -> IoResult {
        self.filter_mode = filter_mode;
        Ok(())
    }

    /// Set the deflate compression strategy. By default it will use Adaptive,
    /// which picks Default for Fixed<None> or Filtered for other filter types.
    /// This matches libpng's logic as well.
    pub fn set_strategy_mode(&mut self, strategy_mode: Mode<Strategy>) -> IoResult {
        self.strategy_mode = strategy_mode;
        Ok(())
    }

    /// Enable or disable streaming mode, which emits a separate "IDAT" PNG chunk
    /// around each compressed data chunk. This allows for streaming a large file
    /// over a network etc during compression, at a cost of a few more bytes at
    /// chunk boundaries.
    pub fn set_streaming(&mut self, streaming: bool) -> IoResult {
        self.streaming = streaming;
        Ok(())
    }
}

impl<'a> Default for Options<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// Accumulates a set of pixels, then gets sent off as input
// to the deflate jobs.
struct PixelChunk {
    header: Header,

    index: usize,
    start_row: usize,
    end_row: usize,
    is_start: bool,
    is_end: bool,

    stride: usize,

    // Rows of pixel data, each with stride bytes per row
    rows: Vec<Vec<u8>>,
}

impl PixelChunk {
    fn new(header: Header, index: usize, start_row: usize, end_row: usize) -> PixelChunk {
        assert!(start_row <= end_row);

        let height = header.height as usize;
        assert!(end_row <= height);

        PixelChunk {
            header,

            index,
            start_row,
            end_row,
            is_start: start_row == 0,
            is_end: end_row == height,

            stride: header.stride(),

            rows: Vec::with_capacity(end_row - start_row),
        }
    }

    fn is_full(&self) -> bool {
        self.rows.len() == (self.end_row - self.start_row)
    }

    fn read_row(&mut self, row: &[u8]) {
        let mut row_copy = Vec::with_capacity(self.stride);
        row_copy.extend_from_slice(row);

        self.rows.push(row_copy);
    }

    fn get_row(&self, row: usize) -> Result<&[u8], std::io::Error> {
        if row < self.start_row {
            Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Tried to access row from earlier chunk: {} < {}",
                    row, self.start_row
                ),
            ))
        } else if row >= self.end_row {
            Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Tried to access row from later chunk: {} >= {}",
                    row, self.end_row
                ),
            ))
        } else {
            Ok(&self.rows[row - self.start_row])
        }
    }
}

// Takes pixel chunks as input and accumulates filtered output.
struct FilterChunk {
    index: usize,
    start_row: usize,
    end_row: usize,
    is_start: bool,
    is_end: bool,

    stride: usize,
    filter_mode: Mode<Filter>,

    // The input pixels for chunk n-1
    // Needed for its last row only.
    prior_input: Option<Arc<PixelChunk>>,

    // The input pixels for chunk n
    input: Arc<PixelChunk>,

    // Filtered output bytes
    data: Vec<u8>,
}

impl FilterChunk {
    fn new(
        prior_input: Option<Arc<PixelChunk>>,
        input: Arc<PixelChunk>,
        filter_mode: Mode<Filter>,
    ) -> FilterChunk {
        // Prepend one byte for the filter selector.
        let stride = input.stride + 1;
        let nbytes = stride * (input.end_row - input.start_row);

        FilterChunk {
            index: input.index,
            start_row: input.start_row,
            end_row: input.end_row,
            is_start: input.is_start,
            is_end: input.is_end,

            stride,
            filter_mode,

            prior_input,
            input,
            data: Vec::with_capacity(nbytes),
        }
    }

    // Return the last up-to-32kib, used as an input dictionary
    // for the next chunk's deflate job.
    fn get_trailer(&self) -> &[u8] {
        let trailer = 32768;
        let len = self.data.len();
        if len > trailer {
            &self.data[len - trailer..len]
        } else {
            &self.data[0..len]
        }
    }

    //
    // Run the filtering, on a background thread.
    //
    fn run(&mut self) -> IoResult {
        let mut filter = AdaptiveFilter::new(self.input.header, self.filter_mode);
        let zero = vec![0u8; self.stride - 1];
        for i in self.start_row..self.end_row {
            let prior = if i == self.start_row {
                match self.prior_input {
                    Some(ref input) => input,
                    None => &self.input, // Won't get used.
                }
            } else {
                &self.input
            };
            let prev = if i == 0 { &zero } else { prior.get_row(i - 1)? };

            let row = self.input.get_row(i)?;

            let output = filter.filter(prev, row);

            self.data.write_all(output)?
        }
        Ok(())
    }
}

// Takes filter chunks as input and accumulates compressed output.
struct DeflateChunk {
    index: usize,
    is_start: bool,
    is_end: bool,

    compression_level: CompressionLevel,
    strategy: Strategy,

    // The filtered pixels for chunk n-1
    // Empty on first chunk.
    // Needed for its last row only.
    prior_input: Option<Arc<FilterChunk>>,

    // The filtered pixels for chunk n
    input: Arc<FilterChunk>,

    // Compressed output bytes
    data: Vec<u8>,

    // Checksum of this chunk
    adler32: u32,
}

impl DeflateChunk {
    fn new(
        compression_level: CompressionLevel,
        strategy: Strategy,
        prior_input: Option<Arc<FilterChunk>>,
        input: Arc<FilterChunk>,
    ) -> DeflateChunk {
        DeflateChunk {
            index: input.index,
            is_start: input.is_start,
            is_end: input.is_end,

            compression_level,
            strategy,

            prior_input,
            input,
            data: Vec::new(),
            adler32: adler32::adler32_initial(),
        }
    }

    fn run(&mut self) -> IoResult {
        // Run the deflate!
        // Todo: don't create an empty vector earlier, but reuse it sanely.
        let data = Vec::<u8>::new();

        let mut options = deflate::Options::new();

        options.set_window_bits(if self.is_start {
            // 15 means 2^15 (32 KiB), the max supported.
            15
        } else {
            // Negative forces raw stream output so we don't get
            // a second header...
            -15
        });

        match self.compression_level {
            CompressionLevel::Default => {}
            CompressionLevel::Fast => options.set_level(1),
            CompressionLevel::High => options.set_level(9),
        }
        options.set_strategy(self.strategy);

        let mut encoder = Deflate::new(options, data);

        if let Some(ref filter) = self.prior_input {
            let trailer = filter.get_trailer();
            encoder.set_dictionary(trailer)?;
        }

        encoder.write(
            &self.input.data,
            if self.is_end {
                Flush::Finish
            } else {
                Flush::SyncFlush
            },
        )?;

        // In raw deflate mode we have to calculate the checksum ourselves.
        self.adler32 = adler32::adler32(1, &self.input.data);

        match encoder.finish() {
            Ok(data) => {
                // This seems lame to move the vector back, but it's actually cheap.
                self.data = data;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

//
// List of completed chunks, which may come in in any order
// but are returned in original order, in pairs with the
// prior chunk when available.
//
// The prior chunk is passed around because filtering and
// deflating jobs need the end (last row, or last 32 KiB)
// of the previous chunk's input as well as their own.
//
struct ChunkMap<T> {
    cursor_in: usize,
    cursor_out: usize,
    running: usize,

    chunks: VecDeque<Option<Arc<T>>>,
    prev: Option<Arc<T>>,
}

impl<T> ChunkMap<T> {
    fn new() -> ChunkMap<T> {
        ChunkMap {
            cursor_in: 0,
            cursor_out: 0,
            running: 0,
            chunks: VecDeque::new(),
            prev: None,
        }
    }

    fn in_flight(&self) -> bool {
        self.cursor_in > self.cursor_out
    }

    fn running_jobs(&self) -> usize {
        self.running
    }

    //
    // Record that this job is now in-flight
    //
    fn advance(&mut self) {
        self.cursor_in += 1;
        self.running += 1;
    }

    //
    // Record that this job has landed and save its data.
    //
    fn land(&mut self, index: usize, chunk: Arc<T>) -> IoResult {
        if index < self.cursor_out {
            return Err(std::io::Error::new(
                io::ErrorKind::InvalidInput,
                "Tried to land an expired chunk",
            ));
        }
        if index > self.cursor_in {
            return Err(std::io::Error::new(
                io::ErrorKind::InvalidData,
                "Tried to land a future chunk",
            ));
        }
        self.running -= 1;
        let offset = index - self.cursor_out;
        while offset > self.chunks.len() {
            self.chunks.push_back(None);
        }
        if offset == self.chunks.len() {
            self.chunks.push_back(Some(chunk));
        } else {
            self.chunks[offset] = Some(chunk);
        }
        Ok(())
    }

    fn pop_front(&mut self) -> Option<(Option<Arc<T>>, Arc<T>)> {
        match self.chunks.get(0) {
            Some(Some(_)) => {
                // Ok we're good we have something
                self.cursor_out += 1;
                match self.chunks.pop_front() {
                    Some(Some(item)) => {
                        let prev = std::mem::replace(&mut self.prev, Some(Arc::clone(&item)));
                        Some((prev, item))
                    }
                    _ => {
                        println!("[mtpng] Bad job queue state");
                        None
                    }
                }
            }
            Some(None) => {
                // Not ready yet but a later chunk landed.
                None
            }
            None => {
                // Nothing yet.
                None
            }
        }
    }
}

enum ThreadMessage {
    FilterDone(Arc<FilterChunk>),
    DeflateDone(Arc<DeflateChunk>),
    Error(io::Error),
}

#[derive(Copy, Clone)]
enum DispatchMode {
    Blocking,
    NonBlocking,
}

enum RowStatus {
    Continue,
    Done,
}

/// Parallel PNG encoder state.
/// Takes an Options struct with initializer data and a Write struct
/// to send output to.
pub struct Encoder<'a, W: Write> {
    writer: Writer<W>,
    options: Options<'a>,

    header: Header,

    wrote_header: bool,
    wrote_palette: bool,
    palette_length: usize,
    wrote_transparency: bool,
    started_image: bool,

    chunks_total: usize,
    chunks_output: usize,

    // Accumulates input rows until enough are ready to fire off a filter job.
    pixel_accumulator: Arc<PixelChunk>,
    pixel_index: usize,
    current_row: u32,

    // Accumulates completed output from pixel input, filter, and deflate jobs.
    pixel_chunks: ChunkMap<PixelChunk>,
    filter_chunks: ChunkMap<FilterChunk>,
    deflate_chunks: ChunkMap<DeflateChunk>,

    // Accumulates the checksum of all output chunks in turn.
    adler32: u32,

    // Accumulates IDAT output when not using streaming output mode
    idat_buffer: Vec<u8>,

    // For messages from the thread pool.
    tx: Sender<ThreadMessage>,
    rx: Receiver<ThreadMessage>,
}

impl<'a, W: Write> Encoder<'a, W> {
    /// Creates a new Encoder instance with the given Write output sink and options.
    pub fn new(write: W, options: &Options<'a>) -> Encoder<'a, W> {
        let (tx, rx) = mpsc::channel();
        Encoder {
            writer: Writer::new(write),

            header: Header::new(),
            options: *options,

            wrote_header: false,
            wrote_palette: false,
            palette_length: 0,
            wrote_transparency: false,
            started_image: false,

            chunks_total: 0,
            chunks_output: 0,

            // hack, clean this up later
            pixel_accumulator: Arc::new(PixelChunk::new(Header::new(), 0, 0, 0)),
            pixel_index: 0,
            current_row: 0,

            pixel_chunks: ChunkMap::new(),
            filter_chunks: ChunkMap::new(),
            deflate_chunks: ChunkMap::new(),

            adler32: adler32::adler32_initial(),
            idat_buffer: Vec::new(),

            tx,
            rx,
        }
    }

    /// Flush output and return the Write sink for further manipulation.
    /// Consumes the encoder instance.
    pub fn finish(mut self) -> io::Result<W> {
        self.flush()?;
        if self.is_finished() {
            self.writer.write_end()?;
            self.writer.finish()
        } else {
            Err(other("Incomplete image input"))
        }
    }

    fn running_jobs(&self) -> usize {
        self.filter_chunks.running_jobs() + self.deflate_chunks.running_jobs()
    }

    fn threads(&self) -> usize {
        match self.options.thread_pool {
            Some(pool) => pool.current_num_threads(),
            None => ::rayon::current_num_threads(),
        }
    }

    fn max_threads(&self) -> usize {
        // Keep the threads busy by queueing a couple extra jobs
        // But not so busy that we don't interleave types
        self.threads() + 2
    }

    fn dispatch_func<F>(&self, func: F)
    where
        F: Fn(&Sender<ThreadMessage>) + Send + 'static,
    {
        let tx = self.tx.clone();
        match self.options.thread_pool {
            Some(pool) => {
                pool.spawn(move || {
                    func(&tx);
                });
            }
            None => {
                rayon::spawn(move || {
                    func(&tx);
                });
            }
        }
    }

    fn start_row(&self, index: usize) -> usize {
        index * self.header.height() as usize / self.chunks_total
    }

    fn end_row(&self, index: usize) -> usize {
        self.start_row(index + 1)
    }

    fn receive(&mut self, blocking: DispatchMode) -> Option<ThreadMessage> {
        match blocking {
            DispatchMode::Blocking => match self.rx.recv() {
                Ok(msg) => Some(msg),
                _ => None,
            },
            DispatchMode::NonBlocking => match self.rx.try_recv() {
                Ok(msg) => Some(msg),
                _ => None,
            },
        }
    }

    fn filter_mode(&self) -> Mode<Filter> {
        match self.options.filter_mode {
            Fixed(s) => Fixed(s),
            Adaptive => match self.header.color_type {
                ColorType::IndexedColor => Fixed(Filter::None),
                _ => Adaptive,
            },
        }
    }

    fn compression_strategy(&self) -> Strategy {
        match self.options.strategy_mode {
            Fixed(s) => s,
            Adaptive => match self.filter_mode() {
                Fixed(Filter::None) => Strategy::Default,
                _ => Strategy::Filtered,
            },
        }
    }

    fn dispatch(&mut self, mode: DispatchMode) -> IoResult {
        // See if anything interesting happened on the threads.
        let mut blocking_mode = mode;
        while self.filter_chunks.in_flight() || self.deflate_chunks.in_flight() {
            match self.receive(blocking_mode) {
                Some(ThreadMessage::FilterDone(filter)) => {
                    self.filter_chunks.land(filter.index, filter)?;
                }
                Some(ThreadMessage::DeflateDone(deflate)) => {
                    self.deflate_chunks.land(deflate.index, deflate)?;
                }
                Some(ThreadMessage::Error(e)) => {
                    return Err(e);
                }
                None => {
                    // No more output from the threads.
                    break;
                }
            }
            // After the first one, keep reading any if they're there
            // but don't block further.
            blocking_mode = DispatchMode::NonBlocking;
        }

        // If we have more deflate work to do, dispatch them!
        while self.running_jobs() < self.max_threads() {
            match self.filter_chunks.pop_front() {
                Some((previous, current)) => {
                    // Prepare to dispatch the deflate job:
                    let level = self.options.compression_level;
                    let strategy = self.compression_strategy();
                    self.deflate_chunks.advance();
                    self.dispatch_func(move |tx| {
                        let mut deflate =
                            DeflateChunk::new(level, strategy, previous.clone(), current.clone());
                        tx.send(match deflate.run() {
                            Ok(()) => ThreadMessage::DeflateDone(Arc::new(deflate)),
                            Err(e) => ThreadMessage::Error(e),
                        })
                        .ok();
                    });
                }
                None => {
                    break;
                }
            }
        }

        // If we have more filter work to do, dispatch them!
        while self.running_jobs() < self.max_threads() {
            match self.pixel_chunks.pop_front() {
                Some((previous, current)) => {
                    // Prepare to dispatch the filter job:
                    self.filter_chunks.advance();
                    let filter_mode = self.filter_mode();
                    self.dispatch_func(move |tx| {
                        let mut filter =
                            FilterChunk::new(previous.clone(), current.clone(), filter_mode);
                        tx.send(match filter.run() {
                            Ok(()) => ThreadMessage::FilterDone(Arc::new(filter)),
                            Err(e) => ThreadMessage::Error(e),
                        })
                        .ok();
                    });
                }
                None => {
                    break;
                }
            }
        }

        // If we have output to run, write it!
        while let Some((_previous, current)) = self.deflate_chunks.pop_front() {
            if self.chunks_output >= self.chunks_total {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "Got extra output after end of file; should not happen.",
                ));
            }

            // Combine the checksums!
            self.adler32 =
                adler32::adler32_combine(self.adler32, current.adler32, current.input.data.len());

            // if not streaming, append to an in-memory buffer
            // and output a giant tag later.
            if self.options.streaming {
                self.writer.write_chunk(b"IDAT", &current.data)?;

                if current.is_end {
                    let mut chunk = Vec::<u8>::new();
                    if !current.is_start {
                        write_be32(&mut chunk, self.adler32)?;
                    }
                    self.writer.write_chunk(b"IDAT", &chunk)?;
                }
            } else {
                self.idat_buffer.write_all(&current.data)?;

                if current.is_end {
                    if !current.is_start {
                        write_be32(&mut self.idat_buffer, self.adler32)?;
                    }
                    self.writer.write_chunk(b"IDAT", &self.idat_buffer)?;
                }
            }

            self.chunks_output += 1;
        }

        Ok(())
    }

    /// Write the PNG signature and header chunk.
    /// Must be done before anything else is output.
    ///
    /// Subsequent image data must match the given header data.
    pub fn write_header(&mut self, header: &Header) -> IoResult {
        if self.wrote_header {
            return Err(invalid_input("Cannot write header a second time."));
        }

        self.header = *header;

        let stride = self.header.stride() + 1;
        let height = self.header.height as usize;

        let chunks = stride * height / self.options.chunk_size;
        self.chunks_total = if chunks < 1 { 1 } else { chunks };

        self.pixel_chunks.advance();
        self.pixel_accumulator = Arc::new(PixelChunk::new(
            self.header,
            0, // index
            self.start_row(0),
            self.end_row(0),
        ));

        self.wrote_header = true;

        self.writer.write_signature()?;
        self.writer.write_header(self.header)
    }

    pub fn write_srgb(&mut self) -> IoResult {
        if self.started_image {
            return Err(invalid_data("Cannot write sRGB after image started."));
        }
        self.writer.write_srgb()
    }

    /// Write an indexed-color palette as a PLTE chunk.
    ///
    /// Data must be formatted per the spec matching the color mode:
    /// https://www.w3.org/TR/2003/REC-PNG-20031110/#11PLTE
    ///
    /// Note this chunk is allowed on truecolor images, though sPLT is preferred.
    pub fn write_palette(&mut self, palette: &[u8]) -> io::Result<()> {
        if !self.wrote_header {
            return Err(invalid_input("Cannot write palette before header."));
        }
        if self.wrote_palette {
            return Err(invalid_input("Cannot write palette a second time."));
        }
        if self.wrote_transparency {
            return Err(invalid_input("Cannot write palette after transparency."));
        }
        if self.started_image {
            return Err(invalid_input("Cannot write palette after image data."));
        }
        if palette.len() < 3 {
            return Err(invalid_input("Palette must have at least one entry."));
        }
        if palette.len() % 3 != 0 {
            return Err(invalid_input(
                "Palette must have an integral number of entries.",
            ));
        }

        self.wrote_palette = true;
        self.palette_length = palette.len() / 3;
        self.writer.write_chunk(b"PLTE", palette)
    }

    /// Write a transparency info chunk.
    ///
    /// For indexed color, contains a single alpha value byte per palette
    /// entry, up to but not exceeding the number of palette entries.
    ///
    /// Note this chunk is allowed on greyscale and truecolor images,
    /// and there references a single color in 16-bit notation.
    ///
    /// https://www.w3.org/TR/PNG/#11tRNS
    pub fn write_transparency(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.wrote_header {
            return Err(invalid_input("Cannot write transparency before header."));
        }
        if self.started_image {
            return Err(invalid_input("Cannot write transparency after image data."));
        }
        match self.header.color_type {
            ColorType::Greyscale => {
                if data.len() != 2 {
                    return Err(invalid_input(
                        "Greyscale transparency data must be exactly 2 bytes.",
                    ));
                }
            }
            ColorType::Truecolor => {
                if data.len() != 6 {
                    return Err(invalid_input(
                        "Truecolor transparency data must be exactly 6 bytes.",
                    ));
                }
            }
            ColorType::IndexedColor => {
                if !self.wrote_palette {
                    return Err(invalid_input("Cannot write transparency before palette."));
                }
                if data.is_empty() {
                    return Err(invalid_input("Transparency data too short."));
                }
                if data.len() > self.palette_length {
                    return Err(invalid_input(
                        "Transparency data cannot contain more entries than palette.",
                    ));
                }
            }
            _ => {
                return Err(invalid_input(
                    "Transparency chunk is invalid for color types with alpha",
                ));
            }
        }
        self.wrote_transparency = true;
        self.writer.write_chunk(b"tRNS", data)
    }

    //
    // Write a custom ancillary chunk to the output stream.
    // The tag must be a 4-byte slice. The data should be provided
    // in the appropriate format for the tag.
    //
    pub fn write_chunk(&mut self, tag: &[u8], data: &[u8]) -> io::Result<()> {
        self.writer.write_chunk(tag, data)
    }

    //
    // Copy a row's pixel data into buffers for async compression.
    // Returns immediately after copying.
    //
    fn process_row(&mut self, row: &[u8]) -> io::Result<RowStatus> {
        if self.pixel_index >= self.chunks_total {
            return Err(other("invalid internal state"));
        }
        if !self.wrote_header {
            return Err(invalid_input("Cannot write image data before header."));
        }
        if let ColorType::IndexedColor = self.header.color_type {
            if !self.wrote_palette {
                return Err(invalid_input(
                    "Cannot write indexed-color image data before palette.",
                ));
            }
        }
        if !self.started_image {
            self.started_image = true;
        }

        Arc::get_mut(&mut self.pixel_accumulator)
            .unwrap()
            .read_row(row);

        if self.pixel_accumulator.is_full() {
            // Move the item off to the completed stack...
            self.pixel_chunks
                .land(self.pixel_index, self.pixel_accumulator.clone())?;

            // Make a nice new buffer to accumulate data into.
            self.pixel_index += 1;
            if self.pixel_index < self.chunks_total {
                self.pixel_chunks.advance();
                self.pixel_accumulator = Arc::new(PixelChunk::new(
                    self.header,
                    self.pixel_index,
                    self.start_row(self.pixel_index),
                    self.end_row(self.pixel_index),
                ));
            }

            // Dispatch any available async tasks and output.
            while self.running_jobs() >= self.max_threads() {
                self.dispatch(DispatchMode::Blocking)?;
            }
            self.dispatch(DispatchMode::NonBlocking)?;
        }

        self.current_row += 1;
        if self.current_row == self.header.height {
            Ok(RowStatus::Done)
        } else {
            Ok(RowStatus::Continue)
        }
    }

    /// Encode and compress the given image data and write to output.
    /// Input data must be packed in the correct format for the given
    /// color type and depth, with no padding at the end of rows.
    ///
    /// An integral number of rows must be provided at once.
    ///
    /// If not all of the image rows are provided, multiple calls are
    /// required to finish out the data.
    pub fn write_image_rows(&mut self, buf: &[u8]) -> IoResult {
        let stride = self.header.stride();
        if buf.len() % stride != 0 {
            Err(invalid_input("Buffer must be an integral number of rows"))
        } else {
            for row in buf.chunks(stride) {
                self.process_row(row)?;
            }
            Ok(())
        }
    }

    /// Return completion progress as a fraction of 1.0
    ///
    /// Currently progress is measured in chunks, so small files may
    /// not report values between 0.0 and 1.0.
    pub fn progress(&self) -> f64 {
        self.chunks_output as f64 / self.chunks_total as f64
    }

    /// Return finished-ness state.
    /// Is it finished? Yeah or no.
    pub fn is_finished(&self) -> bool {
        self.chunks_output == self.chunks_total
    }

    /// Flush all currently in-progress data to output
    /// Warning: this may block.
    pub fn flush(&mut self) -> IoResult {
        while self.chunks_output < self.pixel_index {
            // Dispatch any available async tasks and output.
            self.dispatch(DispatchMode::Blocking)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ColorType;
    use super::super::Header;
    use super::Encoder;
    use super::IoResult;
    use super::Options;

    use std::io;

    fn test_encoder<F>(width: u32, height: u32, func: F)
    where
        F: Fn(&mut Encoder<Vec<u8>>, &[u8]) -> IoResult,
    {
        match {
            || -> io::Result<Vec<u8>> {
                let mut data = Vec::<u8>::with_capacity(width as usize * 3);
                for i in 0..width as usize * 3 {
                    data.push((i % 255) as u8);
                }

                let writer = Vec::<u8>::new();
                let options = Options::new();
                let mut encoder = Encoder::new(writer, &options);

                let mut header = Header::new();
                header.set_size(width, height).unwrap();
                header.set_color(ColorType::Truecolor, 8).unwrap();
                encoder.write_header(&header)?;

                func(&mut encoder, &data)?;
                encoder.finish()
            }()
        } {
            Ok(_writer) => {}
            Err(e) => panic!("Error {}", e),
        }
    }

    #[test]
    fn create_and_state() {
        test_encoder(1920, 1080, |encoder, data| {
            assert!(!encoder.is_finished());
            assert!(encoder.progress() < f64::EPSILON);

            // We must finish out the file or it'll whinge.
            for _y in 0..1080 {
                encoder.write_image_rows(data)?;
            }

            Ok(())
        });
    }

    #[test]
    fn test_rows() {
        test_encoder(1920, 1080, |encoder, data| {
            assert!(!encoder.is_finished());
            assert!(encoder.progress() < f64::EPSILON);

            for _y in 0..1080 {
                encoder.write_image_rows(data)?;
            }

            // Should trigger all blocks!
            encoder.flush()?;
            assert!(encoder.is_finished());
            assert!(encoder.progress() > f64::EPSILON);

            Ok(())
        });
    }
}
