use dvdforge_domain::{Diagnostic, Severity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VerifyProfile {
    Auto,
    DvdVideo,
    Bdmv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VerifyVerdict {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyLocation {
    pub file: String,
    pub offset: Option<u64>,
    pub sector: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCheck {
    pub id: String,
    pub severity: Severity,
    pub status: CheckStatus,
    pub message: String,
    pub location: Option<VerifyLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub iso_path: String,
    pub size_bytes: u64,
    pub profile: VerifyProfile,
    pub verdict: VerifyVerdict,
    pub checks: Vec<VerifyCheck>,
    pub diagnostics: Vec<Diagnostic>,
    pub elapsed_ms: u64,
}

impl VerifyReport {
    pub fn from_checks(
        iso_path: String,
        size_bytes: u64,
        profile: VerifyProfile,
        checks: Vec<VerifyCheck>,
        elapsed_ms: u64,
    ) -> Self {
        let mut diagnostics = Vec::new();
        for check in &checks {
            if check.status == CheckStatus::Fail {
                let diagnostic = Diagnostic::error(check.id.clone(), check.message.clone());
                diagnostics.push(match &check.location {
                    Some(location) => diagnostic.with_entity(format!(
                        "{}:{}",
                        location.file,
                        location.offset.unwrap_or_default()
                    )),
                    None => diagnostic,
                });
            } else if check.status == CheckStatus::Warn {
                diagnostics.push(Diagnostic::warning(check.id.clone(), check.message.clone()));
            }
        }
        let verdict = if checks.iter().any(|check| check.status == CheckStatus::Fail) {
            VerifyVerdict::Fail
        } else if checks.iter().any(|check| check.status == CheckStatus::Warn) {
            VerifyVerdict::Warn
        } else {
            VerifyVerdict::Pass
        };
        Self {
            iso_path,
            size_bytes,
            profile,
            verdict,
            checks,
            diagnostics,
            elapsed_ms,
        }
    }
}

pub(crate) struct CheckList {
    checks: Vec<VerifyCheck>,
}

impl CheckList {
    pub(crate) fn new() -> Self {
        Self { checks: Vec::new() }
    }

    pub(crate) fn pass(&mut self, id: &str, message: impl Into<String>) {
        self.push(id, Severity::Info, CheckStatus::Pass, message.into(), None);
    }

    pub(crate) fn warn(&mut self, id: &str, message: impl Into<String>) {
        self.push(
            id,
            Severity::Warning,
            CheckStatus::Warn,
            message.into(),
            None,
        );
    }

    pub(crate) fn fail(&mut self, id: &str, message: impl Into<String>) {
        self.push(id, Severity::Error, CheckStatus::Fail, message.into(), None);
    }

    pub(crate) fn skip(&mut self, id: &str, message: impl Into<String>) {
        self.push(
            id,
            Severity::Info,
            CheckStatus::Skipped,
            message.into(),
            None,
        );
    }

    pub(crate) fn push(
        &mut self,
        id: &str,
        severity: Severity,
        status: CheckStatus,
        message: String,
        location: Option<VerifyLocation>,
    ) {
        self.checks.push(VerifyCheck {
            id: id.to_string(),
            severity,
            status,
            message,
            location,
        });
    }

    pub(crate) fn into_vec(self) -> Vec<VerifyCheck> {
        self.checks
    }
}
