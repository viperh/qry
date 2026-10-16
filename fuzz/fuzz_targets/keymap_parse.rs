#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| qry::fuzz::fuzz_keymap_parse(data));
