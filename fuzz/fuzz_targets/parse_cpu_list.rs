#![no_main]

use cpuhint_core::parse_cpu_list;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(parsed) = parse_cpu_list(data) else {
        return;
    };

    assert!(parsed.count() > 0);
    assert!(parsed.count() <= i32::MAX as usize);

    // Leading and trailing ASCII whitespace must not change a valid view.
    let mut padded = Vec::with_capacity(data.len() + 2);
    padded.push(b' ');
    padded.extend_from_slice(data);
    padded.push(b'\n');
    assert_eq!(parse_cpu_list(&padded), Ok(parsed));

    if parsed.is_contiguous_from_zero() {
        let canonical = if parsed.count() == 1 {
            String::from("0")
        } else {
            format!("0-{}", parsed.count() - 1)
        };
        assert_eq!(parse_cpu_list(canonical.as_bytes()), Ok(parsed));
    }
});
