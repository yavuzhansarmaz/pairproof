//! Section-15 artifact bundle and idempotent cleanup coordination.
//! The bundle is written first; cleanup steps run afterwards and can
//! never mask a primary bundle or copy failure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::{ArtifactLayout, ContextSummary, EventLogWriter};
use crate::orchestration::coding::SandboxIo;
use crate::orchestration::finalize::FinalizeError;
use crate::orchestration::lifecycle::{CleanupReport, CleanupSequence, CleanupStep};
use crate::orchestration::session::Orchestrator;

/// Writes the section-15 bundle, then deletes the raw directory.
pub fn write_bundle(
    layout: &ArtifactLayout,
    orch: &Orchestrator,
    report_md: &str,
    context: &ContextSummary,
    codes: &HashMap<usize, String>,
) -> Result<(), FinalizeError> {
    let session = orch.session();
    if let Ok(meta) = std::fs::metadata(layout.events_path()) {
        if meta.len() > 0 {
            return Err(FinalizeError::Artifact(
                "events log already exists and is not empty".into(),
            ));
        }
    }
    let mut writer = EventLogWriter::create(layout.events_path())?;
    writer.write_pending(session)?;
    drop(writer);
    layout.write_token_records(session)?;
    layout.write_context(context)?;
    layout.write_report(report_md)?;
    let mut slots: Vec<usize> = codes.keys().copied().collect();
    slots.sort_unstable();
    for slot in slots {
        if let Some(code) = codes.get(&slot) {
            layout.write_solution(slot, code)?;
        }
    }
    layout.delete_raw_dir()?;
    Ok(())
}

/// Namespace for the section-15 bundle writer.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArtifactBundle;

impl ArtifactBundle {
    /// Writes the bundle via [`write_bundle`].
    pub fn write_bundle(
        layout: &ArtifactLayout,
        orch: &Orchestrator,
        report_md: &str,
        context: &ContextSummary,
        codes: &HashMap<usize, String>,
    ) -> Result<(), FinalizeError> {
        write_bundle(layout, orch, report_md, context, codes)
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), FinalizeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Fails if `raw/` remains under the session dir or any canary byte
/// string appears in any artifact file. The error never echoes a canary.
pub fn assert_artifacts_clean(dir: &Path, canaries: &[&str]) -> Result<(), FinalizeError> {
    if dir.join("raw").exists() {
        return Err(FinalizeError::Artifact(
            "raw artifacts remain in the session directory".into(),
        ));
    }
    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();
    for path in files {
        let bytes = std::fs::read(&path)?;
        for canary in canaries {
            if canary.is_empty() {
                continue;
            }
            if bytes.windows(canary.len()).any(|w| w == canary.as_bytes()) {
                return Err(FinalizeError::Artifact(format!(
                    "forbidden content in {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Copies solution code for `slots` out of the sandbox, in slot order.
pub async fn collect_solution_codes(
    orch: &Orchestrator,
    sandbox: &dyn SandboxIo,
    slots: &[usize],
) -> Result<HashMap<usize, String>, FinalizeError> {
    let language = orch.session().language();
    let mut ordered: Vec<usize> = slots.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    let mut codes = HashMap::new();
    for slot in ordered {
        let code = sandbox
            .read_solution(slot, language)
            .await
            .map_err(|_| FinalizeError::Io("sandbox solution copy failed".into()))?;
        codes.insert(slot, code);
    }
    Ok(codes)
}

/// Coordinates one idempotent finalize: bundle first, then the
/// injected cleanup sequence. A second finalize rewrites nothing.
pub struct CleanupCoordinator {
    sequence: CleanupSequence,
    finalized: bool,
    last_report: Option<CleanupReport>,
}

impl std::fmt::Debug for CleanupCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CleanupCoordinator")
            .field("finalized", &self.finalized)
            .finish_non_exhaustive()
    }
}

impl CleanupCoordinator {
    /// Creates a coordinator wrapping an ordered cleanup sequence.
    pub fn new(sequence: CleanupSequence) -> Self {
        Self {
            sequence,
            finalized: false,
            last_report: None,
        }
    }

    /// Builds the production-order sequence from injected closures.
    /// Real Docker/Mutagen/connection wiring is supplied by callers.
    pub fn production_steps(
        cancel_inflight: CleanupStep,
        terminate_connection: CleanupStep,
        stop_sync: CleanupStep,
        destroy_sandbox: CleanupStep,
        delete_raw: CleanupStep,
    ) -> CleanupSequence {
        CleanupSequence::new(
            cancel_inflight,
            terminate_connection,
            stop_sync,
            destroy_sandbox,
            delete_raw,
        )
    }

    /// Creates a coordinator from injected production-order closures.
    pub fn with_production_steps(
        cancel_inflight: CleanupStep,
        terminate_connection: CleanupStep,
        stop_sync: CleanupStep,
        destroy_sandbox: CleanupStep,
        delete_raw: CleanupStep,
    ) -> Self {
        Self::new(Self::production_steps(
            cancel_inflight,
            terminate_connection,
            stop_sync,
            destroy_sandbox,
            delete_raw,
        ))
    }

    /// Returns whether finalize has already been attempted.
    pub fn is_finalized(&self) -> bool {
        self.finalized
    }

    /// Returns the most recent cleanup report, if any.
    pub fn last_report(&self) -> Option<&CleanupReport> {
        self.last_report.as_ref()
    }

    /// Writes the bundle first, then runs cleanup steps. A bundle
    /// failure still attempts every cleanup step and is returned
    /// unmasked; cleanup failures are collected in the report.
    pub async fn finalize_and_cleanup(
        &mut self,
        layout: &ArtifactLayout,
        orch: &Orchestrator,
        report_md: &str,
        context: &ContextSummary,
        codes: &HashMap<usize, String>,
    ) -> Result<CleanupReport, FinalizeError> {
        if self.finalized {
            return Err(FinalizeError::AlreadyFinalized);
        }
        self.finalized = true;
        let primary = write_bundle(layout, orch, report_md, context, codes);
        let report = self.sequence.run().await;
        self.last_report = Some(report.clone());
        primary?;
        Ok(report)
    }

    /// Collects solutions from the sandbox, writes the bundle, then
    /// runs cleanup. A copy or bundle failure still attempts cleanup
    /// (including sandbox destruction) and stays the primary error.
    pub async fn finalize_and_cleanup_with_sandbox(
        &mut self,
        layout: &ArtifactLayout,
        orch: &Orchestrator,
        report_md: &str,
        context: &ContextSummary,
        sandbox: &dyn SandboxIo,
        slots: &[usize],
    ) -> Result<CleanupReport, FinalizeError> {
        if self.finalized {
            return Err(FinalizeError::AlreadyFinalized);
        }
        self.finalized = true;
        let primary = match collect_solution_codes(orch, sandbox, slots).await {
            Ok(codes) => write_bundle(layout, orch, report_md, context, &codes),
            Err(e) => Err(e),
        };
        let report = self.sequence.run().await;
        self.last_report = Some(report.clone());
        primary?;
        Ok(report)
    }
}
