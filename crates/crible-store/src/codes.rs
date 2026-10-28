// SPDX-License-Identifier: Apache-2.0

//! Binary codes of the passages of one segment (`codes.bin`).
//!
//! A segment directory holds the reference vectors of a contiguous range of
//! internal identifiers plus one binary code per vector, in `codes.bin`
//! (`docs/02-architecture.md`, "Organisation sur disque"). The code of a vector
//! keeps one bit per dimension, the sign of the component: bit `i` is 1 when
//! `x_i > 0`. A query scans those bits with a Hamming distance, then rescores
//! the best candidates on the reference vectors (`ADR-0004`).
//!
//! # Format
//!
//! `codes.bin` holds **no header** (`ADR-0069`, decision 6):
//!
//! - one code after another, in internal-identifier order;
//! - one code is `ceil(dims / 64)` words of `u64`, little-endian;
//! - the bits above `dims` in the last word stay zero, so a popcount over the
//!   whole word never counts padding;
//! - the length of the file is exactly `count × ceil(dims / 64) × 8` bytes.
//!
//! The version of that layout is the `format_version` of the `segment.json` of
//! the same directory, as for `CURRENT_EPOCH` (`docs/02-architecture.md:84`). No
//! second version number exists, so none can disagree.
//!
//! `codes.bin` is **derived**: it reconstructs from the `vectors.*` of its
//! segment (`docs/02-architecture.md:80`), so it is absent from the checksums of
//! `segment.json`, from an export and from a snapshot (`ADR-0061`, `ADR-0064`
//! decision 4). [`rebuild_codes`] is the "reconstruction job" of the segment
//! task: called by the writer open, it persists the missing file atomically,
//! without a new generation and without a new epoch (`ADR-0069`, decision 1). A
//! read-only open never calls it: [`crate::segment::SegmentReader::open`] builds
//! the missing codes in RAM and writes nothing under the epoch (`ADR-0069`,
//! decision 2).
//!
//! An absent file is not an error; a file of the wrong length is one
//! (`ErrorCode::Internal`, naming the file), because it is a corruption, not a
//! derived file to rebuild (`ADR-0069`, decision 3).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::Path;

use crible_core::error::{CribleError, ErrorCode};

use crate::generation::stop_at_failpoint;
use crate::segment::{self, SegmentReader, io_error, sync_directory};

/// Name of the binary-code file of a segment directory.
pub const CODES_FILE_NAME: &str = "codes.bin";

/// Failpoint step of an interrupted rebuild, read from `CRIBLE_FAILPOINT` in a
/// `failpoints` build (`ADR-0069`; see [`rebuild_codes`]).
pub(crate) const CODES_REBUILD_FAILPOINT: &str = "codes_rebuild_before_rename";

/// Prefix of the temporary file a rebuild writes before its rename.
///
/// The same `tmp-` prefix as everywhere in the store: a rebuild that stops
/// leaves `tmp-codes.bin` inert inside the segment directory, and the next
/// rebuild overwrites it before the rename.
const TEMPORARY_PREFIX: &str = "tmp-";

/// Number of bits one `u64` word holds.
const BITS_PER_WORD: usize = 64;

/// Number of bytes of one `u64` word.
const BYTES_PER_WORD: u64 = 8;

/// Number of `u64` words of one code: `ceil(dims / 64)`.
///
/// A code is padded to whole words: a dimension count that is not a multiple of
/// 64 still needs its last word, never a partial one (`ADR-0069`, decision 6).
#[must_use]
pub fn words_per_code(dims: u32) -> usize {
    (dims as usize).div_ceil(BITS_PER_WORD)
}

/// Byte length of a `codes.bin` holding `count` codes of `dims` dimensions:
/// `count × words_per_code(dims) × 8`.
///
/// Two `u32` operands cannot overflow a `u64`, so the function is total.
#[must_use]
pub fn codes_len(count: u32, dims: u32) -> u64 {
    u64::from(count) * words_per_code(dims) as u64 * BYTES_PER_WORD
}

/// Code of one vector: bit `i` is 1 when `vector[i] > 0`, and the bits above the
/// length of `vector` stay zero (`ADR-0004`, `ADR-0069` decision 6).
///
/// Neither a null component nor a negative one sets its bit.
#[must_use]
pub fn quantize(vector: &[f32]) -> Vec<u64> {
    let mut words = vec![0_u64; vector.len().div_ceil(BITS_PER_WORD)];
    for (index, value) in vector.iter().enumerate() {
        if *value > 0.0 {
            words[index / BITS_PER_WORD] |= 1_u64 << (index % BITS_PER_WORD);
        }
    }
    words
}

/// Reads a whole `codes.bin`: `count` codes of `dims` dimensions, one
/// little-endian `u64` per word, code after code.
///
/// The file is read word by word through a buffered reader, so the peak memory
/// is the returned array itself: the budget of `docs/02-architecture.md:208` is
/// that of the codes, never twice that.
///
/// A length that is not exactly [`codes_len`] is an [`ErrorCode::Internal`]
/// error naming the file and both lengths (`ADR-0069`, decision 3): a truncated
/// or oversized file is a corruption, never a derived file to rebuild.
pub fn read_codes_bin(path: &Path, count: u32, dims: u32) -> Result<Vec<u64>, CribleError> {
    let expected = codes_len(count, dims);
    let file =
        File::open(path).map_err(|error| io_error("open the binary codes file", path, &error))?;
    let actual = file
        .metadata()
        .map_err(|error| io_error("inspect the binary codes file", path, &error))?
        .len();
    verify_codes_len(path, count, dims, actual)?;
    let words = usize::try_from(expected / BYTES_PER_WORD).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "{} holds more code words than this platform can address",
                path.display()
            ),
        )
    })?;
    let mut codes = Vec::with_capacity(words);
    let mut reader = BufReader::new(file);
    let mut word = [0_u8; BYTES_PER_WORD as usize];
    for _ in 0..words {
        reader
            .read_exact(&mut word)
            .map_err(|error| io_error("read the binary codes file", path, &error))?;
        codes.push(u64::from_le_bytes(word));
    }
    Ok(codes)
}

/// Checks that a present `codes.bin` holds exactly [`codes_len`] bytes.
///
/// The reader open uses it before lazily loading the codes: a truncated or
/// oversized derived file is a corruption and must be refused when the segment
/// is opened, even though the array itself is only built on first use
/// (`ADR-0069`, decision 3).
pub(crate) fn verify_codes_len(
    path: &Path,
    count: u32,
    dims: u32,
    actual: u64,
) -> Result<(), CribleError> {
    let expected = codes_len(count, dims);
    if actual == expected {
        return Ok(());
    }
    Err(wrong_length(path, count, dims, expected, actual))
}

/// Loads the codes of one segment for a snapshot.
///
/// This is the only loader of the store: the reader of a segment and the
/// snapshot both call it, so the choice "`codes.bin` present, read it; absent,
/// quantize" is written once, here.
///
/// When `codes.bin` is present the file is read directly and the vector file is
/// never opened: a snapshot does not pay the blake3 read of
/// [`SegmentReader::open`]. A missing file is quantized from the reference
/// vectors, in RAM (`ADR-0069`, decision 2): only an index written outside this
/// build, or a restored one that a write open has not rebuilt yet, reaches that
/// path. The quantized array is returned directly, never copied out of a reader,
/// so the fallback holds one array in memory and not two.
pub(crate) fn load_codes(
    segment_dir: &Path,
    meta: &segment::SegmentMeta,
) -> Result<Vec<u64>, CribleError> {
    let path = segment_dir.join(CODES_FILE_NAME);
    match fs::metadata(&path) {
        Ok(_) => read_codes_bin(&path, meta.count, meta.dims),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            SegmentReader::open(segment_dir)?.quantize_vectors()
        }
        Err(error) => Err(io_error("inspect the binary codes file", &path, &error)),
    }
}

/// Ensures the segment at `segment_dir` holds its `codes.bin`, and returns its
/// byte length.
///
/// A file of the expected length is left untouched. A missing one is rebuilt
/// from the reference vectors of the segment, written as `tmp-codes.bin`,
/// fsynced, renamed, then the directory is fsynced (`ADR-0069`, decision 1). A
/// file of another length is refused with [`ErrorCode::Internal`], naming it
/// (`ADR-0069`, decision 3).
///
/// This is the reconstruction the writer open runs: it creates neither a
/// generation nor an epoch, and it does not change a single result, since the
/// file is the quantized form of the very vectors it is built from.
///
/// With the `failpoints` cargo feature, and only with it, the rebuild stops at
/// `CRIBLE_FAILPOINT=codes_rebuild_before_rename`: the temporary file is written
/// and the rename has not happened, which is the crash ADR-0069 announces. The
/// panic is the expected outcome of the test of that crash; the next recovery
/// removes the temporary file (see `remove_temporary_codes`).
pub fn rebuild_codes(segment_dir: &Path) -> Result<u64, CribleError> {
    let meta = segment::read_meta(segment_dir)?;
    let expected = codes_len(meta.count, meta.dims);
    let path = segment_dir.join(CODES_FILE_NAME);
    let temporary = segment_dir.join(format!("{TEMPORARY_PREFIX}{CODES_FILE_NAME}"));
    // A rebuild interrupted between its write and its rename leaves this file
    // behind; the recovery of the epoch removes it too, for every referenced
    // segment, but a rebuild that runs first must not write next to it.
    remove_temporary_file(&temporary)?;
    match fs::metadata(&path) {
        Ok(metadata) => {
            let actual = metadata.len();
            if actual != expected {
                return Err(wrong_length(&path, meta.count, meta.dims, expected, actual));
            }
            Ok(expected)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            // The read-only open builds the codes in RAM and writes nothing, so
            // the writer is the one that persists them here.
            let reader = SegmentReader::open(segment_dir)?;
            write_codes_file(&temporary, reader.codes_words()?)?;
            // The crash test of ADR-0069 stops here: the file is written, the
            // rename has not happened, so the segment holds a `tmp-codes.bin`.
            stop_at_failpoint(CODES_REBUILD_FAILPOINT);
            fs::rename(&temporary, &path).map_err(|error| {
                io_error("rename the rebuilt binary codes file", &temporary, &error)
            })?;
            sync_directory(segment_dir)?;
            Ok(expected)
        }
        Err(error) => Err(io_error("inspect the binary codes file", &path, &error)),
    }
}

/// Removes the temporary file an interrupted rebuild of `segment_dir` left,
/// ignoring an absent one.
///
/// The recovery of the epoch removes every `tmp-*` entry of `segments/`,
/// `manifests/` and the index directory, but a `tmp-codes.bin` lives **inside**
/// a segment directory, which that sweep does not enter. [`rebuild_codes`] calls
/// this before writing, and [`crate::generation`] recovery calls it for every
/// referenced segment, so an interrupted rebuild leaves nothing behind even when
/// no later rebuild passes.
pub(crate) fn remove_temporary_codes(segment_dir: &Path) -> Result<(), CribleError> {
    remove_temporary_file(&segment_dir.join(format!("{TEMPORARY_PREFIX}{CODES_FILE_NAME}")))
}

/// Removes the temporary file of a rebuild, ignoring an absent one.
fn remove_temporary_file(path: &Path) -> Result<(), CribleError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(
            "remove the interrupted binary codes file",
            path,
            &error,
        )),
    }
}

/// Writes `words` as `codes.bin` bytes into `path` and fsyncs the file.
///
/// The words are encoded one by one, little-endian, so the file never has to be
/// built twice in memory.
pub(crate) fn write_codes_file(path: &Path, words: &[u64]) -> Result<(), CribleError> {
    let mut file = File::create(path)
        .map_err(|error| io_error("create the binary codes file", path, &error))?;
    {
        let mut writer = BufWriter::new(&mut file);
        for word in words {
            writer
                .write_all(&word.to_le_bytes())
                .map_err(|error| io_error("write the binary codes file", path, &error))?;
        }
        writer
            .flush()
            .map_err(|error| io_error("flush the binary codes file", path, &error))?;
    }
    file.sync_all()
        .map_err(|error| io_error("fsync the binary codes file", path, &error))
}

/// Builds the error of a `codes.bin` whose length is not the declared one.
fn wrong_length(path: &Path, count: u32, dims: u32, expected: u64, actual: u64) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!(
            "{} is {actual} bytes long, but {count} codes of {dims} dimensions hold \
             {expected} bytes ({} words of {BYTES_PER_WORD} bytes each); a binary-code \
             file of the wrong length is a corruption, not a derived file to rebuild",
            path.display(),
            words_per_code(dims)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{CODES_FILE_NAME, codes_len, quantize, words_per_code};
    use crible_core::error::ErrorCode;
    use std::fs;

    /// A code is padded to whole `u64` words, and its size is total for any
    /// dimension count of a `u32`.
    #[test]
    fn inv_15_word_count_rounds_up_and_length_is_total() {
        assert_eq!(words_per_code(0), 0);
        assert_eq!(words_per_code(1), 1);
        assert_eq!(words_per_code(768), 12);
        assert_eq!(codes_len(u32::MAX, u32::MAX), 2_305_843_008_676_823_040);
    }

    /// The padding bits of a code above its dimensions stay zero: a popcount
    /// over the last word never counts garbage.
    #[test]
    fn quantize_leaves_padding_bits_zero() {
        let mut vector = vec![0.0_f32; 65];
        vector[0] = 1.0;
        vector[64] = -1.0;
        assert_eq!(quantize(&vector), vec![1, 0]);
        assert_eq!(quantize(&[]), Vec::<u64>::new());
    }

    /// A truncated file is refused as an internal state error naming it.
    #[test]
    fn read_refuses_a_truncated_file() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join(CODES_FILE_NAME);
        fs::write(&path, [0_u8; 7]).expect("the truncated file is written");

        let error =
            super::read_codes_bin(&path, 1, 3).expect_err("a truncated codes.bin must be refused");
        assert_eq!(error.code(), ErrorCode::Internal);
        assert!(error.message().contains("codes.bin"));
    }
}
