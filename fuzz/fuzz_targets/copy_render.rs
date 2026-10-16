#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| qry_core::fuzz::fuzz_copy_render(data));
