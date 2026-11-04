use pairproof::core::{CoreError, EventKind, Language, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::approaches::parse_approach_set;
use pairproof::llm::coding::{apply_usage, parse_coding_step, CodingStepInput, CodingStepOutput};
use pairproof::llm::Usage;
use pairproof::orchestration::{
    ApproachGenerator, CodingTransport, FetchedStatement, Orchestrator, PipelineError,
    PipelineFuture, QuestionPipeline, SandboxIo, StatementFetcher,
};
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;

fn started() -> Orchestrator {
    let easy = qs("e", 5, Difficulty::Easy);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy, 0).unwrap();
    o.mark_waiting_for_candidate(1).unwrap();
    o.attach_candidate(2).unwrap();
    o.start(3).unwrap();
    o
}
fn count(orch: &Orchestrator, pred: impl Fn(&EventKind) -> bool) -> usize {
    let events = orch.session().events();
    events.iter().filter(|e| pred(&e.kind)).count()
}
const SET_JSON: &str = r#"{"approaches":[{"title":"Brute force","summary":"Try every pair","quality":"weak","expected_tokens":111},{"title":"Hash map","summary":"One pass lookup","quality":"best","expected_tokens":222},{"title":"Sort first","summary":"Sort then scan","quality":"workable","expected_tokens":333},{"title":"Two pointers","summary":"Converge from ends","quality":"workable","expected_tokens":444}]}"#;
const READY_JSON: &str =
    r#"{"code":"def solve():\n    return 42","ready":true,"summary":"complete"}"#;
const DRAFT_JSON: &str = r#"{"code":"def solve():\n    return 1","ready":false,"summary":"draft"}"#;
struct FakeFetcher;
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let slug = slug.to_string();
        Box::pin(async move {
            Ok(FetchedStatement {
                statement: format!("Statement for {slug}"),
                starter_code: "# starter".into(),
            })
        })
    }
}
struct FakeGenerator;
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            parse_approach_set(SET_JSON, &mut StdRng::seed_from_u64(7))
                .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move {
            ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into())
                .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
#[derive(Default)]
struct FakeCoding {
    calls: AtomicU32,
    inputs: Mutex<Vec<CodingStepInput>>,
    payload: &'static str,
    book: u64,
    end_session: bool,
}
impl FakeCoding {
    fn new(payload: &'static str, book: u64) -> Self {
        Self {
            payload,
            book,
            ..Default::default()
        }
    }
}
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(
        &'a self,
        session: &'a mut Session,
        input: &'a CodingStepInput,
    ) -> PF<'a, CodingStepOutput> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs.lock().unwrap().push(input.clone());
            if self.end_session {
                let _ = session.end_by_expert(9_999);
                return Ok(CodingStepOutput {
                    code: "def solve():\n    return 99".into(),
                    ready: true,
                    summary: "late".into(),
                });
            }
            let usage = Usage {
                prompt_tokens: self.book,
                completion_tokens: 0,
                total_tokens: self.book,
            };
            apply_usage(session, &usage).map_err(|e| PipelineError::Llm(e.to_string()))?;
            parse_coding_step(self.payload).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
#[derive(Default)]
struct FakeSandbox {
    files: Mutex<Vec<(usize, String)>>,
    runnable_calls: AtomicU32,
    test_calls: AtomicU32,
    outcome: Option<TestOutcome>,
}
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, i: usize, _l: Language, code: &'a str) -> PF<'a, ()> {
        Box::pin(async move {
            self.files.lock().unwrap().push((i, code.into()));
            Ok(())
        })
    }
    fn read_solution<'a>(&'a self, i: usize, _l: Language) -> PF<'a, String> {
        Box::pin(async move {
            let files = self.files.lock().unwrap();
            let found = files.iter().rev().find(|(x, _)| *x == i);
            Ok(found.map(|(_, c)| c.clone()).unwrap_or_default())
        })
    }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> {
        Box::pin(async move {
            self.runnable_calls.fetch_add(1, Ordering::SeqCst);
            Ok(RunnabilityReport {
                runnable: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            })
        })
    }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> {
        Box::pin(async move {
            self.test_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.outcome.clone().unwrap_or(TestOutcome {
                success: true,
                exit_code: Some(0),
                stdout: "all ok".into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            }))
        })
    }
}
async fn setup(
    payload: &'static str,
    book: u64,
) -> (Orchestrator, QuestionPipeline, FakeCoding, FakeSandbox) {
    let orch = started();
    let mut pipe = QuestionPipeline::new();
    pipe.load_current_statement(&orch, &FakeFetcher)
        .await
        .unwrap();
    pipe.generate_current_approaches(&orch, &FakeGenerator)
        .await
        .unwrap();
    let coding = FakeCoding::new(payload, book);
    let sandbox = FakeSandbox::default();
    (orch, pipe, coding, sandbox)
}
#[tokio::test]
async fn coding_step_writes_code_as_workspace_data() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 40).await;
    let outcome = pipe
        .select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    assert_eq!(outcome.code, "def solve():\n    return 1");
    assert_eq!(
        sandbox.files.lock().unwrap().as_slice(),
        [(0, "def solve():\n    return 1".to_string())]
    );
    assert_eq!(pipe.current_code(0), Some("def solve():\n    return 1"));
    let input = &coding.inputs.lock().unwrap()[0];
    assert_eq!(input.question_statement, "Statement for e0");
    assert_eq!(input.current_code, "# starter");
    assert!(!input.tests_requested);
}
#[tokio::test]
async fn coding_loop_checks_runnability_without_running_tests() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 40).await;
    let outcome = pipe
        .select_preset(&mut orch, 1, &coding, &sandbox, 10)
        .await
        .unwrap();
    assert!(outcome.runnable);
    assert_eq!(sandbox.runnable_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sandbox.test_calls.load(Ordering::SeqCst), 0);
    let n = count(&orch, |k| matches!(k, EventKind::TestsRequested { .. }));
    assert_eq!(n, 0);
    // Redirect and continue each run exactly one further coding step.
    pipe.continue_coding(&mut orch, &coding, &sandbox, 11)
        .await
        .unwrap();
    pipe.redirect_coding(&mut orch, "use a heap", &coding, &sandbox, 12)
        .await
        .unwrap();
    assert_eq!(coding.calls.load(Ordering::SeqCst), 3);
    let inputs = coding.inputs.lock().unwrap();
    assert_eq!(inputs[1].instruction, None);
    assert_eq!(inputs[2].instruction.as_deref(), Some("use a heap"));
    let n = count(&orch, |k| matches!(k, EventKind::Redirected { .. }));
    assert_eq!(n, 1);
}
#[tokio::test]
async fn coding_loop_stops_on_ai_ready() {
    let (mut orch, mut pipe, coding, sandbox) = setup(READY_JSON, 40).await;
    let outcome = pipe
        .select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    assert!(outcome.ready);
    assert_eq!(coding.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn coding_loop_stops_on_candidate_action() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 10).await;
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    pipe.finish_question(&mut orch, 20).unwrap();
    assert_eq!(orch.current_index(), Some(1));
    assert!(pipe.candidate_options().is_empty());
    assert!(!pipe.has_current_approaches(&orch));
    let err = pipe
        .select_preset(&mut orch, 0, &coding, &sandbox, 21)
        .await
        .expect_err("stale approaches");
    assert_eq!(err, PipelineError::NoApproaches);
    assert_eq!(coding.calls.load(Ordering::SeqCst), 1);
    pipe.skip_question(&mut orch, 22).unwrap();
    assert_eq!(orch.current_index(), Some(2));
}
#[tokio::test]
async fn coding_loop_stops_on_expert_end() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 10).await;
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    orch.end_by_expert(30).unwrap();
    let err = pipe
        .continue_coding(&mut orch, &coding, &sandbox, 31)
        .await
        .expect_err("ended");
    assert_eq!(err, PipelineError::Core(CoreError::Terminal.into()));
    assert_eq!(coding.calls.load(Ordering::SeqCst), 1);
    assert_eq!(sandbox.files.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn request_tests_runs_real_sandbox_test_wrapper() {
    let (mut orch, mut pipe, coding, _) = setup(DRAFT_JSON, 10).await;
    let raw = TestOutcome {
        success: false,
        exit_code: Some(1),
        stdout: "1 passed, 1 failed in output text".into(),
        stderr: "assert boom".into(),
        stdout_truncated: true,
        stderr_truncated: false,
    };
    let sandbox = FakeSandbox {
        outcome: Some(raw.clone()),
        ..Default::default()
    };
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    assert_eq!(sandbox.test_calls.load(Ordering::SeqCst), 0);
    let record = pipe.request_tests(&mut orch, &sandbox, 20).await.unwrap();
    assert_eq!(sandbox.test_calls.load(Ordering::SeqCst), 1);
    assert_eq!(record.outcome, raw);
    let n = count(&orch, |k| matches!(k, EventKind::TestsRequested { .. }));
    assert_eq!(n, 1);
}
#[tokio::test]
async fn test_results_are_recorded_once_and_displayed_verbatim() {
    let (mut orch, mut pipe, coding, _) = setup(DRAFT_JSON, 10).await;
    let raw = TestOutcome {
        success: true,
        exit_code: Some(0),
        stdout: "raw stdout: 3 ok".into(),
        stderr: "raw stderr".into(),
        stdout_truncated: false,
        stderr_truncated: true,
    };
    let sandbox = FakeSandbox {
        outcome: Some(raw.clone()),
        ..Default::default()
    };
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    let record = pipe.request_tests(&mut orch, &sandbox, 20).await.unwrap();
    assert_eq!(pipe.test_records(), std::slice::from_ref(&record));
    assert_eq!(record.outcome.stdout, "raw stdout: 3 ok");
    assert_eq!(orch.session().current_slot().unwrap().latest_test, None);
    let n = count(&orch, |k| matches!(k, EventKind::TestRunCompleted { .. }));
    assert_eq!(n, 0);
    pipe.record_counted_test_result(&mut orch, 3, 0, 21)
        .unwrap();
    let slot = orch.session().current_slot().unwrap();
    let latest = slot.latest_test.map(|t| (t.passed, t.failed));
    assert_eq!(latest, Some((3, 0)));
    let n = count(&orch, |k| matches!(k, EventKind::TestRunCompleted { .. }));
    assert_eq!(n, 1);
}
#[tokio::test]
async fn coding_api_usage_is_booked_exactly_once_on_success_and_failure() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 40).await;
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    assert_eq!(orch.session().current_slot().unwrap().actual_tokens, 40);
    let (mut orch, mut pipe, failing, sandbox) = setup("not json at all", 25).await;
    let err = pipe
        .select_preset(&mut orch, 0, &failing, &sandbox, 10)
        .await
        .expect_err("parse fails");
    assert!(matches!(err, PipelineError::Llm(_)), "{err:?}");
    assert_eq!(orch.session().current_slot().unwrap().actual_tokens, 25);
    assert!(sandbox.files.lock().unwrap().is_empty());
}
#[tokio::test]
async fn late_llm_response_cannot_mutate_ended_session() {
    let (mut orch, mut pipe, coding, sandbox) = setup(DRAFT_JSON, 10).await;
    pipe.select_preset(&mut orch, 0, &coding, &sandbox, 10)
        .await
        .unwrap();
    let tokens_before = orch.session().current_slot().unwrap().actual_tokens;
    let files_before = sandbox.files.lock().unwrap().len();
    let late = FakeCoding {
        end_session: true,
        ..FakeCoding::new(READY_JSON, 500)
    };
    let err = pipe
        .continue_coding(&mut orch, &late, &sandbox, 20)
        .await
        .expect_err("late response refused");
    assert_eq!(err, PipelineError::Core(CoreError::Terminal.into()));
    assert_eq!(sandbox.files.lock().unwrap().len(), files_before);
    let tokens = orch.session().current_slot().unwrap().actual_tokens;
    assert_eq!(tokens, tokens_before);
    assert_eq!(pipe.current_code(0), Some("def solve():\n    return 1"));
}
struct DirectTokenCoding;
impl CodingTransport for DirectTokenCoding {
    fn coding_step<'a>(
        &'a self,
        session: &'a mut Session,
        _input: &'a CodingStepInput,
    ) -> PF<'a, CodingStepOutput> {
        Box::pin(async move {
            session
                .add_tokens(37)
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(CodingStepOutput {
                code: "def solve():\n    return 7".into(),
                ready: false,
                summary: "draft".into(),
            })
        })
    }
}
#[tokio::test]
async fn coding_step_books_tokens_once_on_real_session() {
    let (mut orch, mut pipe, _coding, sandbox) = setup(DRAFT_JSON, 0).await;
    let direct = DirectTokenCoding;
    pipe.select_preset(&mut orch, 0, &direct, &sandbox, 10)
        .await
        .unwrap();
    assert_eq!(orch.session().current_slot().unwrap().actual_tokens, 37);
    pipe.continue_coding(&mut orch, &direct, &sandbox, 11)
        .await
        .unwrap();
    assert_eq!(orch.session().current_slot().unwrap().actual_tokens, 74);
}

#[tokio::test]
async fn full_iterative_question_lifecycle_and_final_submission_boundary() {
    use pairproof::core::QuestionStatus;

    let (mut orch, mut pipe, _coding, sandbox) = setup(DRAFT_JSON, 10).await;
    let step_coding = FakeCoding::new(
        r#"{"code":"def solve(nums):\n    return sum(nums)","ready":false,"summary":"iter"}"#,
        15,
    );

    // 1-4. Select approach and generate initial solution; not auto-executed or submitted.
    let initial = pipe
        .select_preset(&mut orch, 1, &step_coding, &sandbox, 10)
        .await
        .unwrap();
    assert_eq!(initial.code, "def solve(nums):\n    return sum(nums)");
    assert_eq!(sandbox.test_calls.load(Ordering::SeqCst), 0);
    assert_eq!(orch.current_index(), Some(0));
    assert_eq!(
        orch.session().slot(0).unwrap().status,
        QuestionStatus::Active
    );
    assert_eq!(pipe.code_revision(0), 1);

    // 5-6. Request additional changes (`c` / redirect); question & context are retained.
    let iter2_coding = FakeCoding::new(
        r#"{"code":"def solve(nums):\n    if not nums:\n        return 0\n    return sum(nums)","ready":false,"summary":"edge case"}"#,
        15,
    );
    pipe.redirect_coding(
        &mut orch,
        "handle empty list edge case",
        &iter2_coding,
        &sandbox,
        20,
    )
    .await
    .unwrap();
    {
        let inputs = iter2_coding.inputs.lock().unwrap();
        assert_eq!(inputs[0].question_statement, "Statement for e0");
        assert!(!inputs[0].approach_text.is_empty());
        assert_eq!(
            inputs[0].current_code,
            "def solve(nums):\n    return sum(nums)"
        );
        assert_eq!(
            inputs[0].instruction.as_deref(),
            Some("handle empty list edge case")
        );
        assert!(!inputs[0].tests_requested);
    }
    assert_eq!(pipe.code_revision(0), 2);
    assert_eq!(orch.current_index(), Some(0));

    // 7. Request test generation; `tests_requested` becomes true and stays sticky for slot 0.
    let test_gen_coding = FakeCoding::new(
        r#"{"code":"def solve(nums):\n    return sum(nums)\nif __name__ == '__main__':\n    assert solve([]) == 0","ready":true,"summary":"with tests"}"#,
        15,
    );
    pipe.redirect_coding(
        &mut orch,
        "generate unit tests for empty list",
        &test_gen_coding,
        &sandbox,
        30,
    )
    .await
    .unwrap();
    assert!(test_gen_coding.inputs.lock().unwrap()[0].tests_requested);
    assert_eq!(pipe.code_revision(0), 3);

    // 8-9. Run solution and execute tests independently.
    let run_rec = pipe.run_solution(&mut orch, &sandbox, 40).await.unwrap();
    assert!(run_rec.outcome.success);
    assert_eq!(pipe.run_records().len(), 1);
    assert_eq!(pipe.test_records().len(), 0);
    assert_eq!(
        count(&orch, |k| matches!(k, EventKind::TestsRequested { .. })),
        0
    );

    let test_rec = pipe.request_tests(&mut orch, &sandbox, 50).await.unwrap();
    assert!(test_rec.outcome.success);
    assert_eq!(pipe.test_records().len(), 1);
    assert_eq!(
        count(&orch, |k| matches!(k, EventKind::TestsRequested { .. })),
        1
    );
    assert_eq!(
        orch.current_index(),
        Some(0),
        "passing tests must not auto-submit"
    );

    // 10-13. Candidate edits file in workspace, then continues with agent; edits and test flag are preserved.
    sandbox
        .write_solution(
            0,
            Language::Python,
            "def solve(nums):\n    # candidate manual edit\n    return sum(nums)\n",
        )
        .await
        .unwrap();
    let iter3_coding = FakeCoding::new(
        r#"{"code":"def solve(nums):\n    # candidate manual edit + refactored\n    return sum(nums)","ready":true,"summary":"refactored"}"#,
        15,
    );
    pipe.redirect_coding(
        &mut orch,
        "refactor implementation",
        &iter3_coding,
        &sandbox,
        60,
    )
    .await
    .unwrap();
    {
        let inputs = iter3_coding.inputs.lock().unwrap();
        assert!(
            inputs[0].current_code.contains("# candidate manual edit"),
            "agent must read candidate's workspace edit: {}",
            inputs[0].current_code
        );
        assert!(
            inputs[0].tests_requested,
            "test generation flag must remain preserved across iterations"
        );
    }

    // Candidate makes one final manual tweak in workspace before pressing `f`.
    let final_candidate_code =
        "def solve(nums):\n    return sum(nums)  # final candidate solution\n";
    sandbox
        .write_solution(0, Language::Python, final_candidate_code)
        .await
        .unwrap();

    // 14-17. Final submission (`f`) captures workspace solution, marks Q0 Finished, advances to Q1, and locks Q0.
    pipe.finish_question_with_sandbox(&mut orch, &sandbox, 70)
        .await
        .unwrap();
    assert!(pipe.is_submitted(0));
    assert_eq!(pipe.current_code(0), Some(final_candidate_code));
    assert_eq!(
        orch.session().slot(0).unwrap().status,
        QuestionStatus::Finished
    );
    assert_eq!(orch.current_index(), Some(1));

    // Previous question (slot 0) cannot be reopened or modified.
    assert!(pipe.navigate_question(&mut orch, 0, 71).is_err());
    assert!(orch.navigate_to(0, 72).is_err());
    assert_eq!(orch.current_index(), Some(1));
    assert_eq!(pipe.current_code(0), Some(final_candidate_code));
}
