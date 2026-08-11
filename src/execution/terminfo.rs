pub(super) const NAME: &str = "cargo-xtest";
pub(super) const DIRECTORY: &str = "/cargo-xtest/terminfo";
pub(super) const ENTRY_DIRECTORY: &str = "/cargo-xtest/terminfo/c";
pub(super) const ENTRY_PATH: &str = "/cargo-xtest/terminfo/c/cargo-xtest";

const ENTRY_LEN: usize = 825;
const PREFIX: &[u8] = b"\x1a\x01\x23\x00\x00\x00\x0f\x00\x69\x01\x19\x00\
    cargo-xtest|cargo-xtest ANSI color\x00\x00";
const NUMBERS: &[u8] = b"\x08\x00\x40\x00";
const STRING_OFFSETS: &[u8] = b"\x00\x00";
const STRINGS: &[u8] = b"\x05\x00\x0f\x00\x1b[0m\x00\x1b[3%p1%dm\x00\x1b[4%p1%dm\x00";

// Generated with `tic -x` from `assets/terminfo/cargo-xtest.src`.
const fn build_entry() -> [u8; ENTRY_LEN] {
    let mut entry = [0xff; ENTRY_LEN];
    let mut index = 0;
    while index < PREFIX.len() {
        entry[index] = PREFIX[index];
        index += 1;
    }
    index = 0;
    while index < NUMBERS.len() {
        entry[74 + index] = NUMBERS[index];
        index += 1;
    }
    index = 0;
    while index < STRING_OFFSETS.len() {
        entry[156 + index] = STRING_OFFSETS[index];
        index += 1;
    }
    index = 0;
    while index < STRINGS.len() {
        entry[796 + index] = STRINGS[index];
        index += 1;
    }
    entry
}

const GENERATED_ENTRY: [u8; ENTRY_LEN] = build_entry();
pub(super) const ENTRY: &[u8] = &GENERATED_ENTRY;
