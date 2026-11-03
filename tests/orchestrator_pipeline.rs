use pairproof::core::{ApproachChoice, EventKind, Language, Session};
use pairproof::leetcode::{CodeSnippet, Difficulty, Question};
use pairproof::llm::approaches::{parse_approach_grade, parse_approach_set};
use pairproof::llm::coding::{parse_coding_step, CodingStepInput, CodingStepOutput};
use pairproof::orchestration::{
    fetched_from_question, ApproachGenerator, CodingTransport, FetchedStatement, Orchestrator,
    PipelineError, PipelineFuture, QuestionPipeline, SandboxIo, StatementFetcher,
};
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachSet};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::sync::Mutex;

mod common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;

fn started(lang: Language) -> Orchestrator {
    let mut o = Orchestrator::create("s1".into(), lang, qs("e", 5, Difficulty::Easy), 0).unwrap();
    o.mark_waiting_for_candidate(1).unwrap();
    o.attach_candidate(2).unwrap();
    o.start(3).unwrap();
    o
}
const SET_JSON: &str = r#"{"approaches":[{"title":"Brute force","summary":"Try every pair","quality":"weak","expected_tokens":111},{"title":"Hash map","summary":"One pass lookup","quality":"best","expected_tokens":222},{"title":"Sort first","summary":"Sort then scan","quality":"workable","expected_tokens":333},{"title":"Two pointers","summary":"Converge from ends","quality":"workable","expected_tokens":444}]}"#;
const GRADE_JSON: &str = r#"{"quality":"workable","expected_tokens":250,"rationale":"sound"}"#;
const CODE_JSON: &str = r#"{"code":"def solve():\n    return 42","ready":false,"summary":"draft"}"#;
#[derive(Debug, Default)]
struct FakeFetcher {
    calls: Mutex<Vec<(String, Language)>>,
    fail: bool,
}
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, lang: Language) -> PF<'a, FetchedStatement> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((slug.into(), lang));
            if self.fail {
                return Err(PipelineError::Fetch("boom".into()));
            }
            let starter = match lang {
                Language::Python => "class Solution: pass",
                Language::JavaScript => "var solve = function() {};",
                Language::Sql => "SELECT 1;",
            };
            Ok(FetchedStatement {
                statement: format!("Statement for {slug}"),
                starter_code: starter.into(),
            })
        })
    }
}
#[derive(Debug, Default)]
struct FakeGenerator {
    generate_calls: Mutex<Vec<(String, Language)>>,
    grade_fails: bool,
}
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, stmt: &'a str, lang: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            self.generate_calls
                .lock()
                .unwrap()
                .push((stmt.into(), lang));
            parse_approach_set(SET_JSON, &mut StdRng::seed_from_u64(7))
                .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move {
            if self.grade_fails {
                return Err(PipelineError::Llm("grade unavailable".into()));
            }
            parse_approach_grade(GRADE_JSON).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
#[derive(Debug, Default)]
struct FakeCoding {
    calls: Mutex<u32>,
}
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(
        &'a self,
        s: &'a mut Session,
        _i: &'a CodingStepInput,
    ) -> PF<'a, CodingStepOutput> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            s.add_tokens(40)
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            parse_coding_step(CODE_JSON).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
#[derive(Debug, Default)]
struct FakeSandbox {
    files: Mutex<Vec<(usize, String)>>,
}
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, i: usize, _l: Language, code: &'a str) -> PF<'a, ()> {
        Box::pin(async move {
            self.files.lock().unwrap().push((i, code.into()));
            Ok(())
        })
    }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> {
        Box::pin(async move { Ok(String::new()) })
    }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> {
        Box::pin(async move {
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
            Ok(TestOutcome {
                success: true,
                exit_code: Some(0),
                stdout: "ok".into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        })
    }
}
fn sample_question() -> Question {
    Question {
        question_id: "1".into(),
        title: "Two Sum".into(),
        title_slug: "two-sum".into(),
        difficulty: "Easy".into(),
        content: "<p>stmt</p>".into(),
        example_testcases: String::new(),
        is_paid_only: false,
        topic_tags: vec![],
        code_snippets: vec![
            CodeSnippet {
                lang: "Python3".into(),
                lang_slug: "python3".into(),
                code: "py-starter".into(),
            },
            CodeSnippet {
                lang: "JavaScript".into(),
                lang_slug: "javascript".into(),
                code: "js-starter".into(),
            },
        ],
    }
}
async fn loaded_pipe(orch: &Orchestrator) -> (QuestionPipeline, FakeGenerator) {
    let mut pipe = QuestionPipeline::new();
    pipe.load_current_statement(orch, &FakeFetcher::default())
        .await
        .unwrap();
    (pipe, FakeGenerator::default())
}
#[tokio::test]
async fn question_fetch_uses_selected_language_and_difficulty() {
    let question = sample_question();
    assert_eq!(
        fetched_from_question(&question, Language::JavaScript).starter_code,
        "js-starter"
    );
    assert_eq!(
        fetched_from_question(&question, Language::Python).starter_code,
        "py-starter"
    );
    assert_eq!(
        fetched_from_question(&question, Language::Sql).starter_code,
        ""
    );
    assert_eq!(
        fetched_from_question(&question, Language::Sql).statement,
        "<p>stmt</p>"
    );
    let orch = started(Language::JavaScript);
    assert_eq!(
        orch.current_question().unwrap().difficulty,
        Difficulty::Easy
    );
    let fetcher = FakeFetcher::default();
    let mut pipe = QuestionPipeline::new();
    let fetched = pipe
        .load_current_statement(&orch, &fetcher)
        .await
        .expect("fetch");
    assert_eq!(fetched.statement, "Statement for e0");
    assert_eq!(fetched.starter_code, "var solve = function() {};");
    assert_eq!(
        fetcher.calls.lock().unwrap().as_slice(),
        [("e0".to_string(), Language::JavaScript)]
    );
    assert_eq!(pipe.current_statement(&orch), Some(fetched));
}
#[tokio::test]
async fn question_fetch_failure_does_not_advance_session() {
    let orch = started(Language::Python);
    let events_before = orch.session().events().len();
    let fetcher = FakeFetcher {
        fail: true,
        ..Default::default()
    };
    let mut pipe = QuestionPipeline::new();
    let err = pipe.load_current_statement(&orch, &fetcher).await;
    assert_eq!(err, Err(PipelineError::Fetch("boom".into())));
    assert_eq!(orch.session().events().len(), events_before);
    assert_eq!(orch.current_index(), Some(0));
    assert_eq!(pipe.current_statement(&orch), None);
}
#[tokio::test]
async fn question_order_matches_expert_selection() {
    let mut orch = Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .unwrap();
    orch.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        1,
    )
    .unwrap();
    orch.mark_waiting_for_candidate(2).unwrap();
    orch.attach_candidate(3).unwrap();
    orch.start(4).unwrap();
    for i in 0..5u64 {
        orch.finish_question(10 + i).unwrap();
    }
    assert_eq!(orch.current_index(), Some(5));
    assert_eq!(orch.current_question().unwrap().title_slug, "m0");
    let fetcher = FakeFetcher::default();
    let mut pipe = QuestionPipeline::new();
    let fetched = pipe.load_current_statement(&orch, &fetcher).await.unwrap();
    assert_eq!(fetched.statement, "Statement for m0");
    assert_eq!(
        fetcher.calls.lock().unwrap().as_slice(),
        [("m0".to_string(), Language::Python)]
    );
}
#[tokio::test]
async fn approach_generation_uses_phase4_contract() {
    let orch = started(Language::Python);
    let (mut pipe, generator) = loaded_pipe(&orch).await;
    let views = pipe
        .generate_current_approaches(&orch, &generator)
        .await
        .expect("generate");
    assert_eq!(views.len(), 4);
    assert_eq!(
        generator.generate_calls.lock().unwrap().as_slice(),
        [("Statement for e0".to_string(), Language::Python)]
    );
    let mut titles: Vec<&str> = views.iter().map(|v| v.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(
        titles,
        ["Brute force", "Hash map", "Sort first", "Two pointers"]
    );
    assert!(pipe.has_current_approaches(&orch));
}
#[tokio::test]
async fn candidate_receives_only_public_approach_options() {
    let orch = started(Language::Python);
    let (mut pipe, generator) = loaded_pipe(&orch).await;
    pipe.generate_current_approaches(&orch, &generator)
        .await
        .unwrap();
    let views = pipe.candidate_options();
    assert_eq!(views.len(), 4);
    let debug = format!("{views:?}");
    for hidden in [
        "111",
        "222",
        "333",
        "444",
        "Best",
        "Workable",
        "Weak",
        "best_index",
    ] {
        assert!(!debug.contains(hidden), "leaked {hidden}: {debug}");
    }
}
#[tokio::test]
async fn hidden_best_approach_and_expected_tokens_never_reach_candidate() {
    let orch = started(Language::Python);
    let (mut pipe, generator) = loaded_pipe(&orch).await;
    let views = pipe
        .generate_current_approaches(&orch, &generator)
        .await
        .unwrap();
    assert_eq!(views, pipe.candidate_options());
    let debug = format!("{:?}", pipe.candidate_options());
    assert!(!debug.contains("222"), "best tokens leaked: {debug}");
    assert!(!debug.contains("best"), "best marker leaked: {debug}");
    let coding = FakeCoding::default();
    let sandbox = FakeSandbox::default();
    let mut orch2 = orch;
    let err = pipe
        .select_preset(&mut orch2, 9, &coding, &sandbox, 10)
        .await
        .expect_err("bad preset");
    assert_eq!(err, PipelineError::InvalidPreset);
    assert_eq!(*coding.calls.lock().unwrap(), 0);
    assert!(sandbox.files.lock().unwrap().is_empty());
}
#[tokio::test]
async fn approach_selection_locks_tokens_and_resubmission_relocks() {
    let mut orch = started(Language::Python);
    let (mut pipe, generator) = loaded_pipe(&orch).await;
    let views = pipe
        .generate_current_approaches(&orch, &generator)
        .await
        .unwrap();
    let preset = views.iter().position(|v| v.title == "Hash map").unwrap() as u8;
    let coding = FakeCoding::default();
    let sandbox = FakeSandbox::default();
    let outcome = pipe
        .select_preset(&mut orch, preset, &coding, &sandbox, 10)
        .await
        .expect("select and code");
    assert!(outcome.runnable);
    assert_eq!(orch.session().token_locks(0), vec![222]);
    let slot = orch.session().current_slot().unwrap();
    assert_eq!(slot.locked_expected_tokens, Some(222));
    assert_eq!(
        orch.session().current_slot().unwrap().approach,
        Some(ApproachChoice::Preset(preset))
    );
    let selected = orch
        .session()
        .events()
        .iter()
        .filter(|e| matches!(e.kind, EventKind::ApproachSelected { .. }))
        .count();
    assert_eq!(selected, 1);
    assert_eq!(*coding.calls.lock().unwrap(), 1);
    // Submitting again over an existing selection is a change of
    // approach: the old lock is freed and the new choice locks again.
    let again = pipe
        .select_preset(&mut orch, preset, &coding, &sandbox, 11)
        .await;
    assert!(again.is_ok(), "{again:?}");
    assert_eq!(orch.session().token_locks(0), vec![222, 222]);
    assert_eq!(*coding.calls.lock().unwrap(), 2);
}
#[tokio::test]
async fn select_other_grades_first_and_locks_grade_tokens() {
    let mut orch = started(Language::Python);
    let (mut pipe, generator) = loaded_pipe(&orch).await;
    let coding = FakeCoding::default();
    let sandbox = FakeSandbox::default();
    pipe.select_other(&mut orch, "use a heap", &generator, &coding, &sandbox, 10)
        .await
        .expect("other select");
    assert_eq!(orch.session().token_locks(0), vec![250]);
    assert_eq!(
        orch.session().current_slot().unwrap().approach,
        Some(ApproachChoice::Other("use a heap".into()))
    );
    let mut orch = started(Language::Python);
    let (mut pipe, _) = loaded_pipe(&orch).await;
    let failing = FakeGenerator {
        grade_fails: true,
        ..Default::default()
    };
    let err = pipe
        .select_other(&mut orch, "use a heap", &failing, &coding, &sandbox, 10)
        .await
        .expect_err("grade fails");
    assert_eq!(err, PipelineError::Llm("grade unavailable".into()));
    assert_eq!(orch.session().current_slot().unwrap().approach, None);
    assert!(orch.session().token_locks(0).is_empty());
}
