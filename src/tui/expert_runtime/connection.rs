//! Expert runtime connection: hello polling, heartbeat, reissue,
//! and intent draining.

use super::*;

impl ExpertRuntime {
    /// Polls for candidate hello evidence. Only a first valid
    /// session-bound hello locks the gate and starts the interview;
    /// polling without one never changes connection state.
    pub async fn poll_candidate(&mut self, now_ms: u64) -> PollOutcome {
        self.apply_pending_load(now_ms).await;
        if self.started && self.orch.is_connected() {
            return PollOutcome::AlreadyConnected;
        }
        // Persistent rm failure implies broken container exec, so snapshot
        // publish is broken too; expert End/cleanup paths never need hello.
        if self.hello_pending_consume {
            if self.hello.consume_hello().await.is_err() {
                return PollOutcome::Waiting;
            }
            self.hello_pending_consume = false;
        }
        let Some(bytes) = self.hello.read_hello().await else {
            return PollOutcome::Waiting;
        };
        let hello = match parse_hello(&bytes, self.orch.session_id()) {
            Ok(h) => h,
            Err(ControlError::WrongSession) => return PollOutcome::WrongSession,
            Err(_) => return PollOutcome::Waiting,
        };
        if let Err(ControlError::VersionMismatch { candidate, expert }) =
            check_app_versions(hello.app_version.as_deref(), env!("CARGO_PKG_VERSION"))
        {
            self.version_mismatch = Some((candidate, expert));
            return PollOutcome::VersionMismatch;
        }
        if self
            .gate
            .accept_after_ssh_auth(&mut self.orch, now_ms)
            .is_err()
        {
            return PollOutcome::Waiting;
        }
        self.hello_pending_consume = self.hello.consume_hello().await.is_err();
        if !self.started {
            if self.orch.start(now_ms).is_err() {
                return PollOutcome::Waiting;
            }
            self.started = true;
        } else {
            self.reissued = None;
        }
        self.version_mismatch = None;
        self.last_seen_ms = now_ms;
        self.maybe_trigger_load();
        PollOutcome::Connected
    }

    /// Refreshes liveness from a fresh heartbeat; disconnects past the timeout.
    /// Only an advancing candidate `at_ms` counts, so a stale file never masks death.
    pub async fn check_heartbeat(&mut self, now_ms: u64) -> bool {
        if let Some(bytes) = self.hello.read_heartbeat().await {
            if let Ok(hb) = parse_heartbeat(&bytes, self.orch.session_id()) {
                if self.last_hb_at.is_none_or(|prev| hb.at_ms > prev) {
                    self.last_hb_at = Some(hb.at_ms);
                    self.last_seen_ms = now_ms;
                }
            }
        }
        if self.started
            && !self.ended
            && self.orch.is_connected()
            && now_ms.saturating_sub(self.last_seen_ms) >= self.hb_timeout_ms
        {
            let _ = self.gate.handle_disconnect(&mut self.orch, now_ms);
            return true;
        }
        false
    }

    /// Expert 'R': fresh string after disconnect, rotated into the sandbox.
    pub async fn reissue(&mut self, now_ms: u64) -> Result<ConnectionString, ExpertRuntimeError> {
        if !self.is_disconnected() {
            return Err(ExpertRuntimeError::Message(
                "No disconnect to recover".into(),
            ));
        }
        let fresh = self.gate.reissue_after_disconnect(now_ms)?;
        if let Some(handle) = self.sandbox_handle.clone() {
            let user = self.username.clone();
            crate::sandbox::lifecycle::rotate_secret(&handle, &user, fresh.secret())
                .await
                .map_err(|e| ExpertRuntimeError::Message(e.to_string()))?;
        }
        self.reissued = Some(fresh.clone());
        self.pushed_window = Some(fresh.expires_at_ms());
        Ok(fresh)
    }
    pub async fn refresh_pre_lock_secret(
        &mut self,
        now_ms: u64,
    ) -> Result<bool, ExpertRuntimeError> {
        if self.ended || self.orch.is_connected() {
            return Ok(false);
        }
        let Some(pusher) = self.secret_pusher.clone() else {
            return Ok(false);
        };
        let current = self.gate.current_string(now_ms)?;
        if Some(current.expires_at_ms()) == self.pushed_window {
            return Ok(false);
        }
        pusher
            .push(current.secret())
            .await
            .map_err(|_| ExpertRuntimeError::Message("secret refresh push failed".into()))?;
        self.pushed_window = Some(current.expires_at_ms());
        Ok(true)
    }

    /// Rejects a stale presented string against the live reissue window.
    pub fn verify_reconnect(
        &mut self,
        presented: &ConnectionString,
        now_ms: u64,
    ) -> Result<(), ExpertRuntimeError> {
        if !self.is_disconnected() {
            return Err(ExpertRuntimeError::Message(
                "No disconnect to recover".into(),
            ));
        }
        let live = self.gate.current_string(now_ms)?;
        if presented != &live {
            return Err(ExpertRuntimeError::Message(
                "stale connection string".into(),
            ));
        }
        Ok(())
    }

    /// Drains candidate intents once through the control executor, then publishes the snapshot;
    /// heartbeat is checked on a fresh clock before and after the drain, never the stale start time.
    pub async fn drain_intents(
        &mut self,
        now_ms: u64,
    ) -> Result<Vec<ApplyReport>, ExpertRuntimeError> {
        if self.ended {
            return Ok(Vec::new());
        }
        self.check_heartbeat(self.fresh_now(now_ms)).await;
        self.apply_pending_load(now_ms).await;
        let transports = ControlTransports {
            coding: &*self.coding,
            sandbox: &*self.sandbox_io,
            generator: &*self.generator,
            chat: &*self.chat,
        };
        let base = build_snapshot_with_recovery(
            &self.orch,
            &self.pipeline,
            self.sync_phase,
            self.executor.chat_replies(),
            None,
            self.snapshot_recovery(),
        );
        self.executor.set_snapshot_base(base);
        let reports = self
            .executor
            .drain(
                &*self.control_io,
                &mut self.orch,
                &mut self.pipeline,
                &transports,
                now_ms,
            )
            .await?;
        // A Disconnect intent lands here within one drain tick; sync
        // the gate so the monitor shows the disconnect (and the
        // reissue action) immediately, not at the heartbeat timeout.
        if self.started
            && !self.ended
            && !self.orch.is_connected()
            && self.gate.phase() == GatePhase::Locked
        {
            let _ = self.gate.handle_disconnect(&mut self.orch, now_ms);
        }
        for (slot, grade) in self.pipeline.other_grades() {
            self.shadow.record_other(slot, grade);
        }
        let session_id = self.orch.session_id().to_string();
        self.shadow
            .poll(&session_id, self.orch.session(), &self.layout);
        if reports.iter().any(|r| r.outcome == ApplyOutcome::Applied) {
            self.last_seen_ms = self.fresh_now(now_ms);
        }
        let mut snapshot = build_snapshot_with_recovery(
            &self.orch,
            &self.pipeline,
            self.sync_phase,
            self.executor.chat_replies(),
            None,
            self.snapshot_recovery(),
        );
        snapshot.chat_partial = self.executor.chat_partial().map(str::to_string);
        let run = snapshot.latest_run();
        // `latest_run` is stored in a process-wide map, so reading it
        // back off `last_published_snapshot` always sees the run just
        // recorded by `build_snapshot`. Capture it at publish time or
        // a coding-only update (`t` after `r`) is skipped and the
        // candidate stays on "updating tests".
        if self.last_published_snapshot.as_ref() != Some(&snapshot)
            || run != self.last_published_run
        {
            publish_snapshot(&*self.control_io, &snapshot).await?;
            self.last_published_snapshot = Some(snapshot);
            self.last_published_run = run;
        }
        self.maybe_trigger_load();
        self.check_heartbeat(self.fresh_now(now_ms)).await;
        Ok(reports)
    }
}
