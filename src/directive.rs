use std::io::{self, BufRead};
use std::path::Path;

use microsandbox::sandbox::PullPolicy;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, SourceLabel, SourceSpan};
use crate::helpers::AsStr;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirectiveForm {
    Presence,
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectiveLine {
    name: String,
    form: DirectiveForm,
    remark: Option<String>,
    span: SourceSpan,
    line_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Capability {
    Threads,
    Subprocess,
    Symlink,
    DynamicLinking,
    TargetStd,
    Unwind,
}

impl AsStr for Capability {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Threads => "threads",
            Self::Subprocess => "subprocess",
            Self::Symlink => "symlink",
            Self::DynamicLinking => "dynamic-linking",
            Self::TargetStd => "target-std",
            Self::Unwind => "unwind",
        }
    }
}

impl AsStr for PullPolicy {
    fn as_str(&self) -> &'static str {
        match self {
            Self::IfMissing => "if-missing",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Directive {
    IgnoreTest(String),
    Only(String),
    Ignore(String),
    Capability(Capability),
    RunFlags(Vec<String>),
    ExecEnv { key: String, value: String },
    UnsetExecEnv(String),
    Image(String),
    FromSnapshot(String),
    PullPolicy(PullPolicy),
    Cpus(u8),
    Memory(u32),
    RootDisk(u32),
    MaxDuration(u64),
    User(String),
    Workdir(String),
    Shell(String),
    Init(String),
    DisableNetwork,
    PreserveOnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) span: SourceSpan,
    pub(crate) line_text: String,
}

impl Location {
    pub(crate) fn label(&self, message: impl Into<String>) -> SourceLabel {
        SourceLabel {
            span: self.span,
            line_text: self.line_text.clone(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocatedDirective {
    pub(crate) value: Directive,
    pub(crate) location: Location,
}

pub(crate) struct ParseOutput {
    pub(crate) directives: Vec<LocatedDirective>,
    pub(crate) diagnostics: Diagnostics,
}

pub(crate) fn parse_reader(path: &Path, mut reader: impl BufRead) -> io::Result<ParseOutput> {
    let mut directives = Vec::new();
    let mut diagnostics = Diagnostics::default();
    let mut line = String::new();
    let mut line_number = 0;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        let source_line = line.strip_suffix('\n').unwrap_or(&line);
        if let Some(directive_line) = scan_line(line_number, source_line) {
            match parse_directive(path, &directive_line) {
                Ok(directive) => directives.push(directive),
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
    }

    Ok(ParseOutput { directives, diagnostics })
}

fn scan_line(line_number: usize, line: &str) -> Option<DirectiveLine> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let content = line.trim_start();
    let marker_offset = line.len() - content.len();
    let after_marker = content.strip_prefix("//@")?;
    let directive = after_marker.trim_start();
    let directive_offset = after_marker.len() - directive.len();
    let name_end = directive
        .find(|character: char| character == ':' || character.is_whitespace())
        .unwrap_or(directive.len());
    let name = &directive[..name_end];
    let remainder = &directive[name_end..];

    let (form, remark) = if let Some(value) = remainder.strip_prefix(':') {
        (DirectiveForm::Value(value.trim().to_owned()), None)
    } else {
        let remark = remainder.trim();
        let remark = (!remark.is_empty()).then(|| remark.to_owned());
        (DirectiveForm::Presence, remark)
    };

    Some(DirectiveLine {
        name: name.to_owned(),
        form,
        remark,
        span: SourceSpan {
            line: line_number,
            column: marker_offset + "//@".len() + directive_offset + 1,
            length: name.len().max(1),
        },
        line_text: line.to_owned(),
    })
}

fn parse_directive(path: &Path, line: &DirectiveLine) -> Result<LocatedDirective, Diagnostic> {
    let location = Location { span: line.span, line_text: line.line_text.clone() };
    if line.name.starts_with('[') {
        return Err(diagnostic(
            DiagnosticCode::Unsupported,
            "revision-qualified directives are not supported",
            path,
            &location,
        )
        .help("use separate Cargo test targets or library-level cases"));
    }
    if line.name.is_empty() {
        return Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            "directive name is missing",
            path,
            &location,
        ));
    }

    if matches!(line.form, DirectiveForm::Presence)
        && line
            .remark
            .as_deref()
            .and_then(|remark| remark.split_whitespace().next())
            .is_some_and(is_known_name)
    {
        return Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            "a directive must be the only directive on its line",
            path,
            &location,
        )
        .help("put each `//@` directive on its own line"));
    }

    let value = if let Some(value) = parse_test_directive(path, line, &location)? {
        value
    } else {
        parse_sandbox_directive(path, line, &location)?
    };

    Ok(LocatedDirective { value, location })
}

fn parse_test_directive(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Option<Directive>, Diagnostic> {
    let name = line.name.as_str();
    if name == "ignore-test" {
        return parse_ignore_test(path, line, location).map(Some);
    }
    if let Some(predicate) = name.strip_prefix("only-").filter(|value| !value.is_empty()) {
        expect_presence(path, line, location)?;
        return Ok(Some(Directive::Only(predicate.to_owned())));
    }
    if let Some(predicate) = name.strip_prefix("ignore-").filter(|value| !value.is_empty()) {
        expect_presence(path, line, location)?;
        return Ok(Some(Directive::Ignore(predicate.to_owned())));
    }

    let directive = match name {
        "needs-threads" => presence_capability(path, line, location, Capability::Threads)?,
        "needs-subprocess" => presence_capability(path, line, location, Capability::Subprocess)?,
        "needs-symlink" => presence_capability(path, line, location, Capability::Symlink)?,
        "needs-dynamic-linking" => {
            presence_capability(path, line, location, Capability::DynamicLinking)?
        }
        "needs-target-std" => presence_capability(path, line, location, Capability::TargetStd)?,
        "needs-unwind" => presence_capability(path, line, location, Capability::Unwind)?,
        "run-flags" => parse_run_flags(path, line, location)?,
        "exec-env" => parse_exec_env(path, line, location)?,
        "unset-exec-env" => {
            let key = expect_value(path, line, location)?;
            validate_environment_key(path, location, key)?;
            Directive::UnsetExecEnv(key.trim().to_owned())
        }
        _ => return Ok(None),
    };
    Ok(Some(directive))
}

fn parse_sandbox_directive(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let value = || expect_value(path, line, location);
    match line.name.as_str() {
        "image" => Ok(Directive::Image(value()?.to_owned())),
        "from-snapshot" => Ok(Directive::FromSnapshot(value()?.to_owned())),
        "pull-policy" => parse_pull_policy(path, line, location),
        "cpus" => Ok(Directive::Cpus(parse_positive(path, location, value()?, "cpus")?)),
        "memory" => Ok(Directive::Memory(parse_positive(path, location, value()?, "memory")?)),
        "root-disk" => {
            Ok(Directive::RootDisk(parse_positive(path, location, value()?, "root-disk")?))
        }
        "max-duration" => {
            Ok(Directive::MaxDuration(parse_positive(path, location, value()?, "max-duration")?))
        }
        "user" => Ok(Directive::User(value()?.to_owned())),
        "workdir" => {
            Ok(Directive::Workdir(expect_absolute_guest_path(path, line, location, "workdir")?))
        }
        "shell" => Ok(Directive::Shell(expect_absolute_guest_path(path, line, location, "shell")?)),
        "init" => parse_init(path, line, location),
        "disable-network" => {
            expect_presence(path, line, location)?;
            Ok(Directive::DisableNetwork)
        }
        "preserve-on-failure" => {
            expect_presence(path, line, location)?;
            Ok(Directive::PreserveOnFailure)
        }
        name => Err(diagnostic(
            DiagnosticCode::UnknownDirective,
            format!("unknown directive `{name}`"),
            path,
            location,
        )),
    }
}

fn parse_ignore_test(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    expect_presence(path, line, location)?;
    line.remark
        .as_ref()
        .filter(|remark| !remark.trim().is_empty())
        .map(|reason| Directive::IgnoreTest(reason.clone()))
        .ok_or_else(|| {
            diagnostic(
                DiagnosticCode::InvalidValue,
                "`ignore-test` requires an explanatory remark",
                path,
                location,
            )
            .help("write `//@ ignore-test (reason)`")
        })
}

fn parse_run_flags(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let flags = split_flags(expect_value(path, line, location)?);
    if flags.is_empty() {
        Err(invalid_value(path, location, "`run-flags` must not be empty"))
    } else {
        Ok(Directive::RunFlags(flags))
    }
}

fn parse_exec_env(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| invalid_value(path, location, "`exec-env` requires `KEY=VALUE`"))?;
    validate_environment_key(path, location, key)?;
    Ok(Directive::ExecEnv { key: key.trim().to_owned(), value: value.to_owned() })
}

fn parse_pull_policy(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let policy = match expect_value(path, line, location)? {
        "if-missing" => PullPolicy::IfMissing,
        "always" => PullPolicy::Always,
        "never" => PullPolicy::Never,
        _ => {
            return Err(invalid_value(path, location, "invalid `pull-policy` value")
                .help("expected `if-missing`, `always`, or `never`"));
        }
    };
    Ok(Directive::PullPolicy(policy))
}

fn parse_init(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let value = expect_value(path, line, location)?;
    if value == "auto" || value.starts_with('/') {
        Ok(Directive::Init(value.to_owned()))
    } else {
        Err(invalid_value(path, location, "`init` must be `auto` or an absolute guest path"))
    }
}

fn presence_capability(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
    capability: Capability,
) -> Result<Directive, Diagnostic> {
    expect_presence(path, line, location)?;
    Ok(Directive::Capability(capability))
}

fn expect_presence(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<(), Diagnostic> {
    if matches!(line.form, DirectiveForm::Presence) {
        Ok(())
    } else {
        Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            format!("`{}` is a presence directive", line.name),
            path,
            location,
        )
        .help(format!("write `//@ {}` without a colon", line.name)))
    }
}

fn expect_value<'a>(
    path: &Path,
    line: &'a DirectiveLine,
    location: &Location,
) -> Result<&'a str, Diagnostic> {
    match &line.form {
        DirectiveForm::Value(value) if !value.is_empty() => Ok(value),
        DirectiveForm::Value(_) => {
            Err(invalid_value(path, location, format!("`{}` value must not be empty", line.name)))
        }
        DirectiveForm::Presence => Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            format!("`{}` requires a colon and value", line.name),
            path,
            location,
        )
        .help(format!("write `//@ {}: value`", line.name))),
    }
}

fn expect_absolute_guest_path(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
    name: &str,
) -> Result<String, Diagnostic> {
    let value = expect_value(path, line, location)?;
    if value.starts_with('/') {
        Ok(value.to_owned())
    } else {
        Err(invalid_value(path, location, format!("`{name}` must be an absolute guest path")))
    }
}

fn parse_positive<T>(
    path: &Path,
    location: &Location,
    raw: &str,
    name: &str,
) -> Result<T, Diagnostic>
where
    T: TryFrom<u64>,
{
    let value = raw.parse::<u64>().map_err(|_| {
        invalid_value(path, location, format!("`{name}` must be a positive base-10 integer"))
    })?;
    if value == 0 {
        return Err(invalid_value(path, location, format!("`{name}` must be greater than zero")));
    }
    T::try_from(value).map_err(|_| {
        invalid_value(path, location, format!("`{name}` value is outside the supported range"))
    })
}

fn validate_environment_key(path: &Path, location: &Location, key: &str) -> Result<(), Diagnostic> {
    let key = key.trim();
    if key.is_empty() || key.contains(['=', '\0']) {
        Err(invalid_value(
            path,
            location,
            "environment key must be non-empty and contain neither `=` nor NUL",
        ))
    } else {
        Ok(())
    }
}

fn split_flags(flags: &str) -> Vec<String> {
    flags
        .split('\'')
        .enumerate()
        .flat_map(
            |(index, segment)| {
                if index % 2 == 1 { vec![segment] } else { segment.split_whitespace().collect() }
            },
        )
        .map(str::to_owned)
        .collect()
}

fn is_known_name(name: &str) -> bool {
    matches!(
        name,
        "ignore-test"
            | "needs-threads"
            | "needs-subprocess"
            | "needs-symlink"
            | "needs-dynamic-linking"
            | "needs-target-std"
            | "needs-unwind"
            | "run-flags"
            | "exec-env"
            | "unset-exec-env"
            | "image"
            | "from-snapshot"
            | "pull-policy"
            | "cpus"
            | "memory"
            | "root-disk"
            | "max-duration"
            | "user"
            | "workdir"
            | "shell"
            | "init"
            | "disable-network"
            | "preserve-on-failure"
    ) || name.starts_with("only-")
        || name.starts_with("ignore-")
}

fn diagnostic(
    code: DiagnosticCode,
    message: impl Into<String>,
    path: &Path,
    location: &Location,
) -> Diagnostic {
    Diagnostic::new(code, message, path, location.label("directive declared here"))
}

fn invalid_value(path: &Path, location: &Location, message: impl Into<String>) -> Diagnostic {
    diagnostic(DiagnosticCode::InvalidValue, message, path, location)
}

#[cfg(test)]
mod tests {
    use super::{DirectiveForm, scan_line};

    #[test]
    fn ignores_an_ordinary_rust_line() {
        assert_eq!(scan_line(1, "fn main() {}"), None);
    }

    #[test]
    fn scans_presence_directive_after_indentation() {
        let directive = scan_line(7, "    //@ needs-threads").unwrap();

        assert_eq!(directive.name, "needs-threads");
        assert_eq!(directive.form, DirectiveForm::Presence);
        assert_eq!(directive.remark, None);
        assert_eq!(directive.span.line, 7);
        assert_eq!(directive.span.column, 9);
    }

    #[test]
    fn retains_a_presence_directive_remark() {
        let directive = scan_line(3, "//@ only-linux (uses epoll)").unwrap();

        assert_eq!(directive.name, "only-linux");
        assert_eq!(directive.form, DirectiveForm::Presence);
        assert_eq!(directive.remark.as_deref(), Some("(uses epoll)"));
    }

    #[test]
    fn splits_a_value_only_at_the_name_colon() {
        let directive =
            scan_line(4, "//@ image: registry.example:5000/team/app@sha256:deadbeef").unwrap();

        assert_eq!(directive.name, "image");
        assert_eq!(
            directive.form,
            DirectiveForm::Value("registry.example:5000/team/app@sha256:deadbeef".into())
        );
        assert_eq!(directive.remark, None);
    }

    #[test]
    fn normalizes_a_crlf_line_ending() {
        let directive = scan_line(2, "//@ memory: 512\r").unwrap();

        assert_eq!(directive.form, DirectiveForm::Value("512".into()));
    }

    #[test]
    fn retains_unicode_in_values_and_remarks() {
        let value = scan_line(1, "//@ user: oluwaseyi-测试").unwrap();
        let remark = scan_line(2, "//@ ignore-test (blocked by résumé fixture)").unwrap();

        assert_eq!(value.form, DirectiveForm::Value("oluwaseyi-测试".into()));
        assert_eq!(remark.remark.as_deref(), Some("(blocked by résumé fixture)"));
    }
}
