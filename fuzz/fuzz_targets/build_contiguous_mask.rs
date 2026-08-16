#![no_main]

use cpuhint_core::build_contiguous_mask;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mask_len = data.first().copied().unwrap_or(0) as usize;
    let count = data
        .get(1..9)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0) as usize;

    let mut storage = [0xa5; 255];
    let original = storage;
    let result = build_contiguous_mask(&mut storage[..mask_len], count);
    let fits = count > 0 && count.div_ceil(8) <= mask_len;

    assert_eq!(result.is_ok(), fits);
    if !fits {
        assert_eq!(storage, original);
        return;
    }

    for bit in 0..mask_len * 8 {
        let is_set = storage[bit / 8] & (1 << (bit % 8)) != 0;
        assert_eq!(is_set, bit < count);
    }
    assert_eq!(&storage[mask_len..], &original[mask_len..]);
});
