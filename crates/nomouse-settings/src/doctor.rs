//! Reading `nomouse doctor`'s report: `[ok  ] name: detail` and `[FAIL] name: problem` lines.

/// One line (with its continuation lines) of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorLine {
    /// A check.
    Check {
        /// Whether it passed.
        ok: bool,
        /// What was checked.
        name: String,
        /// The detail or the problem.
        detail: String,
    },
    /// Anything else: `platform: linux`, `hint: …`, an error message.
    Note(String),
}

/// Splits `[mark] name: text` into the check.
fn check(line: &str) -> Option<DoctorLine> {
    let (ok, rest) = if let Some(rest) = line.strip_prefix("[ok  ]") {
        (true, rest)
    } else {
        (false, line.strip_prefix("[FAIL]")?)
    };
    let rest = rest.trim();
    let (name, detail) = rest.split_once(": ").unwrap_or((rest, ""));
    Some(DoctorLine::Check {
        ok,
        name: name.trim().to_owned(),
        detail: detail.trim().to_owned(),
    })
}

/// Parses the whole report. An indented line continues the check above it.
pub fn parse(output: &str) -> Vec<DoctorLine> {
    let mut lines: Vec<DoctorLine> = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace)
            && let Some(DoctorLine::Check { detail, .. }) = lines.last_mut()
        {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(line.trim());
            continue;
        }
        lines.push(check(line).unwrap_or_else(|| DoctorLine::Note(line.trim().to_owned())));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_checks_notes_and_continuations() {
        let report = "platform: linux\n\
                      [ok  ] at-spi: bus reachable\n\
                      [FAIL] accessibility: switched off\n       turn it on in Settings\n\
                      \n\
                      [ok  ] ignored apps: none\n\
                      hint: AT-SPI must be running\n";
        assert_eq!(
            parse(report),
            vec![
                DoctorLine::Note("platform: linux".to_owned()),
                DoctorLine::Check {
                    ok: true,
                    name: "at-spi".to_owned(),
                    detail: "bus reachable".to_owned(),
                },
                DoctorLine::Check {
                    ok: false,
                    name: "accessibility".to_owned(),
                    detail: "switched off\nturn it on in Settings".to_owned(),
                },
                DoctorLine::Check {
                    ok: true,
                    name: "ignored apps".to_owned(),
                    detail: "none".to_owned(),
                },
                DoctorLine::Note("hint: AT-SPI must be running".to_owned()),
            ]
        );
    }

    #[test]
    fn a_check_without_detail_keeps_its_name() {
        assert_eq!(
            parse("[FAIL] display\n  indented note without a check\n"),
            vec![DoctorLine::Check {
                ok: false,
                name: "display".to_owned(),
                detail: "indented note without a check".to_owned(),
            },]
        );
        assert_eq!(
            parse("  stray\n"),
            vec![DoctorLine::Note("stray".to_owned())]
        );
    }
}
