pub(crate) trait AsStr {
    fn as_str(&self) -> &'static str;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliOrRunOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: u8,
}
