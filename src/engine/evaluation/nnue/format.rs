use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use super::{ACTIVATION_MAX, HIDDEN_SIZE, INPUT_FEATURES, Network, OUTPUT_BUCKETS, OUTPUT_SCALE};

/// Magic bytes of a Jakgro network; this is not a Stockfish network format.
pub const MAGIC: [u8; 8] = *b"JAKNNUE\0";
/// Version of the fixed feature mapping, tensor layout and quantization.
pub const FORMAT_VERSION: u32 = 2;
/// Fixed little-endian header size, including the payload checksum.
pub const HEADER_BYTES: usize = 48;
const FEATURE_SET: u32 = 1;
const PAYLOAD_BYTES: usize = 2
    * (HIDDEN_SIZE + INPUT_FEATURES * HIDDEN_SIZE + OUTPUT_BUCKETS * 2 * HIDDEN_SIZE)
    + 4 * OUTPUT_BUCKETS;
/// Exact file length accepted by this fixed architecture.
pub const FILE_BYTES: usize = HEADER_BYTES + PAYLOAD_BYTES;

/// A rejected file never constructs a partially usable network.
#[derive(Debug)]
pub enum LoadError {
    /// File or stream I/O failed, including unexpected EOF.
    Io(io::Error),
    /// Filesystem loading accepts only regular files.
    NotRegularFile,
    /// A byte slice or file has the wrong length.
    Length { expected: usize, actual: u64 },
    /// Header fields do not describe the supported architecture.
    Header(&'static str),
    /// The payload's FNV-1a checksum does not match its header.
    Checksum,
    /// Output arithmetic cannot be proven safe in signed 32-bit integers.
    OutputOverflow,
    /// The fixed-size allocation failed.
    Allocation,
    /// The stream contains bytes after the complete model.
    TrailingData,
}

impl Display for LoadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "NNUE I/O error: {error}"),
            Self::NotRegularFile => formatter.write_str("NNUE input is not a regular file"),
            Self::Length { expected, actual } => {
                write!(formatter, "NNUE length {actual}, expected {expected}")
            }
            Self::Header(field) => write!(formatter, "unsupported NNUE header: {field}"),
            Self::Checksum => formatter.write_str("NNUE payload checksum mismatch"),
            Self::OutputOverflow => {
                formatter.write_str("NNUE output arithmetic exceeds i32 bounds")
            }
            Self::Allocation => formatter.write_str("unable to allocate the fixed NNUE model"),
            Self::TrailingData => formatter.write_str("trailing data after NNUE model"),
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for LoadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Network {
    /// Loads a regular file of exactly [`FILE_BYTES`] bytes.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(LoadError::NotRegularFile);
        }
        check_length(metadata.len())?;
        Self::read_from(file)
    }

    /// Decodes an exact-length file image without trusting its dimensions.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        check_length(bytes.len() as u64)?;
        let header: &[u8; HEADER_BYTES] = bytes[..HEADER_BYTES].try_into().unwrap();
        validate_header(header)?;
        decode(header, &bytes[HEADER_BYTES..])
    }

    /// Reads at most [`FILE_BYTES`] + 1 bytes, rejecting extra stream data.
    ///
    /// Header dimensions must match the fixed architecture before allocation.
    /// The memory and byte-read bounds do not impose an I/O time limit.
    pub fn read_from(mut reader: impl Read) -> Result<Self, LoadError> {
        let mut header = [0_u8; HEADER_BYTES];
        reader.read_exact(&mut header)?;
        validate_header(&header)?;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(PAYLOAD_BYTES)
            .map_err(|_| LoadError::Allocation)?;
        payload.resize(PAYLOAD_BYTES, 0);
        reader.read_exact(&mut payload)?;
        let mut extra = [0_u8; 1];
        match reader.read_exact(&mut extra) {
            Ok(()) => return Err(LoadError::TrailingData),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
            Err(error) => return Err(error.into()),
        }
        decode(&header, &payload)
    }
}

fn check_length(actual: u64) -> Result<(), LoadError> {
    if actual != FILE_BYTES as u64 {
        return Err(LoadError::Length {
            expected: FILE_BYTES,
            actual,
        });
    }
    Ok(())
}

fn validate_header(header: &[u8; HEADER_BYTES]) -> Result<(), LoadError> {
    if header[..8] != MAGIC {
        return Err(LoadError::Header("magic"));
    }
    // Eight fixed fields. Payload dimensions can never request a larger
    // allocation or a different feature contract.
    for (index, (expected, name)) in [
        (FORMAT_VERSION, "version"),
        (FEATURE_SET, "feature set"),
        (INPUT_FEATURES as u32, "input features"),
        (HIDDEN_SIZE as u32, "hidden size"),
        (ACTIVATION_MAX as u32, "activation scale"),
        (OUTPUT_SCALE as u32, "output scale"),
        (PAYLOAD_BYTES as u32, "payload length"),
        (OUTPUT_BUCKETS as u32, "output buckets"),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 8 + index * 4;
        let actual = u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
        if actual != expected {
            return Err(LoadError::Header(name));
        }
    }
    Ok(())
}

fn decode(header: &[u8; HEADER_BYTES], payload: &[u8]) -> Result<Network, LoadError> {
    let expected = u64::from_le_bytes(header[40..48].try_into().unwrap());
    if checksum(payload) != expected {
        return Err(LoadError::Checksum);
    }
    // Tensor order: hidden bias, feature-major input rows, then per output
    // bucket two output rows, then one i32 bias per bucket. All weights and
    // hidden biases are signed little-endian i16.
    let output_start = 2 * (HIDDEN_SIZE + INPUT_FEATURES * HIDDEN_SIZE);
    let bias_start = output_start + 2 * OUTPUT_BUCKETS * 2 * HIDDEN_SIZE;
    let output_weights: [[[i16; HIDDEN_SIZE]; 2]; OUTPUT_BUCKETS] = std::array::from_fn(|bucket| {
        std::array::from_fn(|side| {
            std::array::from_fn(|unit| {
                read_i16(
                    payload,
                    output_start + 2 * ((bucket * 2 + side) * HIDDEN_SIZE + unit),
                )
            })
        })
    });
    let output_bias: [i32; OUTPUT_BUCKETS] = std::array::from_fn(|bucket| {
        let offset = bias_start + 4 * bucket;
        i32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
    });
    for (weights, &bias) in output_weights.iter().zip(&output_bias) {
        let bound = i64::from(bias).abs()
            + i64::from(ACTIVATION_MAX)
                * weights
                    .iter()
                    .flatten()
                    .map(|&weight| i64::from(weight).abs())
                    .sum::<i64>();
        if bound > i64::from(i32::MAX) {
            return Err(LoadError::OutputOverflow);
        }
    }
    // A board occupies at most 64 squares. With i16 weights and hidden bias,
    // every unclipped sum is bounded by 65 * 32768, well within i32.
    let hidden_bias = std::array::from_fn(|unit| read_i16(payload, 2 * unit));
    let mut input_weights = Vec::new();
    input_weights
        .try_reserve_exact(INPUT_FEATURES * HIDDEN_SIZE)
        .map_err(|_| LoadError::Allocation)?;
    input_weights.extend(
        payload[2 * HIDDEN_SIZE..output_start]
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]])),
    );
    Ok(Network {
        hidden_bias,
        input_weights: input_weights.into_boxed_slice(),
        output_weights,
        output_bias,
    })
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

// FNV-1a detects accidental corruption; it is not authentication or a model ID.
fn checksum(payload: &[u8]) -> u64 {
    payload
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
}
