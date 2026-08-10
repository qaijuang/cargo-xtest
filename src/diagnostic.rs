use std::fmt;
use std::path::{Path, PathBuf};

use crate::helpers::AsStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceSpan {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticCode {
    UnknownDirective,
    MalformedDirective,
    InvalidValue,
    Duplicate,
    Conflict,
    Unsupported,
}

impl AsStr for DiagnosticCode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownDirective => "XT001",
            Self::MalformedDirective => "XT002",
            Self::InvalidValue => "XT003",
            Self::Duplicate => "XT004",
            Self::Conflict => "XT005",
            Self::Unsupported => "XT006",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceLabel {
    pub(crate) span: SourceSpan,
    pub(crate) line_text: String,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    data: Box<DiagnosticData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticData {
    code: DiagnosticCode,
    message: String,
    path: PathBuf,
    primary: SourceLabel,
    related: Vec<(SourceSpan, String)>,
    help: Option<String>,
}

impl Diagnostic {
    pub(crate) fn new(
        code: DiagnosticCode,
        message: impl Into<String>,
        path: &Path,
        primary: SourceLabel,
    ) -> Self {
        Self {
            data: Box::new(DiagnosticData {
                code,
                message: message.into(),
                path: path.to_owned(),
                primary,
                related: Vec::new(),
                help: None,
            }),
        }
    }

    pub(crate) fn related(mut self, span: SourceSpan, message: impl Into<String>) -> Self {
        self.data.related.push((span, message.into()));
        self
    }

    pub(crate) fn help(mut self, help: impl Into<String>) -> Self {
        self.data.help = Some(help.into());
        self
    }

    fn position(&self) -> (usize, usize) {
        (self.data.primary.span.line, self.data.primary.span.column)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    entries: Vec<Diagnostic>,
}

impl Diagnostics {
    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.entries.push(diagnostic);
    }

    pub(crate) fn sort(&mut self) {
        self.entries.sort_by_key(Diagnostic::position);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.entries.iter().enumerate() {
            if index != 0 {
                writeln!(formatter)?;
            }
            render_diagnostic(formatter, diagnostic)?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

fn render_diagnostic(formatter: &mut fmt::Formatter<'_>, diagnostic: &Diagnostic) -> fmt::Result {
    let data = &diagnostic.data;
    let primary = &data.primary;
    let gutter_padding = " ".repeat(primary.span.line.to_string().len());
    writeln!(formatter, "error[{}]: {}", data.code.as_str(), data.message)?;
    writeln!(
        formatter,
        "{gutter_padding}--> {}:{}:{}",
        data.path.display(),
        primary.span.line,
        primary.span.column
    )?;
    writeln!(formatter, "{gutter_padding} |")?;
    writeln!(formatter, "{} | {}", primary.span.line, primary.line_text)?;
    let padding = " ".repeat(primary.span.column.saturating_sub(1));
    let underline = "^".repeat(primary.span.length.max(1));
    write!(formatter, "{gutter_padding} | {padding}{underline}")?;
    if let Some(message) = &primary.message {
        write!(formatter, " {message}")?;
    }
    writeln!(formatter)?;

    if !data.related.is_empty() || data.help.is_some() {
        writeln!(formatter, "{gutter_padding} |")?;
    }
    for (span, message) in &data.related {
        writeln!(
            formatter,
            "{gutter_padding} = note: {message} at {}:{}:{}",
            data.path.display(),
            span.line,
            span.column
        )?;
    }
    if let Some(help) = &data.help {
        writeln!(formatter, "{gutter_padding} = help: {help}")?;
    }
    Ok(())
}
