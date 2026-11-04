//! Expert runtime session end: monitor/final views, final-ack wait,
//! cleanup, and question-load triggering.

use super::*;

impl ExpertRuntime {
    /// Live monitor data from permitted expert-side state only.
    /// Uses the pause-adjusted clock so elapsed freezes during recovery
    /// and excludes recovery duration after resume.
    pub fn monitor_view(&self, now_ms: u64) -> MonitorView {
        monitor_view(
            self.orch.session(),
            &self.pipeline.approach_qualities(&self.orch),
            self.adjusted_now_ms(now_ms),
        )
    }
    /// Final breakdown lines for the expert Final screen, built from
    /// the grades stored by `end_session_with_ack_deadline`.
    pub fn final_lines(&self) -> Vec<String> {
        if self.is_technical_failure() {
            return candidate_final_lines(&CandidateFinal::TechnicalFailure);
        }
        match final_view(
            self.orch.session(),
            &self.pipeline.approach_qualities(&self.orch),
            &self.final_grades,
        ) {
            Ok(view) => candidate_final_lines(&CandidateFinal::Breakdown(view)),
            Err(_) => vec!["session ended.".to_string()],
        }
    }
    /// Ends the session. A live candidate gets the full final-ack wait;
    /// a disconnected one does not, because that ack will never arrive.
    pub async fn end_session(&mut self, now_ms: u64) -> Result<CleanupReport, ExpertRuntimeError> {
        let ack_deadline_ms = if self.is_disconnected() {
            DISCONNECTED_ACK_WAIT_MS
        } else {
            FINAL_ACK_WAIT_MS
        };
        self.end_session_with_ack_deadline(now_ms, ack_deadline_ms)
            .await
    }
    /// Ends the session: publish the final snapshot, wait bounded
    /// for the candidate final ack, then run cleanup. A timeout
    /// proceeds anyway; a second call re-attempts nothing.
    pub async fn end_session_with_ack_deadline(
        &mut self,
        now_ms: u64,
        ack_deadline_ms: u64,
    ) -> Result<CleanupReport, ExpertRuntimeError> {
        if self.ended {
            return Ok(self.cleanup_report.clone().unwrap_or_default());
        }
        self.ended = true;
        if self.is_recovery_active() {
            self.cancel_recovery();
        }
        let sid = self.orch.session_id().to_string();
        self.shadow.poll(&sid, self.orch.session(), &self.layout);
        self.shadow.shutdown();
        let kill = self.supervisor.kill_switch().await;
        self.last_kill = Some(kill);
        if self.gate.phase() != GatePhase::Ended {
            let _ = self.gate.terminate(&mut self.orch, now_ms);
        }
        // A quit (zero ack wait) destroys the sandbox as soon as grading
        // returns. Tell the candidate before that, so the work screen
        // does not stay up until the container is already gone.
        if ack_deadline_ms == 0 {
            let mut closing = build_snapshot_with_recovery(
                &self.orch,
                &self.pipeline,
                self.sync_phase,
                self.executor.chat_replies(),
                None,
                self.snapshot_recovery(),
            );
            if !matches!(
                closing.phase.as_str(),
                "Completed" | "EndedByExpert" | "TechnicalFailure"
            ) {
                closing.phase = "EndedByExpert".to_string();
            }
            let _ = publish_snapshot(&*self.control_io, &closing).await;
        }
        let mut statements: HashMap<String, String> = HashMap::new();
        if let (Some(question), Some(fetched)) = (
            self.orch.current_question().cloned(),
            self.pipeline.current_statement(&self.orch),
        ) {
            statements.insert(question.title_slug.clone(), fetched.statement);
        }
        let codes: HashMap<usize, String> = (0..self.orch.session().slots().len())
            .filter_map(|index| {
                self.pipeline
                    .current_code(index)
                    .map(|code| (index, code.to_string()))
            })
            .collect();
        let (final_grades, final_snapshot, report_md) = if self.is_technical_failure() {
            let snap = build_snapshot_with_recovery(
                &self.orch,
                &self.pipeline,
                self.sync_phase,
                self.executor.chat_replies(),
                None,
                self.snapshot_recovery(),
            );
            (Vec::new(), snap, format!("# PairProof Session Report\n\nSession: {}\n\nStatus: TechnicalFailure\n\nSession ended due to technical failure.\n", self.orch.session_id()))
        } else {
            let grades = grade_session(&self.orch, &*self.grader, &statements, &codes).await?;
            let breakdown = compute_final(
                &self.orch,
                &self.pipeline.approach_qualities(&self.orch),
                &grades,
            )?;
            let snap = build_snapshot_with_recovery(
                &self.orch,
                &self.pipeline,
                self.sync_phase,
                self.executor.chat_replies(),
                Some(&breakdown),
                self.snapshot_recovery(),
            );
            let mut md = generate_report(self.orch.session(), &breakdown, &grades, &[]);
            let shadow_reports: Vec<_> = self.shadow.reports().values().cloned().collect();
            if !shadow_reports.is_empty() {
                md.push_str(&crate::orchestration::report::render_shadow_section(
                    &shadow_reports,
                ));
            }
            (grades, snap, md)
        };
        self.final_grades = final_grades;
        let published = publish_snapshot(&*self.control_io, &final_snapshot).await;
        let ack_outcome = if published.is_ok() {
            Some(self.wait_for_final_ack(ack_deadline_ms).await)
        } else {
            None
        };
        let context = ContextSummary::from_session(
            self.orch.session(),
            self.topics.clone(),
            self.level.clone(),
        );
        let token = self.supervisor.token();
        let gate_ended = self.gate.phase() == GatePhase::Ended;
        let session_id = self.orch.session_id().to_string();
        let handle = self.sandbox_handle.clone();
        let layout = self.layout.clone();
        let sequence = CleanupSequence::new(
            Box::new(move || {
                let token = token.clone();
                Box::pin(async move {
                    token.cancel();
                    Ok(())
                })
            }),
            Box::new(move || {
                Box::pin(async move {
                    if gate_ended {
                        Ok(())
                    } else {
                        Err("connection not terminated".to_string())
                    }
                })
            }),
            Box::new(move || {
                let session_id = session_id.clone();
                Box::pin(async move {
                    match tokio::time::timeout(
                        Duration::from_secs(8),
                        crate::sync::terminate_sync(&session_id),
                    )
                    .await
                    {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(_) => Err("timeout".to_string()),
                    }
                })
            }),
            Box::new(move || {
                let handle = handle.clone();
                Box::pin(async move {
                    match &handle {
                        Some(handle) => crate::sandbox::destroy(handle)
                            .await
                            .map_err(|e| e.to_string()),
                        None => Ok(()),
                    }
                })
            }),
            Box::new(move || {
                let layout = layout.clone();
                Box::pin(async move { layout.delete_raw_dir().map_err(|e| e.to_string()) })
            }),
        );
        let mut coordinator = CleanupCoordinator::new(sequence);
        let result = coordinator
            .finalize_and_cleanup(&self.layout, &self.orch, &report_md, &context, &codes)
            .await;
        if let Some(mapping) = self.port_mapping.take() {
            mapping.close().await;
        }
        if let Some(report) = coordinator.last_report() {
            let mut stored = report.clone();
            if ack_outcome == Some(FinalAckOutcome::Timeout) {
                stored
                    .failed
                    .push(("final_snapshot_ack".into(), "timeout".into()));
            }
            if let Err(err) = &published {
                stored
                    .failed
                    .push(("final_snapshot_publish".into(), err.to_string()));
            }
            self.cleanup_report = Some(stored);
        }
        let mut report = result?;
        if ack_outcome == Some(FinalAckOutcome::Timeout) {
            report
                .failed
                .push(("final_snapshot_ack".into(), "timeout".into()));
        }
        if let Err(err) = published {
            if self.sandbox_handle.is_none() {
                return Err(err.into());
            }
            report
                .failed
                .push(("final_snapshot_publish".into(), err.to_string()));
        }
        self.cleanup_report = Some(report.clone());
        Ok(report)
    }

    /// Polls the ack file every ~250ms until `deadline_ms`; a wrong
    /// session/version ack is ignored. Re-entry never waits twice.
    async fn wait_for_final_ack(&mut self, deadline_ms: u64) -> FinalAckOutcome {
        if self.ack_waited {
            return self.final_ack_outcome.unwrap_or(FinalAckOutcome::Timeout);
        }
        self.ack_waited = true;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(deadline_ms);
        let outcome = loop {
            if let Some(bytes) = self.hello.read_final_ack().await {
                if parse_final_ack(&bytes, self.orch.session_id()).is_ok() {
                    break FinalAckOutcome::Received;
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break FinalAckOutcome::Timeout;
            }
            tokio::time::sleep(
                Duration::from_millis(250).min(deadline.saturating_duration_since(now)),
            )
            .await;
        };
        self.final_ack_outcome = Some(outcome);
        outcome
    }

    pub(crate) fn maybe_trigger_load(&mut self) {
        if self.supervisor.is_killed() || self.supervisor.token().is_cancelled() {
            return;
        }
        let Some(index) = self.orch.current_index() else {
            return;
        };
        if self.loaded_index == Some(index) {
            return;
        }
        if let Some((pending, _)) = &self.pending_load {
            if *pending == index {
                return;
            }
        }
        let Some(question) = self.orch.current_question() else {
            return;
        };
        let slug = question.title_slug.clone();
        let session_id = self.orch.session_id().to_string();
        let language = self.orch.session().language();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let fetcher = self.fetcher.clone();
        let generator = self.generator.clone();
        let token = self.supervisor.token();
        self.supervisor.spawn(format!("load-q{index}"), async move {
            let work = async move {
                let fetched = fetcher.fetch_statement(&slug, language).await?;
                let statement = fetched.statement.clone();
                let set = generator.generate(&statement, language).await?;
                Ok((fetched, set))
            };
            let result = run_cancellable(&token, work).await;
            let _ = tx.send(LoadOutcome {
                session_id,
                index,
                result,
            });
        });
        self.pending_load = Some((index, rx));
    }

    /// Resolves a pending re-question. An approved replacement needs a
    /// fresh statement and approaches, so the current load is cleared.
    pub fn resolve_requestion(
        &mut self,
        approved: bool,
        replacement: Option<QuestionRef>,
        now_ms: u64,
    ) -> Result<(), ExpertRuntimeError> {
        let index = self.orch.current_index();
        self.orch
            .resolve_requestion(approved, replacement, now_ms)?;
        if approved {
            if let Some(index) = index {
                self.pipeline.invalidate_approaches(index);
            }
        }
        self.loaded_index = None;
        self.maybe_trigger_load();
        Ok(())
    }
}
