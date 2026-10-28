// Round-trip and robustness tests for the packed history block and the
// Gorilla coder underneath it. Randomised cases use a fixed seed so a
// failure reproduces.

use crate::compression::{
    GorillaBlock, MAX_BLOCK_SAMPLES, decode_series, encode_series, gorilla_decode, gorilla_encode,
    series_len,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn assert_roundtrip(samples: &[(i64, f64)]) -> usize {
    let packed = encode_series(samples).expect("encode");
    let back = decode_series(&packed).expect("decode");
    assert_eq!(back.len(), samples.len(), "sample count");
    assert_eq!(series_len(&packed).unwrap(), samples.len());
    for (i, (a, b)) in back.iter().zip(samples).enumerate() {
        assert_eq!(a.0, b.0, "timestamp {i}");
        assert_eq!(a.1.to_bits(), b.1.to_bits(), "value bits {i}");
    }
    packed.len()
}

#[test]
fn single_sample_blocks() {
    for v in [
        0.0,
        1.0,
        -0.0,
        42.5,
        f64::NAN,
        f64::INFINITY,
        f64::MIN_POSITIVE / 2.0,
    ] {
        assert_roundtrip(&[(1_700_000_000, v)]);
    }
    assert_roundtrip(&[(i64::MIN, 1.0)]);
    assert_roundtrip(&[(i64::MAX, -1.0)]);
}

#[test]
fn special_float_values_keep_their_bits() {
    let nan_payload = f64::from_bits(0x7FF8_0000_DEAD_BEEF);
    let neg_nan = f64::from_bits(0xFFF8_0000_0000_0001);
    let values = [
        0.0,
        -0.0,
        f64::NAN,
        nan_payload,
        neg_nan,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::from_bits(1), // smallest subnormal
        f64::EPSILON,
        1.0,
        1.0,
        -1.0,
    ];
    let samples: Vec<(i64, f64)> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| (1_000 + i as i64, v))
        .collect();
    assert_roundtrip(&samples);
    // Every value alone after the first, so each starts a fresh XOR chain.
    for pair in values.windows(2) {
        assert_roundtrip(&[(0, pair[0]), (1, pair[1])]);
    }
}

#[test]
fn equal_values_use_the_constant_codec() {
    let samples: Vec<(i64, f64)> = (0..3600).map(|i| (1_700_000_000 + i, 21.5)).collect();
    let size = assert_roundtrip(&samples);
    assert!(size < 40, "3600 identical samples took {size} bytes");

    let nan: Vec<(i64, f64)> = (0..100).map(|i| (i, f64::NAN)).collect();
    assert_roundtrip(&nan);
    // +0.0 and -0.0 are different bit patterns and must not collapse.
    assert_roundtrip(&[(0, 0.0), (1, -0.0), (2, 0.0)]);
}

#[test]
fn duplicate_and_out_of_order_timestamps() {
    assert_roundtrip(&[(10, 1.0), (10, 2.0), (10, 3.0), (10, 3.0)]);
    assert_roundtrip(&[
        (100, 1.5),
        (50, 2.5),
        (200, 3.5),
        (-30, 4.5),
        (200, 5.5),
        (0, 0.5),
    ]);
    assert_roundtrip(&[(5, 1.0), (4, 1.0), (3, 0.0), (2, 1.0), (1, 0.0)]);
}

#[test]
fn extreme_timestamps_wrap_without_loss() {
    assert_roundtrip(&[(i64::MIN, 1.0), (i64::MAX, 2.0), (0, 3.0), (i64::MAX, 4.0)]);
    assert_roundtrip(&[(i64::MAX - 2, 1.0), (i64::MAX - 1, 2.0), (i64::MAX, 3.0)]);
    assert_roundtrip(&[(i64::MIN, 1.0), (i64::MIN + 1, 2.0), (i64::MIN + 2, 2.5)]);
}

#[test]
fn digital_signals_pack_to_run_lengths() {
    // A pump that runs 8 h, stops 1 h, runs again, polled every second.
    let mut samples = Vec::new();
    let mut ts = 1_700_000_000i64;
    for (value, secs) in [(1.0, 28_800), (0.0, 3_600), (1.0, 900)] {
        for _ in 0..secs {
            samples.push((ts, value));
            ts += 1;
        }
    }
    // Pack in hour blocks like the historian does.
    let total: usize = samples.chunks(3600).map(assert_roundtrip).sum();
    assert!(
        total < 13 * 40,
        "a day of one digital tag took {total} bytes over 13 blocks"
    );

    // One long block as well: the whole 37 500 samples stay tiny.
    let one = assert_roundtrip(&samples);
    assert!(one < 100, "single block took {one} bytes");

    // Toggling every sample is the worst case; it must still round-trip.
    let toggling: Vec<(i64, f64)> = (0..2000).map(|i| (i, (i % 2) as f64)).collect();
    assert_roundtrip(&toggling);
    // Starting on 1.0 and with the last run of length one.
    assert_roundtrip(&[(0, 1.0), (1, 1.0), (2, 0.0)]);
    assert_roundtrip(&[(0, 1.0), (1, 0.0)]);
}

#[test]
fn zero_run_token_boundaries() {
    // Regular polling produces dod == 0; runs are coded as tokens from 23 up
    // and split at 65 536. Cover both sides of each threshold.
    for n in [
        2, 3, 22, 23, 24, 25, 100, 65_536, 65_537, 65_538, 131_072, 131_100, 200_000,
    ] {
        let samples: Vec<(i64, f64)> = (0..n)
            .map(|i| (1_000_000 + i * 5, (i % 7) as f64 * 0.25))
            .collect();
        assert_roundtrip(&samples);
    }
    // A regular series with one jitter in the middle of a run.
    let mut samples: Vec<(i64, f64)> = (0..500).map(|i| (i * 10, i as f64)).collect();
    for s in samples.iter_mut().skip(250) {
        s.0 += 3;
    }
    assert_roundtrip(&samples);
}

#[test]
fn steady_polling_costs_almost_nothing_for_timestamps() {
    let samples: Vec<(i64, f64)> = (0..86_400)
        .map(|i| (i, 7.0 + (i % 2) as f64 * 0.5))
        .collect();
    let size = assert_roundtrip(&samples);
    // 86 400 samples of a two-level signal: timestamps are a handful of run
    // tokens; each value costs 3 bits (changed, reuse window, 1 payload bit).
    assert!(size <= 86_400 * 3 / 8 + 64, "took {size} bytes");
}

#[test]
fn every_dod_width_roundtrips() {
    // dods around each prefix boundary: 63/64, 255/256, 2047/2048, 2^19 edges.
    let mut ts = 0i64;
    let mut samples = vec![(ts, 0.0)];
    let mut delta = 10i64;
    for dod in [
        1,
        -1,
        63,
        -63,
        64,
        -64,
        65,
        -65,
        255,
        -255,
        256,
        -256,
        257,
        2047,
        -2047,
        2048,
        -2048,
        2049,
        524_287,
        -524_287,
        524_288,
        -524_288,
        524_289,
        1 << 40,
        -(1 << 40),
        0,
        0,
    ] {
        delta += dod;
        ts += delta;
        samples.push((ts, samples.len() as f64));
    }
    assert_roundtrip(&samples);
}

fn random_timestamps(rng: &mut StdRng, n: usize) -> Vec<i64> {
    let mut ts = rng.random_range(-1_000_000_000i64..4_000_000_000);
    let mut out = Vec::with_capacity(n);
    let style = rng.random_range(0..4);
    let step = rng.random_range(1..90);
    for _ in 0..n {
        out.push(ts);
        ts += match style {
            0 => step,                          // regular
            1 => rng.random_range(0..3) * step, // duplicates and jitter
            2 => rng.random_range(-50i64..200), // slightly out of order
            _ => {
                if rng.random_bool(0.05) {
                    rng.random_range(1..5_000_000) // big gaps
                } else {
                    step
                }
            }
        };
    }
    out
}

fn random_values(rng: &mut StdRng, n: usize) -> Vec<f64> {
    match rng.random_range(0..7) {
        // Pure noise over the whole bit space, NaNs and infinities included.
        0 => (0..n)
            .map(|_| f64::from_bits(rng.random::<u64>()))
            .collect(),
        // Slow analog signal with noise.
        1 => {
            let mut x = rng.random_range(-100.0..100.0);
            (0..n)
                .map(|_| {
                    x += rng.random_range(-0.05..0.05);
                    x
                })
                .collect()
        }
        // Steps between a few levels.
        2 => {
            let levels = [0.0, 1.5, -3.25, 1e9, f64::INFINITY];
            let mut cur = 0usize;
            (0..n)
                .map(|_| {
                    if rng.random_bool(0.1) {
                        cur = rng.random_range(0..levels.len());
                    }
                    levels[cur]
                })
                .collect()
        }
        // Boolean.
        3 => {
            let mut cur = rng.random_bool(0.5);
            (0..n)
                .map(|_| {
                    if rng.random_bool(0.07) {
                        cur = !cur;
                    }
                    if cur { 1.0 } else { 0.0 }
                })
                .collect()
        }
        // Counter.
        4 => {
            let mut c = rng.random_range(0..1_000_000) as f64;
            (0..n)
                .map(|_| {
                    c += rng.random_range(0..4) as f64;
                    c
                })
                .collect()
        }
        // Constant.
        5 => vec![rng.random_range(-10.0..10.0); n],
        // Mixed zero/one with an occasional intruder.
        _ => (0..n)
            .map(|_| match rng.random_range(0..20) {
                0 => 2.0,
                1..=10 => 1.0,
                _ => 0.0,
            })
            .collect(),
    }
}

#[test]
fn random_blocks_roundtrip_bit_exactly() {
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    for _ in 0..1500 {
        let n = match rng.random_range(0..4) {
            0 => rng.random_range(1..4),
            1 => rng.random_range(1..60),
            2 => rng.random_range(1..600),
            _ => rng.random_range(1..4000),
        };
        let ts = random_timestamps(&mut rng, n);
        let vals = random_values(&mut rng, n);
        let samples: Vec<(i64, f64)> = ts.into_iter().zip(vals).collect();
        assert_roundtrip(&samples);
    }
}

#[test]
fn gorilla_coder_roundtrips_random_input() {
    // The raw coder, not only the block wrapper. The window-reuse path (new
    // xor fits inside the previous leading/trailing zero window) is hit by
    // slowly drifting values.
    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..800 {
        let n = rng.random_range(1..500);
        let ts = random_timestamps(&mut rng, n);
        let vals = random_values(&mut rng, n);
        let block = gorilla_encode(&ts, &vals).unwrap();
        let (dts, dvals) = gorilla_decode(&block).unwrap();
        assert_eq!(dts, ts);
        assert!(
            dvals
                .iter()
                .zip(&vals)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }
}

#[test]
fn gorilla_window_reuse_with_a_narrower_xor() {
    // Second xor has fewer meaningful bits than the first but sits inside
    // its window: the reuse branch must write the *window* width.
    let vals = [1.0, 3.0, 2.0, 2.5, 2.25, 2.125, 2.0625];
    let ts: Vec<i64> = (0..vals.len() as i64).collect();
    let block = gorilla_encode(&ts, &vals).unwrap();
    let (_, back) = gorilla_decode(&block).unwrap();
    assert_eq!(back, vals);
}

#[test]
fn gorilla_rejects_bad_input() {
    assert!(gorilla_encode(&[], &[]).is_err());
    assert!(gorilla_encode(&[1, 2], &[1.0]).is_err());
    let block = GorillaBlock {
        first_ts: 0,
        count: 0,
        first_value: 0.0,
        data: vec![],
    };
    assert!(gorilla_decode(&block).is_err());
    let too_many = GorillaBlock {
        first_ts: 0,
        count: MAX_BLOCK_SAMPLES + 1,
        first_value: 0.0,
        data: vec![0; 8],
    };
    assert!(gorilla_decode(&too_many).is_err());
    assert!(encode_series(&[]).is_err());
}

#[test]
fn corrupt_blocks_fail_cleanly() {
    let mut rng = StdRng::seed_from_u64(99);
    for _ in 0..600 {
        let n = rng.random_range(2..300);
        let ts = random_timestamps(&mut rng, n);
        let vals = random_values(&mut rng, n);
        let samples: Vec<(i64, f64)> = ts.into_iter().zip(vals).collect();
        let packed = encode_series(&samples).unwrap();

        // Truncation: must error (the payload is incomplete), never panic.
        for cut in [0, 1, 2, packed.len() / 2, packed.len().saturating_sub(1)] {
            let _ = decode_series(&packed[..cut.min(packed.len())]);
        }
        // Bit flips: either an error or some other sample list, never a
        // panic and never an allocation beyond the block limit.
        let mut flipped = packed.clone();
        for _ in 0..rng.random_range(1..4) {
            let i = rng.random_range(0..flipped.len());
            flipped[i] ^= 1 << rng.random_range(0..8);
        }
        if let Ok(back) = decode_series(&flipped) {
            assert!(back.len() <= MAX_BLOCK_SAMPLES);
        }
    }
    // A header that claims a huge count.
    let mut huge = vec![0x10u8];
    huge.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]); // count ~ 4.29e9
    huge.extend_from_slice(&[0; 9]);
    assert!(decode_series(&huge).is_err());
    // Unknown format version.
    assert!(decode_series(&[0xF0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
}

#[test]
fn blocks_beyond_the_limit_are_refused() {
    let samples = vec![(0i64, 0.0f64); MAX_BLOCK_SAMPLES + 1];
    assert!(encode_series(&samples).is_err());
    assert!(encode_series(&samples[..MAX_BLOCK_SAMPLES]).is_ok());
}
