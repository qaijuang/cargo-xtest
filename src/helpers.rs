use std::io;

pub(crate) trait AsStr {
    fn as_str(&self) -> &'static str;
}

pub(crate) fn write_live(output: &mut dyn io::Write, bytes: &[u8]) -> io::Result<()> {
    output.write_all(bytes)?;
    output.flush()
}
