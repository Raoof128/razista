//! Changed-hunk context for a snapshot, rebuilt from the stored blobs.
//!
//! Lines are split on `\n` only, exactly as `line_count` counts them, so a
//! stray `\r` cannot shift a citation. A hunk cites the reviewed bytes when it
//! has any reviewed-side lines; a hunk that only removes lines (including a
//! file emptied of content, or a deleted file) cites the base bytes instead.
//! Text is decoded lossily for display; spans always refer to stored bytes.

use similar::{ChangeTag, TextDiff};
use ultra_core::{
    ArtifactId, BoundedVec, ChangeKind, ContextItem, ContextPacket, ContextProvenance, CoverageGap,
    Digest, GapKind, ManifestEntry, Result, Snapshot, SourceSpan, SourceText, Summary, Text,
    TrustLabel,
};

use crate::snapshot::Snapshotter;

const CONTEXT_LINES: usize = 3;
const MAX_ITEMS: usize = 512;
const MAX_OMITTED: usize = 256;

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

impl Snapshotter {
    fn bytes_of(&self, digest: &Digest) -> Result<Vec<u8>> {
        Ok(self
            .store
            .read_artifact_unscoped(&ArtifactId::from(digest.clone()))?
            .0)
    }

    /// One context item per changed hunk. Respects a path-restricted scope.
    pub fn diff(&self, snapshot: &Snapshot) -> Result<ContextPacket> {
        let scope = &snapshot.resolved_scope;
        let in_scope = |e: &ManifestEntry| {
            let path = e.path.as_bytes();
            scope.paths.is_empty()
                || scope.paths.iter().any(|p| {
                    let p = p.as_bytes();
                    path == p || (path.starts_with(p) && path.get(p.len()) == Some(&b'/'))
                })
        };

        let mut items: Vec<ContextItem> = Vec::new();
        let mut omitted: Vec<CoverageGap> = Vec::new();
        let mut omitted_extra = 0usize;
        let mut chars = 0usize;

        for entry in snapshot.manifest.entries.iter().filter(|e| in_scope(e)) {
            // (old bytes, digest the old side cites, new bytes)
            let (old, old_digest, new) = match entry.change {
                ChangeKind::Unchanged => continue,
                ChangeKind::Added => (Vec::new(), None, self.bytes_of(&entry.digest)?),
                ChangeKind::Deleted => (
                    self.bytes_of(&entry.digest)?,
                    Some(entry.digest.clone()),
                    Vec::new(),
                ),
                ChangeKind::Modified | ChangeKind::Renamed => match &entry.base_digest {
                    Some(base) => (
                        self.bytes_of(base)?,
                        Some(base.clone()),
                        self.bytes_of(&entry.digest)?,
                    ),
                    // No comparable base (e.g. it was a submodule): all new.
                    None => (Vec::new(), None, self.bytes_of(&entry.digest)?),
                },
            };
            let (old_text, new_text) =
                (String::from_utf8_lossy(&old), String::from_utf8_lossy(&new));
            let (old_lines, new_lines) = (lines(&old_text), lines(&new_text));
            let diff = TextDiff::from_slices(&old_lines, &new_lines);

            for group in diff.grouped_ops(CONTEXT_LINES) {
                let mut text = String::new();
                let (mut new_lo, mut new_hi) = (usize::MAX, 0usize);
                let (mut old_lo, mut old_hi) = (usize::MAX, 0usize);
                for op in &group {
                    for change in diff.iter_changes(op) {
                        let sign = match change.tag() {
                            ChangeTag::Delete => '-',
                            ChangeTag::Insert => '+',
                            ChangeTag::Equal => ' ',
                        };
                        text.push(sign);
                        text.push_str(change.value());
                        if !change.value().ends_with('\n') {
                            text.push('\n');
                        }
                        if let Some(i) = change.new_index() {
                            new_lo = new_lo.min(i + 1);
                            new_hi = new_hi.max(i + 1);
                        }
                        if let Some(i) = change.old_index() {
                            old_lo = old_lo.min(i + 1);
                            old_hi = old_hi.max(i + 1);
                        }
                    }
                }
                let span_side = if new_lo != usize::MAX {
                    Some((entry.digest.clone(), new_lo, new_hi))
                } else {
                    old_digest.clone().map(|d| (d, old_lo, old_hi))
                };
                let Some((file_digest, start, end)) = span_side else {
                    continue;
                };
                if items.len() >= MAX_ITEMS {
                    if omitted.len() < MAX_OMITTED - 1 {
                        omitted.push(gap(&entry.path.display(), "context item limit reached")?);
                    } else {
                        omitted_extra += 1;
                    }
                    break;
                }
                let (content, truncated) = match SourceText::new(text.clone()) {
                    Ok(t) => (t, false),
                    Err(_) => {
                        let cut: String = text.chars().take(64 * 1024).collect();
                        (SourceText::new(cut)?, true)
                    }
                };
                chars += content.as_str().chars().count();
                items.push(ContextItem {
                    span: SourceSpan {
                        path: entry.path.clone(),
                        file_digest,
                        start_line: u32::try_from(start).unwrap_or(u32::MAX),
                        end_line: u32::try_from(end).unwrap_or(u32::MAX),
                    },
                    trust: TrustLabel::Untrusted,
                    provenance: ContextProvenance::Diff,
                    content,
                    truncated,
                });
            }
        }
        if omitted_extra > 0 {
            omitted.push(gap(
                "context",
                &format!("{omitted_extra} further files were omitted at the item limit"),
            )?);
        }

        Ok(ContextPacket {
            snapshot_id: snapshot.snapshot_id,
            items: BoundedVec::new(items)?,
            omitted: BoundedVec::new(omitted)?,
            token_estimate: u32::try_from(chars / 4).unwrap_or(u32::MAX),
        })
    }
}

fn gap(subject: &str, detail: &str) -> Result<CoverageGap> {
    Ok(CoverageGap {
        kind: GapKind::ContextTruncated,
        subject: Text::new(subject.chars().take(512).collect::<String>())?,
        code: None,
        detail: Summary::new(detail)?,
    })
}
