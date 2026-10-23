// src/compression/tests.rs — Integration and unit tests for compression modules

#[cfg(test)]
mod tests {
    use crate::compression::{
        BitReader, BitWriter, gorilla_decode, gorilla_encode, rle_decode, rle_deserialize,
        rle_encode, rle_serialize, varint_decode, varint_encode,
    };

    // -----------------------------------------------------------------------
    // BitWriter / BitReader tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_read_roundtrip_single_bits() {
        let mut writer = BitWriter::new();
        for i in 0..16 {
            writer.write_bit(i % 2 == 0);
        }
        let data = writer.finish();

        let mut reader = BitReader::new(&data);
        for i in 0..16 {
            assert_eq!(reader.read_bit(), Some(i % 2 == 0));
        }
    }

    #[test]
    fn test_write_read_roundtrip_multi_bit() {
        let mut writer = BitWriter::new();
        writer.write_bits(0b1010, 4);
        writer.write_bits(0b11001100, 8);
        writer.write_bits(0b111, 3);
        let data = writer.finish();

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(4), Some(0b1010));
        assert_eq!(reader.read_bits(8), Some(0b11001100));
        assert_eq!(reader.read_bits(3), Some(0b111));
    }

    #[test]
    fn test_write_read_large_value() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xDEADBEEF, 32);
        let data = writer.finish();

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(32), Some(0xDEADBEEF));
    }

    #[test]
    fn test_empty_writer() {
        let writer = BitWriter::new();
        assert!(writer.finish().is_empty());
    }

    #[test]
    fn test_reader_past_end_returns_none() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xFF, 8);
        let data = writer.finish();

        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(8), Some(0xFF));
        assert_eq!(reader.read_bits(1), None);
    }

    // -----------------------------------------------------------------------
    // Gorilla encoder/decoder tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gorilla_encode_decode_roundtrip() {
        // 10 SCADA-like readings: ~1s intervals, small variations, some duplicates
        let timestamps: Vec<i64> = vec![
            1_700_000_000,
            1_700_000_001,
            1_700_000_002,
            1_700_000_003,
            1_700_000_003, // duplicate timestamp (delta-of-delta = -1)
            1_700_000_005, // 2s gap
            1_700_000_006,
            1_700_000_007,
            1_700_000_008,
            1_700_000_009,
        ];
        let values: Vec<f64> = vec![
            23.5, 23.6, 23.6, // duplicate value
            23.7, 23.7, // duplicate value
            23.5, // back to earlier value
            24.0, 24.1, 24.1, // duplicate value
            24.2,
        ];

        let block = gorilla_encode(&timestamps, &values).expect("encode should succeed");
        assert_eq!(block.count, 10);
        assert_eq!(block.first_ts, 1_700_000_000);

        let (dec_ts, dec_vals) = gorilla_decode(&block).expect("decode should succeed");

        assert_eq!(dec_ts, timestamps);
        assert_eq!(dec_vals, values);
    }

    #[test]
    fn test_gorilla_encode_empty_input() {
        let ts: Vec<i64> = vec![];
        let vals: Vec<f64> = vec![];
        assert!(gorilla_encode(&ts, &vals).is_err());

        // Also mismatched lengths
        let ts2 = vec![1i64];
        let vals2: Vec<f64> = vec![];
        assert!(gorilla_encode(&ts2, &vals2).is_err());
    }

    #[test]
    fn test_gorilla_single_value() {
        let timestamps = vec![1234567890i64];
        let values = vec![42.0f64];

        let block = gorilla_encode(&timestamps, &values).expect("encode single value");
        assert_eq!(block.count, 1);
        assert_eq!(block.first_ts, 1234567890);
        assert!(block.data.is_empty());

        let (dec_ts, dec_vals) = gorilla_decode(&block).expect("decode single value");
        assert_eq!(dec_ts, timestamps);
        assert_eq!(dec_vals, values);
    }

    // -----------------------------------------------------------------------
    // RLE encoder/decoder tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rle_encode_decode_roundtrip() {
        // Pump running 8 hours at 1Hz then off 1 hour
        let mut values: Vec<bool> = vec![true; 28800];
        values.extend(vec![false; 3600]);
        let encoded = rle_encode(&values);
        let decoded = rle_decode(&encoded);
        assert_eq!(decoded, values);
        assert!(
            encoded.len() < 20,
            "RLE should compress to <20 bytes, got {}",
            encoded.len()
        );
    }

    #[test]
    fn test_rle_alternating() {
        let values: Vec<bool> = (0..100).map(|i| i % 2 == 0).collect();
        let encoded = rle_encode(&values);
        let decoded = rle_decode(&encoded);
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_rle_single_value() {
        let values = vec![true];
        let encoded = rle_encode(&values);
        let decoded = rle_decode(&encoded);
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_rle_all_same() {
        let values = vec![false; 100000];
        let encoded = rle_encode(&values);
        let decoded = rle_decode(&encoded);
        assert_eq!(decoded, values);
        assert!(encoded.len() < 10);
    }

    #[test]
    fn test_rle_serialize_deserialize_roundtrip() {
        let runs = vec![(true, 28800u32), (false, 3600u32)];
        let data = rle_serialize(&runs);
        let decoded = rle_deserialize(&data).unwrap();
        assert_eq!(decoded, runs);
    }

    // -----------------------------------------------------------------------
    // Varint encoder/decoder tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_varint_roundtrip() {
        let values: Vec<i64> = vec![0, 1, -1, 127, -128, 16383, -16384, 1000000, -1000000];
        let encoded = varint_encode(&values);
        let decoded = varint_decode(&encoded).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_varint_small_values() {
        let values: Vec<i64> = vec![0, 0, 0, 1, 0, -1, 0, 2, -1, 0];
        let encoded = varint_encode(&values);
        assert!(encoded.len() < 20, "Small deltas should be very compact");
        let decoded = varint_decode(&encoded).unwrap();
        assert_eq!(decoded, values);
    }

    // -----------------------------------------------------------------------
    // Compression ratio integration test
    // -----------------------------------------------------------------------

    #[test]
    fn test_gorilla_compression_ratio() {
        // 1 hour of SCADA data (3600 readings) at 1s intervals.
        // Realistic sensor pattern: mostly constant temperature with occasional
        // small step changes. Gorilla encoding excels here because XOR deltas
        // are zero for repeated values and small for nearby values, while
        // constant-rate timestamps compress to ~1 bit each.
        let mut values = Vec::with_capacity(3600);
        let mut current = 25.0f64;
        for i in 0..3600 {
            // Mostly constant, with a small step every ~60s
            if i > 0 && i % 60 == 0 {
                current += 0.1;
            }
            values.push(current);
        }

        let timestamps: Vec<i64> = (0..3600).map(|i| 1700000000000 + i * 1000).collect();
        let block = gorilla_encode(&timestamps, &values).unwrap();
        let raw_size = timestamps.len() * 16; // 8 bytes ts + 8 bytes value
        let compressed_size = block.data.len();
        let ratio = raw_size as f64 / compressed_size as f64;

        assert!(ratio > 5.0, "Compression ratio too low: {:.1}:1", ratio);
    }
}
