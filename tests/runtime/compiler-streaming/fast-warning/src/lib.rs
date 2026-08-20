#[deprecated(note = "cargo compiler output is live")]
fn deprecated_marker() {}

pub fn emit_warning() {
    deprecated_marker();
}
