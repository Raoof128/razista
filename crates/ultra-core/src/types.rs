//! Domain contracts (design §17, plan T02 type registry).
//!
//! Every struct rejects unknown fields and every enum is closed. Commands are
//! typed requests; no contract carries an arbitrary command string or an
//! untyped JSON blob. Policy and capability types live in [`crate::policy`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::ErrorCode;
use crate::primitives::{
    AgentId, ArtifactId, BoundedVec, CallerId, Digest, EventId, EvidenceId, FindingId,
    HypothesisId, IdempotencyKey, Ident, JobId, Label, ObjectId, ObservationId, PreparationId,
    ProposalId, RepoId, RepoPath, SnapshotId, SourceText, Summary, Text, Timestamp, VerificationId,
};

/// Current domain schema version.
pub const SCHEMA_VERSION: u32 = 1;

// ---------- trust, severity, evidence ----------------------------------------

/// Provenance label carried with every piece of context and output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrustLabel {
    /// Engine-generated or operator-approved configuration.
    Trusted,
    /// Reviewed repository content, including comments and docs.
    Untrusted,
    /// Output of a third-party tool or code under test.
    ToolOutput,
    /// Output of a model.
    ModelOutput,
}

/// Impact if the claim holds. Independent of how certain the claim is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// What the evidence establishes (design §11). Separate from [`Severity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    /// Supervisor-observed execution violates a sourced behavioural contract.
    RuntimeReproduced,
    /// A pinned analyser/compiler independently reproduces a diagnostic.
    DeterministicDiagnostic,
    /// A checkable argument under explicit assumptions and bounds.
    BoundedStaticProof,
    /// Plausible but not established.
    Unverified,
    /// Evidence contradicts the claim.
    Disproved,
    /// Conflicting, flaky, budget-exhausted or oracle-inadequate.
    Inconclusive,
}

/// Whether the reviewed change introduced the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAttribution {
    IntroducedByChange,
    PreExisting,
    Unknown,
}

// ---------- scope and snapshots ----------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeMode {
    Branch,
    Range,
    Commit,
    Staged,
    Worktree,
    Paths,
    ImportedPr,
}

/// A local revision expression as typed by the caller. The snapshotter (T06)
/// resolves it to an [`ObjectId`] before any agent runs and refuses anything
/// that could be read as a Git option.
pub type RevSpec = Text<1, 256>;

/// The scope a caller asks for. Resolved into a [`ResolvedScope`] before any
/// agent is scheduled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewScope {
    pub mode: ScopeMode,
    pub base: Option<RevSpec>,
    pub head: Option<RevSpec>,
    pub paths: BoundedVec<RepoPath, 1000>,
    pub include_untracked: bool,
    /// 1-based parent for `commit` mode on a merge commit.
    pub merge_parent: Option<u8>,
}

/// The exact comparison the engine used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedScope {
    pub mode: ScopeMode,
    pub base_commit: Option<ObjectId>,
    pub head_commit: Option<ObjectId>,
    pub merge_parent: Option<u8>,
    pub paths: BoundedVec<RepoPath, 1000>,
    pub include_untracked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Unchanged,
}

/// One included file, identified by its reviewed bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub path: RepoPath,
    /// Digest of the reviewed bytes (for a deletion: the bytes that were deleted).
    pub digest: Digest,
    /// Digest of the base-side bytes (the same path, or the rename source) for
    /// a `modified` or `renamed` entry; `None` otherwise. Lets diffs be rebuilt
    /// from stored blobs.
    pub base_digest: Option<Digest>,
    pub mode: FileMode,
    pub size_bytes: u64,
    pub line_count: u32,
    pub change: ChangeKind,
    pub renamed_from: Option<RepoPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Binary,
    TooLarge,
    LfsPointer,
    Submodule,
    Generated,
    Vendored,
    UnsafePath,
    UntrackedNotApproved,
    FileLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkippedInput {
    pub path: RepoPath,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub entries: BoundedVec<ManifestEntry, 50000>,
    pub skipped: BoundedVec<SkippedInput, 200000>,
}

/// How the capture proved it did not mix concurrent edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsistencyEvidence {
    pub capture_passes: u8,
    pub manifest_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub snapshot_id: SnapshotId,
    pub repo_id: RepoId,
    pub resolved_scope: ResolvedScope,
    pub manifest: SnapshotManifest,
    pub consistency_evidence: ConsistencyEvidence,
    /// Everything the capture could not include or wants the reader to know.
    pub gaps: BoundedVec<CoverageGap, 1000>,
}

/// A cited range of reviewed bytes. Lines are 1-based and inclusive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub path: RepoPath,
    pub file_digest: Digest,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepoGrant {
    pub repo_id: RepoId,
    /// Canonical local identity, display-escaped.
    pub canonical_identity: Text<1, 4096>,
    pub owner: CallerId,
    pub allowed_scopes: BoundedVec<ScopeMode, 7>,
    pub created_at: Timestamp,
}

// ---------- budgets, coverage, jobs ------------------------------------------

/// Hard resource limits for a job (R10). Defaults are the design's initial
/// scheduling defaults, not measured optima.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceBudget {
    pub wall_time_ms: u64,
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub output_bytes: u64,
    pub processes: u32,
    pub tokens: u64,
    pub max_inference_concurrency: u8,
    pub max_verification_workers: u8,
    pub max_discovery_roles: u8,
    pub max_repair_attempts: u8,
    pub max_verification_candidates: u16,
    pub max_packet_tokens: u32,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            wall_time_ms: 60 * 60 * 1000,
            cpu_millis: 4 * 60 * 60 * 1000,
            memory_bytes: 4 * 1024 * 1024 * 1024,
            disk_bytes: 8 * 1024 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
            processes: 256,
            tokens: 2_000_000,
            max_inference_concurrency: 1,
            max_verification_workers: 2,
            max_discovery_roles: 6,
            max_repair_attempts: 2,
            max_verification_candidates: 24,
            max_packet_tokens: 12_000,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceUsage {
    pub wall_time_ms: u64,
    pub cpu_millis: u64,
    pub peak_memory_bytes: u64,
    pub disk_bytes: u64,
    pub output_bytes: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    LanguageUnsupported,
    DependencyUnavailable,
    VerificationUnavailable,
    ScannerUnavailable,
    InputSkipped,
    ContextTruncated,
    /// Paths that differ only by letter case; unsafe on case-insensitive volumes.
    CaseCollision,
    CandidateCapReached,
    BudgetExhausted,
    StageFailed,
}

/// A visible statement of what was not reviewed or not verified (R09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageGap {
    pub kind: GapKind,
    pub subject: Text<1, 512>,
    pub code: Option<ErrorCode>,
    pub detail: Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    Preflight,
    Snapshot,
    Indexing,
    Scanning,
    Discovering,
    Verifying,
    Adjudicating,
    Reporting,
}

/// Lifecycle state. `Completed`, `Partial`, `Failed` and `Cancelled` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    InputRequired,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    Correctness,
    Security,
    Contracts,
    StateConcurrency,
    PerformanceResources,
    TestsRegressions,
    Verifier,
    Challenger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewPlan {
    pub profile_id: Ident,
    pub roles: BoundedVec<RoleKind, 8>,
    pub disabled_capabilities: BoundedVec<CoverageGap, 256>,
    pub budget: ResourceBudget,
}

/// A frozen preparation. Starting it binds exactly this snapshot and policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreparedReview {
    pub preparation_id: PreparationId,
    pub snapshot_id: SnapshotId,
    pub policy_hash: Digest,
    pub plan: ReviewPlan,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub preparation_id: PreparationId,
    pub idempotency_key: IdempotencyKey,
    pub caller: CallerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewJob {
    pub job_id: JobId,
    pub request: ReviewRequest,
    pub stage: JobStage,
    pub status: JobState,
    pub budget: ResourceBudget,
    pub gaps: BoundedVec<CoverageGap, 1000>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobCounters {
    pub hypotheses: u32,
    pub verifications: u32,
    pub reproduced: u32,
    pub diagnostics: u32,
    pub open_questions: u32,
}

/// What `review_status` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewStatus {
    pub job_id: JobId,
    pub stage: JobStage,
    pub state: JobState,
    pub counters: JobCounters,
    pub budget: ResourceBudget,
    pub usage: ResourceUsage,
    pub gaps: BoundedVec<CoverageGap, 1000>,
}

// ---------- capabilities and offline bundles ----------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HardwareProfile {
    pub os: Label,
    pub arch: Ident,
    pub cpu: Label,
    pub logical_cpus: u16,
    pub memory_bytes: u64,
    pub accelerator: Option<Label>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BundleKind {
    ModelWeights,
    InferenceRuntime,
    RulePack,
    AdvisoryDatabase,
    LanguageDependencies,
    ExecutionImage,
    Scanner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleEntry {
    pub name: Ident,
    pub kind: BundleKind,
    pub version: Label,
    pub digest: Digest,
    pub licence: Label,
    pub origin: Text<1, 2048>,
    pub created_at: Timestamp,
}

/// The offline bundle produced by `prepare` (R21). Review uses nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub created_at: Timestamp,
    pub entries: BoundedVec<BundleEntry, 1024>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityComponent {
    Model,
    InferenceRuntime,
    Sandbox,
    Egress,
    Compiler,
    Packages,
    Scanner,
    Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Present and its self-test passed.
    Ready,
    /// Present but not certified (e.g. an unmanaged localhost endpoint).
    Unverified,
    Missing,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub component: CapabilityComponent,
    pub name: Ident,
    pub status: CapabilityStatus,
    pub detail: Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    TypeScript,
    JavaScript,
    Rust,
    Go,
    Java,
    Kotlin,
    C,
    Cpp,
    CSharp,
    Php,
    Ruby,
    Dart,
    Swift,
    Sql,
    Terraform,
    Yaml,
    Dockerfile,
    Shell,
}

/// Per-language support, reported per capability rather than as one badge (R20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageCapability {
    pub language: Language,
    pub parse: bool,
    pub context: bool,
    pub analyse: bool,
    pub execute: bool,
    pub verify: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReport {
    pub offline_certified: bool,
    pub hardware: Option<HardwareProfile>,
    pub components: BoundedVec<Capability, 256>,
    pub languages: BoundedVec<LanguageCapability, 64>,
}

// ---------- context and models --------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextQueryKind {
    ChangedHunks,
    EnclosingDefinitions,
    CallersCallees,
    TestsAndContracts,
    Specific,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextQuery {
    pub snapshot_id: SnapshotId,
    pub kind: ContextQueryKind,
    pub anchors: BoundedVec<SourceSpan, 64>,
    pub max_tokens: u32,
}

/// How a context relationship was obtained. Compiler facts and syntactic
/// guesses stay distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextProvenance {
    Diff,
    Syntactic,
    Compiler,
    ExistingTest,
    Requested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    pub span: SourceSpan,
    pub trust: TrustLabel,
    pub provenance: ContextProvenance,
    pub content: SourceText,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextPacket {
    pub snapshot_id: SnapshotId,
    pub items: BoundedVec<ContextItem, 512>,
    pub omitted: BoundedVec<CoverageGap, 256>,
    pub token_estimate: u32,
}

/// Exact identity of the model that produced an output (design §14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelIdentity {
    pub name: Ident,
    pub weights_digest: Digest,
    pub tokenizer_digest: Digest,
    pub quantisation: Ident,
    pub runtime_build: Label,
    pub context_limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilities {
    pub identity: ModelIdentity,
    pub structured_output: bool,
    pub max_concurrency: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    pub role: RoleKind,
    pub prompt_id: Ident,
    pub prompt_version: Ident,
    pub response_schema: Ident,
    pub packet: ContextPacket,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelResponse {
    pub model: ModelIdentity,
    /// The raw structured output, stored as an untrusted artefact.
    pub output_artifact: ArtifactId,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub repair_attempts: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Always,
    Conditional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    pub role: RoleKind,
    pub activation: Activation,
    pub prompt_version: Ident,
    pub max_packet_tokens: u32,
}

// ---------- discovery ----------------------------------------------------------

/// A falsifiable claim from a discovery role (design §8 stage C).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis {
    pub id: HypothesisId,
    pub role: RoleKind,
    pub discovered_by: AgentId,
    pub claim: Summary,
    pub invariant: Summary,
    pub trigger: Summary,
    pub impact: Summary,
    pub falsification: Summary,
    pub locations: BoundedVec<SourceSpan, 16>,
    pub context_refs: BoundedVec<SourceSpan, 64>,
}

// ---------- execution ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotVariant {
    Base,
    Reviewed,
    Patched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub wall_time_ms: u64,
    pub output_bytes: u64,
    pub processes: u32,
    pub disk_bytes: u64,
}

/// A typed execution request. There is deliberately no command field: the
/// preset decides what runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxRequest {
    pub preset_id: Ident,
    pub snapshot_id: SnapshotId,
    pub variant: SnapshotVariant,
    pub test_artifact: Option<ArtifactId>,
    pub limits: ExecutionLimits,
}

/// Outcome as observed by the supervisor, never as printed by the workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionOutcome {
    Exited { code: i32 },
    Signalled { signal: i32 },
    TimedOut,
    OutputLimit,
    ResourceLimit,
    SandboxFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecord {
    pub request: SandboxRequest,
    pub outcome: ExecutionOutcome,
    pub supervisor: AgentId,
    pub stdout: Option<ArtifactId>,
    pub stderr: Option<ArtifactId>,
    pub output_truncated: bool,
    pub duration_ms: u64,
    pub usage: ResourceUsage,
    pub environment_digest: Digest,
}

// ---------- scanners ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanPlan {
    pub snapshot_id: SnapshotId,
    pub scanners: BoundedVec<Ident, 32>,
    pub compare_base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffSide {
    Base,
    Head,
    Both,
}

/// A deterministic tool result. An observation, not a confirmed vulnerability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: ObservationId,
    pub scanner: Ident,
    pub rule_id: Label,
    pub location: Option<SourceSpan>,
    pub message: Summary,
    pub severity: Severity,
    pub side: DiffSide,
    pub trust: TrustLabel,
}

// ---------- verification and evidence -------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    ApprovedRequirement,
    ApiSemantics,
    ExistingTest,
    CheckedInvariant,
    ReferenceImplementation,
}

/// Where the expected behaviour comes from. Generated tests never define it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractSource {
    pub kind: ContractKind,
    pub statement: Summary,
    pub location: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationRequest {
    pub snapshot_id: SnapshotId,
    pub hypothesis: Hypothesis,
    pub contract: Option<ContractSource>,
    pub attempt_budget: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub id: VerificationId,
    pub hypothesis_id: HypothesisId,
    pub verifier_id: AgentId,
    pub evidence_ids: BoundedVec<EvidenceId, 16>,
    pub outcome: EvidenceClass,
    pub assumptions: BoundedVec<Summary, 16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Execution,
    Diagnostic,
    StaticArgument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub subject_snapshot: SnapshotId,
    pub producer: AgentId,
    pub action: Option<SandboxRequest>,
    pub outcome: Option<ExecutionOutcome>,
    pub artifact_refs: BoundedVec<ArtifactId, 32>,
}

// ---------- findings and reports --------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: FindingId,
    /// The hypothesis this finding publishes. Every cited verification must be of it.
    pub hypothesis_id: HypothesisId,
    pub claim: Summary,
    pub severity: Severity,
    pub evidence_class: EvidenceClass,
    pub introduced_by_change: ChangeAttribution,
    pub discovered_by: AgentId,
    pub locations: BoundedVec<SourceSpan, 16>,
    pub verification_ids: BoundedVec<VerificationId, 8>,
    pub impact: Summary,
    pub reproduction_summary: Option<Summary>,
    pub limitations: BoundedVec<Summary, 16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_id: EvidenceId,
    pub kind: EvidenceKind,
    /// The agent or supervisor that produced the evidence.
    pub producer: AgentId,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub files_in_scope: u32,
    pub files_reviewed: u32,
    pub gaps: BoundedVec<CoverageGap, 1000>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VersionInfo {
    pub engine: Label,
    pub policy_digest: Digest,
    pub prompt_version: Option<Ident>,
    pub model: Option<ModelIdentity>,
}

/// The terminal report. Sections are disjoint by evidence class:
/// `findings` holds only runtime-reproduced bugs, `diagnostics` holds
/// deterministic diagnostics and bounded static proofs, `open_questions`
/// holds unverified and inconclusive concerns. Disproved hypotheses stay in
/// the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewReport {
    pub schema_version: u32,
    pub job_id: JobId,
    pub snapshot_id: SnapshotId,
    pub status: JobState,
    pub scope: ResolvedScope,
    pub findings: BoundedVec<Finding, 500>,
    pub diagnostics: BoundedVec<Finding, 2000>,
    pub open_questions: BoundedVec<Finding, 500>,
    pub verifications: BoundedVec<Verification, 3000>,
    pub evidence_manifest: BoundedVec<EvidenceRef, 10000>,
    /// Digest of the snapshot manifest the report was produced from.
    pub manifest_digest: Digest,
    /// Reviewed files cited by any finding, with the digests of their bytes.
    pub sources: BoundedVec<ManifestEntry, 20000>,
    pub coverage: Coverage,
    pub versions: VersionInfo,
    pub resource_usage: ResourceUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Json,
    Markdown,
    Sarif,
}

// ---------- patches ---------------------------------------------------------------------

/// A file must hold exactly `digest` (or be absent when `None`) for a patch to apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilePrecondition {
    pub path: RepoPath,
    pub digest: Option<Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchProposal {
    pub id: ProposalId,
    pub target_snapshot: SnapshotId,
    pub file_preconditions: BoundedVec<FilePrecondition, 256>,
    pub diff_artifact: ArtifactId,
    pub verification_job_ids: BoundedVec<JobId, 32>,
    pub risks: BoundedVec<Summary, 32>,
}

/// A local human's approval of one exact proposal. Minted only by the local
/// authority (T20), never by a model or an MCP tool argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchApproval {
    pub proposal_id: ProposalId,
    pub patch_digest: Digest,
    pub preconditions_digest: Digest,
    pub approved_by: CallerId,
    pub expires_at: Timestamp,
}

// ---------- events and pagination ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    StageEntered { stage: JobStage },
    StateChanged { state: JobState },
    GapRecorded { gap: CoverageGap },
    HypothesisRecorded { hypothesis_id: HypothesisId },
    VerificationRecorded { verification_id: VerificationId },
    ActionDenied { code: ErrorCode, detail: Summary },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: EventId,
    pub job_id: JobId,
    pub at: Timestamp,
    pub kind: EventKind,
}

/// Opaque pagination cursor. Carries no authority.
pub type Cursor = Text<1, 256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub items: BoundedVec<T, 100>,
    pub next: Option<Cursor>,
}
