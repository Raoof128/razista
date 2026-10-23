//! Validated primitive types shared by every contract: identifiers, digests,
//! bounded text and arrays, repository paths and timestamps.
//!
//! Each type validates on construction *and* on deserialisation, and its JSON
//! Schema states the same bounds, so the wire contract and the Rust contract
//! agree.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::errors::{Result, ReviewError};

/// Disallowed in [`Text`]: C0 controls except tab and newline, DEL and C1
/// controls. Blocks terminal escape injection (ESC, CSI, OSC) at the boundary.
fn is_forbidden_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
}

const TEXT_PATTERN: &str = "^[^\\x00-\\x08\\x0B-\\x1F\\x7F-\\x9F]*$";

// ---------- identifiers -----------------------------------------------------

const UUID_V7_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";

fn parse_uuid_v7(s: &str) -> Result<Uuid> {
    let id = Uuid::parse_str(s).map_err(|e| ReviewError::invalid(format!("bad id {s:?}: {e}")))?;
    let canonical = id.hyphenated().to_string() == s;
    if !canonical || id.get_version_num() != 7 || id.get_variant() != uuid::Variant::RFC4122 {
        return Err(ReviewError::invalid(format!(
            "id {s:?} must be a lowercase hyphenated UUIDv7"
        )));
    }
    Ok(id)
}

macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// A UUIDv7 handle. Identifiers are names, never credentials: possession
        /// of one grants nothing.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(s: &str) -> Result<Self> {
                parse_uuid_v7(s).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0.hyphenated(), f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0.hyphenated())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.collect_str(&self.0.hyphenated())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).map_err(D::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": UUID_V7_PATTERN })
            }
        }
    };
}

uuid_id!(/** A registered repository. */ RepoId);
uuid_id!(/** An immutable input snapshot. */ SnapshotId);
uuid_id!(/** A frozen review preparation. */ PreparationId);
uuid_id!(/** A durable review job. */ JobId);
uuid_id!(/** A reported finding. */ FindingId);
uuid_id!(/** An evidence record. */ EvidenceId);
uuid_id!(/** A patch proposal. */ ProposalId);
uuid_id!(/** An authenticated local caller (CLI user, MCP client, model, worker). */ CallerId);
uuid_id!(/** A discovery hypothesis. */ HypothesisId);
uuid_id!(/** A verification attempt. */ VerificationId);
uuid_id!(/** A logical agent instance (discoverer, verifier, challenger, supervisor). */ AgentId);
uuid_id!(/** A deterministic scanner observation. */ ObservationId);
uuid_id!(/** An append-only job event. */ EventId);
uuid_id!(/** A capability grant. */ GrantId);

// ---------- digests ---------------------------------------------------------

const DIGEST_PATTERN: &str = "^sha256:[0-9a-f]{64}$";

/// SHA-256 over a canonical, versioned representation, written `sha256:<hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
    }

    pub fn parse(s: &str) -> Result<Self> {
        let hex_part = s
            .strip_prefix("sha256:")
            .ok_or_else(|| ReviewError::invalid(format!("digest {s:?} must start with sha256:")))?;
        let ok = hex_part.len() == 64
            && hex_part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !ok {
            return Err(ReviewError::invalid(format!(
                "digest {s:?} must be 64 lowercase hex digits"
            )));
        }
        Ok(Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for Digest {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Digest")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": DIGEST_PATTERN })
    }
}

/// A content-addressed blob in the evidence store: the digest of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactId(Digest);

impl ArtifactId {
    pub fn digest(&self) -> &Digest {
        &self.0
    }
}

impl From<Digest> for ArtifactId {
    fn from(d: Digest) -> Self {
        Self(d)
    }
}

impl JsonSchema for ArtifactId {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ArtifactId")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": DIGEST_PATTERN })
    }
}

/// A Git object name (SHA-1 or SHA-256 repository).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectId(String);

const OBJECT_ID_PATTERN: &str = "^[0-9a-f]{40}([0-9a-f]{24})?$";

impl ObjectId {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let ok = matches!(s.len(), 40 | 64)
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if ok {
            Ok(Self(s))
        } else {
            Err(ReviewError::invalid(format!("bad git object id {s:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for ObjectId {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ObjectId")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": OBJECT_ID_PATTERN })
    }
}

// ---------- bounded text and arrays ------------------------------------------

/// UTF-8 text of `MIN..=MAX` Unicode scalar values with no terminal control
/// characters (tab and newline are allowed).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Text<const MIN: usize, const MAX: usize>(String);

impl<const MIN: usize, const MAX: usize> Text<MIN, MAX> {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let len = s.chars().count();
        if len < MIN || len > MAX {
            return Err(ReviewError::invalid(format!(
                "text length {len} outside {MIN}..={MAX}"
            )));
        }
        if s.chars().any(is_forbidden_control) {
            return Err(ReviewError::invalid("text contains control characters"));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const MIN: usize, const MAX: usize> fmt::Debug for Text<MIN, MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<const MIN: usize, const MAX: usize> fmt::Display for Text<MIN, MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const MIN: usize, const MAX: usize> Serialize for Text<MIN, MAX> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de, const MIN: usize, const MAX: usize> Deserialize<'de> for Text<MIN, MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl<const MIN: usize, const MAX: usize> JsonSchema for Text<MIN, MAX> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Text_{MIN}_{MAX}"))
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": MIN,
            "maxLength": MAX,
            "pattern": TEXT_PATTERN
        })
    }
}

/// One-line label, e.g. a scanner rule id or a version string.
pub type Label = Text<1, 256>;
/// A short claim or summary.
pub type Summary = Text<1, 1024>;
/// A longer explanation.
pub type Prose = Text<1, 4096>;

/// Source text supplied to a model as context. Bounded but not filtered for
/// control characters: source bytes are evidence and are sanitised at render
/// time, never silently altered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceText(String);

pub const SOURCE_TEXT_MAX_BYTES: usize = 256 * 1024;

impl SourceText {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        if s.len() > SOURCE_TEXT_MAX_BYTES {
            return Err(ReviewError::invalid("source text exceeds 256 KiB"));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SourceText {
    type Error = ReviewError;
    fn try_from(s: String) -> Result<Self> {
        Self::new(s)
    }
}

impl From<SourceText> for String {
    fn from(s: SourceText) -> Self {
        s.0
    }
}

impl JsonSchema for SourceText {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SourceText")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        // maxLength counts code points; the Rust bound (bytes) is the stricter one.
        json_schema!({ "type": "string", "maxLength": SOURCE_TEXT_MAX_BYTES })
    }
}

/// A machine identifier: lowercase, `[a-z0-9][a-z0-9._-]{0,63}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ident(String);

const IDENT_PATTERN: &str = "^[a-z0-9][a-z0-9._-]{0,63}$";

impl Ident {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let mut bytes = s.bytes();
        let ok = s.len() <= 64
            && bytes
                .next()
                .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && bytes.all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            });
        if ok {
            Ok(Self(s))
        } else {
            Err(ReviewError::invalid(format!("bad identifier {s:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Ident {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Ident {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for Ident {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Ident")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": IDENT_PATTERN })
    }
}

/// Client-chosen key that makes `review_start` idempotent. Scoped by the store
/// to (caller, repository, preparation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

const IDEMPOTENCY_PATTERN: &str = "^[A-Za-z0-9_-]{8,128}$";

impl IdempotencyKey {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let ok = (8..=128).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
        if ok {
            Ok(Self(s))
        } else {
            Err(ReviewError::invalid(
                "idempotency key must be 8-128 of [A-Za-z0-9_-]",
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for IdempotencyKey {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IdempotencyKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for IdempotencyKey {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("IdempotencyKey")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": IDEMPOTENCY_PATTERN })
    }
}

/// An array of at most `MAX` items.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedVec<T, const MAX: usize>(Vec<T>);

impl<T, const MAX: usize> BoundedVec<T, MAX> {
    pub fn new(items: Vec<T>) -> Result<Self> {
        if items.len() > MAX {
            return Err(ReviewError::invalid(format!(
                "{} items exceed the bound of {MAX}",
                items.len()
            )));
        }
        Ok(Self(items))
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }

    pub fn into_inner(self) -> Vec<T> {
        self.0
    }

    /// Append, refusing to exceed the bound.
    pub fn push(&mut self, item: T) -> Result<()> {
        if self.0.len() >= MAX {
            return Err(ReviewError::invalid(format!(
                "bound of {MAX} items reached"
            )));
        }
        self.0.push(item);
        Ok(())
    }
}

impl<T, const MAX: usize> Default for BoundedVec<T, MAX> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<'a, T, const MAX: usize> IntoIterator for &'a BoundedVec<T, MAX> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Serialize, const MAX: usize> Serialize for BoundedVec<T, MAX> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for BoundedVec<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(Vec::<T>::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl<T: JsonSchema, const MAX: usize> JsonSchema for BoundedVec<T, MAX> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("BoundedVec_{}_{MAX}", T::schema_name()))
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": generator.subschema_for::<T>(),
            "maxItems": MAX
        })
    }
}

// ---------- repository paths -------------------------------------------------

pub const REPO_PATH_MAX_BYTES: usize = 4096;

/// A repository-relative path kept as raw bytes, so non-UTF-8 names survive.
///
/// On the wire it is a string in which `%`, control characters and invalid
/// UTF-8 bytes are percent-escaped (`%XX`, uppercase). The encoding is
/// canonical: a path has exactly one wire form.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > REPO_PATH_MAX_BYTES {
            return Err(ReviewError::invalid("repository path length out of range"));
        }
        if bytes.contains(&0) {
            return Err(ReviewError::invalid("repository path contains NUL"));
        }
        if bytes[0] == b'/' {
            return Err(ReviewError::invalid("repository path must be relative"));
        }
        for component in bytes.split(|&b| b == b'/') {
            if component.is_empty() || component == b"." || component == b".." {
                return Err(ReviewError::invalid(format!(
                    "repository path {:?} has an empty, '.' or '..' component",
                    String::from_utf8_lossy(bytes)
                )));
            }
        }
        Ok(Self(bytes.to_vec()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Escaped, printable form: safe to show in a terminal or report.
    pub fn display(&self) -> String {
        let mut out = String::with_capacity(self.0.len());
        for chunk in self.0.utf8_chunks() {
            for c in chunk.valid().chars() {
                if c == '%' || c.is_control() {
                    let mut buf = [0u8; 4];
                    for b in c.encode_utf8(&mut buf).bytes() {
                        out.push_str(&format!("%{b:02X}"));
                    }
                } else {
                    out.push(c);
                }
            }
            for b in chunk.invalid() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    pub fn parse_display(s: &str) -> Result<Self> {
        let mut bytes = Vec::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '%' {
                let hi = chars.next();
                let lo = chars.next();
                let byte = match (hi, lo) {
                    (Some(h), Some(l)) => h
                        .to_digit(16)
                        .zip(l.to_digit(16))
                        .map(|(h, l)| (h * 16 + l) as u8),
                    _ => None,
                }
                .ok_or_else(|| ReviewError::invalid(format!("bad escape in path {s:?}")))?;
                bytes.push(byte);
            } else {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        let path = Self::from_bytes(&bytes)?;
        if path.display() != s {
            return Err(ReviewError::invalid(format!(
                "path {s:?} is not in canonical escaped form"
            )));
        }
        Ok(path)
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoPath({:?})", self.display())
    }
}

impl Serialize for RepoPath {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.display())
    }
}

impl<'de> Deserialize<'de> for RepoPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::parse_display(&String::deserialize(d)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for RepoPath {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("RepoPath")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": REPO_PATH_MAX_BYTES * 3,
            "pattern": "^[^/\\x00-\\x1F\\x7F-\\x9F][^\\x00-\\x1F\\x7F-\\x9F]*$",
            "description": "Repository-relative path; '%', control characters and invalid UTF-8 bytes are %XX-escaped. '.'/'..' components are rejected by the engine."
        })
    }
}

// ---------- time --------------------------------------------------------------

/// Milliseconds since the Unix epoch (UTC).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn now() -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self(ms)
    }
}
