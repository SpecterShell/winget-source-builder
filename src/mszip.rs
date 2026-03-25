use anyhow::{Context, Result, ensure};
use crc32fast::Hasher as Crc32Hasher;
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

const HEADER_SIZE: usize = 24;
const CHUNK_HEADER_SIZE: usize = 6;
const MAX_CHUNK_SIZE: usize = 1 << 15;
const CHUNK_PADDING: u16 = 0x4B43;
const ALGORITHM_MSZIP: u8 = 2;
const MAGIC_BYTES: [u8; 6] = [10, 81, 229, 192, 24, 0];

pub(crate) fn compress_all(decompressed: &[u8]) -> Result<Vec<u8>> {
    let mut compressed = Vec::new();
    let first_chunk_decompressed_length = decompressed.len().min(MAX_CHUNK_SIZE) as u64;

    let mut header = Vec::with_capacity(HEADER_SIZE);
    header.extend_from_slice(&MAGIC_BYTES);
    header.push(0);
    header.push(ALGORITHM_MSZIP);
    header.extend_from_slice(&(decompressed.len() as u64).to_le_bytes());
    header.extend_from_slice(&first_chunk_decompressed_length.to_le_bytes());
    header[6] = header_crc(&header);
    compressed.extend_from_slice(&header);

    let mut dictionary = Vec::new();
    for chunk in decompressed.chunks(MAX_CHUNK_SIZE) {
        let chunk_compressed = compress_chunk(chunk, &dictionary)?;
        compressed.extend_from_slice(&((chunk_compressed.len() + 2) as u32).to_le_bytes());
        compressed.extend_from_slice(&CHUNK_PADDING.to_le_bytes());
        compressed.extend_from_slice(&chunk_compressed);
        update_dictionary(&mut dictionary, chunk);
    }

    Ok(compressed)
}

pub(crate) fn decompress_all(compressed: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        compressed.len() >= HEADER_SIZE,
        "MSZIP payload is too small for the fixed header"
    );
    ensure!(
        compressed[..MAGIC_BYTES.len()] == MAGIC_BYTES,
        "MSZIP payload has invalid magic bytes"
    );
    ensure!(
        compressed[7] == ALGORITHM_MSZIP,
        "MSZIP payload uses unsupported algorithm {}",
        compressed[7]
    );
    ensure!(
        compressed[6] == header_crc(&compressed[..HEADER_SIZE]),
        "MSZIP payload header CRC does not match"
    );

    let decompressed_length = read_u64_le(&compressed[8..16]) as usize;
    let first_chunk_decompressed_length = read_u64_le(&compressed[16..24]) as usize;

    let mut offset = HEADER_SIZE;
    let mut decompressed = Vec::with_capacity(decompressed_length);
    let mut dictionary = Vec::new();

    while offset < compressed.len() {
        ensure!(
            compressed.len() - offset >= CHUNK_HEADER_SIZE,
            "MSZIP chunk header is truncated"
        );

        let compressed_chunk_size = read_u32_le(&compressed[offset..offset + 4]) as usize;
        let padding = read_u16_le(&compressed[offset + 4..offset + 6]);
        ensure!(
            padding == CHUNK_PADDING,
            "MSZIP chunk padding is invalid: expected 0x{CHUNK_PADDING:04X}, got 0x{padding:04X}"
        );
        ensure!(
            compressed_chunk_size >= 2,
            "MSZIP chunk size is smaller than the padding length"
        );

        offset += CHUNK_HEADER_SIZE;
        let chunk_size = compressed_chunk_size - 2;
        ensure!(
            compressed.len() - offset >= chunk_size,
            "MSZIP chunk payload is truncated"
        );

        let chunk = &compressed[offset..offset + chunk_size];
        let chunk_decompressed = decompress_chunk(chunk, &dictionary)?;
        if decompressed.is_empty() {
            ensure!(
                chunk_decompressed.len() == first_chunk_decompressed_length,
                "MSZIP first chunk decompressed to {} bytes, expected {}",
                chunk_decompressed.len(),
                first_chunk_decompressed_length
            );
        }

        decompressed.extend_from_slice(&chunk_decompressed);
        update_dictionary(&mut dictionary, &chunk_decompressed);
        offset += chunk_size;
    }

    ensure!(
        decompressed.len() == decompressed_length,
        "MSZIP payload decompressed to {} bytes, expected {}",
        decompressed.len(),
        decompressed_length
    );

    Ok(decompressed)
}

fn compress_chunk(chunk: &[u8], dictionary: &[u8]) -> Result<Vec<u8>> {
    let mut compressor = Compress::new(Compression::best(), false);
    if !dictionary.is_empty() {
        compressor
            .set_dictionary(dictionary)
            .context("failed to set MSZIP compression dictionary")?;
    }

    let mut input_offset = 0usize;
    let mut compressed = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        let previous_in = compressor.total_in();
        let previous_out = compressor.total_out();
        let status = compressor
            .compress(&chunk[input_offset..], &mut buffer, FlushCompress::Finish)
            .context("MSZIP chunk compression failed")?;
        input_offset = compressor.total_in() as usize;
        let written = (compressor.total_out() - previous_out) as usize;
        compressed.extend_from_slice(&buffer[..written]);

        if status == Status::StreamEnd {
            break;
        }

        ensure!(
            compressor.total_in() != previous_in || compressor.total_out() != previous_out,
            "MSZIP chunk compression made no progress"
        );
    }

    Ok(compressed)
}

fn decompress_chunk(chunk: &[u8], dictionary: &[u8]) -> Result<Vec<u8>> {
    let mut decompressor = Decompress::new(false);
    if !dictionary.is_empty() {
        decompressor
            .set_dictionary(dictionary)
            .context("failed to set MSZIP decompression dictionary")?;
    }

    let mut input_offset = 0usize;
    let mut decompressed = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        let previous_in = decompressor.total_in();
        let previous_out = decompressor.total_out();
        let status = decompressor
            .decompress(&chunk[input_offset..], &mut buffer, FlushDecompress::Finish)
            .context("MSZIP chunk decompression failed")?;
        input_offset = decompressor.total_in() as usize;
        let written = (decompressor.total_out() - previous_out) as usize;
        decompressed.extend_from_slice(&buffer[..written]);

        if status == Status::StreamEnd {
            break;
        }

        ensure!(
            decompressor.total_in() != previous_in || decompressor.total_out() != previous_out,
            "MSZIP chunk decompression made no progress"
        );
    }

    Ok(decompressed)
}

fn update_dictionary(dictionary: &mut Vec<u8>, new_bytes: &[u8]) {
    if new_bytes.len() >= MAX_CHUNK_SIZE {
        dictionary.clear();
        dictionary.extend_from_slice(&new_bytes[new_bytes.len() - MAX_CHUNK_SIZE..]);
        return;
    }

    dictionary.extend_from_slice(new_bytes);
    if dictionary.len() > MAX_CHUNK_SIZE {
        let to_drop = dictionary.len() - MAX_CHUNK_SIZE;
        dictionary.drain(..to_drop);
    }
}

fn header_crc(header: &[u8]) -> u8 {
    let mut crc = Crc32Hasher::new();
    crc.update(&header[..6]);
    crc.update(&header[7..24]);
    (crc.finalize() & 0xFF) as u8
}

fn read_u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("slice size should be validated"))
}

fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("slice size should be validated"))
}

fn read_u64_le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("slice size should be validated"))
}

#[cfg(test)]
mod tests {
    use super::{HEADER_SIZE, MAGIC_BYTES, MAX_CHUNK_SIZE, compress_all, decompress_all};

    #[test]
    fn round_trips_small_payload() {
        let payload = b"sV: 1.0\nvD:\n  - v: 1.0.0\n    rP: manifests/e/Example/App/1.0.0/test.yaml\n    s256H: abcdef\n";
        let compressed = compress_all(payload).unwrap();
        let decompressed = decompress_all(&compressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn round_trips_multiple_chunks() {
        let payload = (0..(MAX_CHUNK_SIZE * 3 + 1234))
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let compressed = compress_all(&payload).unwrap();
        let decompressed = decompress_all(&compressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn writes_expected_mszip_header_prefix() {
        let compressed = compress_all(b"hello world").unwrap();
        assert!(compressed.len() > HEADER_SIZE);
        assert_eq!(&compressed[..MAGIC_BYTES.len()], &MAGIC_BYTES);
        assert_eq!(compressed[7], 2);
    }
}
