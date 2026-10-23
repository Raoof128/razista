//! Contract validation beyond what the type system enforces.
//!
//! The types make malformed input unrepresentable (bounds, closed enums,
//! unknown fields). These checks enforce cross-object invariants: every
//! published claim carries independent evidence of its own hypothesis and of
//! the right kind for its class (R07, R08, R25); verifiers and evidence
//! producers are not the discoverer (R06); no verification is shared between
//! findings or contradicted by a disproof; and every cited span points at
//! reviewed bytes.

use std::collections::{HashMap, HashSet};

use crate::errors::{Result, ReviewError};
use crate::primitives::{EvidenceId, FindingId, RepoPath, VerificationId};
use crate::types::{
    EvidenceClass, EvidenceKind, EvidenceRef, Finding, ManifestEntry, ReviewReport, ReviewRequest,
    SCHEMA_VERSION, Snapshot, SourceSpan, Verification,
};

/// Which report section a finding sits in.
#[derive(Debug, Clone, Copy)]
enum Section {
    Reproduced,
    Diagnostics,
    OpenQuestions,
}

impl Section {
    fn admits(self, class: EvidenceClass) -> bool {
        use EvidenceClass::*;
        match self {
            Self::Reproduced => class == RuntimeReproduced,
            Self::Diagnostics => matches!(class, DeterministicDiagnostic | BoundedStaticProof),
            Self::OpenQuestions => matches!(class, Unverified | Inconclusive),
        }
    }

    /// Sections whose entries are published as established and therefore need
    /// independent evidence.
    fn requires_evidence(self) -> bool {
        !matches!(self, Self::OpenQuestions)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Reproduced => "findings",
            Self::Diagnostics => "diagnostics",
            Self::OpenQuestions => "open_questions",
        }
    }
}

/// The evidence kind that can establish a class, if any.
fn required_kind(class: EvidenceClass) -> Option<EvidenceKind> {
    match class {
        EvidenceClass::RuntimeReproduced => Some(EvidenceKind::Execution),
        EvidenceClass::DeterministicDiagnostic => Some(EvidenceKind::Diagnostic),
        EvidenceClass::BoundedStaticProof => Some(EvidenceKind::StaticArgument),
        EvidenceClass::Unverified | EvidenceClass::Disproved | EvidenceClass::Inconclusive => None,
    }
}

/// Validate a terminal report on its own. Fails with `EVIDENCE_INVALID` on any
/// breach. This proves internal consistency only; use
/// [`validate_report_against`] to tie the cited bytes to the real snapshot.
pub fn validate_report(report: &ReviewReport) -> Result<()> {
    if report.schema_version != SCHEMA_VERSION {
        return Err(ReviewError::evidence(format!(
            "unsupported report schema version {}",
            report.schema_version
        )));
    }
    if !report.status.is_terminal() {
        return Err(ReviewError::evidence(format!(
            "report status {:?} is not terminal",
            report.status
        )));
    }

    let mut sources: HashMap<&RepoPath, &ManifestEntry> = HashMap::new();
    for entry in &report.sources {
        if sources.insert(&entry.path, entry).is_some() {
            return Err(ReviewError::evidence(format!(
                "source {} is listed more than once",
                entry.path.display()
            )));
        }
    }
    let mut evidence: HashMap<&EvidenceId, &EvidenceRef> = HashMap::new();
    for e in &report.evidence_manifest {
        if evidence.insert(&e.evidence_id, e).is_some() {
            return Err(ReviewError::evidence(format!(
                "evidence {} is listed more than once",
                e.evidence_id
            )));
        }
    }
    let mut verifications: HashMap<&VerificationId, &Verification> = HashMap::new();
    let mut evidence_owner: HashMap<&EvidenceId, &VerificationId> = HashMap::new();
    for v in &report.verifications {
        if verifications.insert(&v.id, v).is_some() {
            return Err(ReviewError::evidence(format!(
                "verification {} is listed more than once",
                v.id
            )));
        }
        // One execution or diagnostic record backs one verification, so a
        // single observation cannot be counted as support for two claims.
        for eid in &v.evidence_ids {
            if let Some(first) = evidence_owner.insert(eid, &v.id) {
                return Err(ReviewError::evidence(format!(
                    "evidence {eid} backs both verification {first} and {}",
                    v.id
                )));
            }
        }
    }

    let ctx = Ctx {
        sources,
        evidence,
        verifications,
    };
    let mut seen_findings: HashSet<FindingId> = HashSet::new();
    let mut cited: HashSet<VerificationId> = HashSet::new();
    for (section, list) in [
        (Section::Reproduced, report.findings.as_slice()),
        (Section::Diagnostics, report.diagnostics.as_slice()),
        (Section::OpenQuestions, report.open_questions.as_slice()),
    ] {
        for finding in list {
            if !seen_findings.insert(finding.id) {
                return Err(ReviewError::evidence(format!(
                    "finding {} appears more than once",
                    finding.id
                )));
            }
            for vid in &finding.verification_ids {
                if !cited.insert(*vid) {
                    return Err(ReviewError::evidence(format!(
                        "verification {vid} is cited by more than one finding"
                    )));
                }
            }
            ctx.check_finding(section, finding)?;
        }
    }
    Ok(())
}

/// Validate a report and prove its cited sources are the snapshot's bytes.
pub fn validate_report_against(report: &ReviewReport, snapshot: &Snapshot) -> Result<()> {
    if report.snapshot_id != snapshot.snapshot_id {
        return Err(ReviewError::evidence(format!(
            "report describes snapshot {} but was checked against {}",
            report.snapshot_id, snapshot.snapshot_id
        )));
    }
    if report.manifest_digest != snapshot.consistency_evidence.manifest_digest {
        return Err(ReviewError::evidence(
            "report manifest digest does not match the snapshot",
        ));
    }
    let manifest: HashMap<&RepoPath, &ManifestEntry> = snapshot
        .manifest
        .entries
        .iter()
        .map(|e| (&e.path, e))
        .collect();
    for source in &report.sources {
        match manifest.get(&source.path) {
            Some(entry) if *entry == source => {}
            Some(_) => {
                return Err(ReviewError::evidence(format!(
                    "source {} differs from the snapshot manifest entry",
                    source.path.display()
                )));
            }
            None => {
                return Err(ReviewError::evidence(format!(
                    "source {} is not in the snapshot manifest",
                    source.path.display()
                )));
            }
        }
    }
    validate_report(report)
}

struct Ctx<'a> {
    sources: HashMap<&'a RepoPath, &'a ManifestEntry>,
    evidence: HashMap<&'a EvidenceId, &'a EvidenceRef>,
    verifications: HashMap<&'a VerificationId, &'a Verification>,
}

impl Ctx<'_> {
    fn check_finding(&self, section: Section, finding: &Finding) -> Result<()> {
        let id = finding.id;
        if !section.admits(finding.evidence_class) {
            return Err(ReviewError::evidence(format!(
                "finding {id} with class {:?} cannot be listed under {}",
                finding.evidence_class,
                section.name()
            )));
        }
        if finding.locations.is_empty() {
            return Err(ReviewError::evidence(format!(
                "finding {id} cites no source location"
            )));
        }
        for span in &finding.locations {
            self.check_span(span)
                .map_err(|e| ReviewError::evidence(format!("finding {id}: {}", e.message())))?;
        }

        let required = required_kind(finding.evidence_class);
        let mut supporting = 0usize;
        for vid in &finding.verification_ids {
            let v = self.verifications.get(vid).ok_or_else(|| {
                ReviewError::evidence(format!("finding {id} cites unknown verification {vid}"))
            })?;
            if v.hypothesis_id != finding.hypothesis_id {
                return Err(ReviewError::evidence(format!(
                    "finding {id} cites verification {vid} of a different hypothesis"
                )));
            }
            if v.verifier_id == finding.discovered_by {
                return Err(ReviewError::evidence(format!(
                    "finding {id}: verification {vid} was produced by the discovering agent"
                )));
            }
            if v.outcome == EvidenceClass::Disproved {
                return Err(ReviewError::evidence(format!(
                    "finding {id} is contradicted by disproving verification {vid}"
                )));
            }
            let mut has_required_kind = false;
            for eid in &v.evidence_ids {
                let e = self.evidence.get(eid).ok_or_else(|| {
                    ReviewError::evidence(format!(
                        "verification {vid} cites evidence {eid} missing from the manifest"
                    ))
                })?;
                if e.producer == finding.discovered_by {
                    return Err(ReviewError::evidence(format!(
                        "evidence {eid} for finding {id} was produced by the discovering agent"
                    )));
                }
                if Some(e.kind) == required {
                    has_required_kind = true;
                }
            }
            if v.outcome == finding.evidence_class && has_required_kind {
                supporting += 1;
            }
        }
        if section.requires_evidence() && supporting == 0 {
            return Err(ReviewError::evidence(format!(
                "finding {id} claims {:?} without an independent verification of that class backed by {:?} evidence",
                finding.evidence_class, required
            )));
        }
        Ok(())
    }

    fn check_span(&self, span: &SourceSpan) -> Result<()> {
        let entry = self.sources.get(&span.path).ok_or_else(|| {
            ReviewError::evidence(format!(
                "span cites {} which is not a reviewed source",
                span.path.display()
            ))
        })?;
        if entry.digest != span.file_digest {
            return Err(ReviewError::evidence(format!(
                "span digest for {} does not match the reviewed bytes",
                span.path.display()
            )));
        }
        if span.start_line == 0 || span.start_line > span.end_line {
            return Err(ReviewError::evidence(format!(
                "span {}:{}-{} is not a valid 1-based range",
                span.path.display(),
                span.start_line,
                span.end_line
            )));
        }
        if span.end_line > entry.line_count {
            return Err(ReviewError::evidence(format!(
                "span {}:{}-{} exceeds the file's {} lines",
                span.path.display(),
                span.start_line,
                span.end_line,
                entry.line_count
            )));
        }
        Ok(())
    }
}

/// Validate a start request. Field formats are enforced by the types; this
/// is the hook for cross-field rules as the request grows.
pub fn validate_request(request: &ReviewRequest) -> Result<()> {
    crate::primitives::IdempotencyKey::new(request.idempotency_key.as_str())?;
    Ok(())
}
