use std::io::Cursor;
use std::path::Path;

use cargo_xtest::{Diagnostics, explain_reader};

#[test]
fn invalid_utf8_is_an_io_error_without_a_source_diagnostic() {
    let bytes = Cursor::new(vec![b'/', b'/', b'@', b' ', 0xff, b'\n']);
    let error = explain_reader(Path::new("tests/not-utf8.rs"), bytes).unwrap_err();

    assert!(error.downcast_ref::<Diagnostics>().is_none());
    assert_eq!(error.to_string(), "could not read test source `tests/not-utf8.rs`");
    assert_eq!(error.chain().count(), 2);
}
