//! Mutations must report two independent facts: whether the model closure
//! applied, and what evaluation did. The failure this defends against is a
//! failed deferred mutation being labelled `Deferred` — it owns no deferred
//! evaluation, so the panel would claim work that never happens.

use crate::Owner;
use crate::model::frontend_model::{EvaluationMode, mutate, try_mutate};
use crate::perf::{EvaluationOutcome, MutationOutcome, PerfTimings};
use crate::state::ModelStore;
use ironcalc_base::UserModel;
use leptos::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn successful_immediate_carries_its_duration() {
    assert_eq!(
        EvaluationOutcome::of_call(true, false, 12.5),
        EvaluationOutcome::Measured { ms: 12.5 }
    );
}

#[wasm_bindgen_test]
fn successful_deferred_reports_no_duration() {
    assert_eq!(
        EvaluationOutcome::of_call(true, true, 12.5),
        EvaluationOutcome::Deferred,
        "a deferred mutation has not evaluated yet, so no duration exists"
    );
}

#[wasm_bindgen_test]
fn failed_mutation_never_reports_evaluation() {
    for deferred in [true, false] {
        assert_eq!(
            EvaluationOutcome::of_call(false, deferred, 12.5),
            EvaluationOutcome::NotRun,
            "a failed closure skips evaluate() in both modes"
        );
    }
}

/// The two facts a sample carries stay independent: an error keeps its own
/// outcome label while evaluation stays `NotRun`.
#[wasm_bindgen_test]
fn mutation_outcome_is_independent_of_evaluation() {
    assert_eq!(
        EvaluationOutcome::of_call(false, true, 0.0),
        EvaluationOutcome::NotRun
    );
    assert_ne!(MutationOutcome::Err, MutationOutcome::Ok);
}

/// Minimal empty workbook for the wrapper tests.
fn make_model() -> ModelStore {
    StoredValue::new_local(
        UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("empty test model"),
    )
}

/// One wrapper call publishes one sample, whatever the closure did, and the
/// sequence grows with every call — two mutations in one task both reach the
/// store instead of the second overwriting the first.
#[wasm_bindgen_test]
#[cfg(feature = "dev-tools")]
fn each_wrapper_call_publishes_its_own_sample() {
    let owner = Owner::new();
    owner.with(|| {
        let perf = PerfTimings::new();
        provide_context(perf);
        let model = make_model();

        let failed = try_mutate(model, EvaluationMode::Immediate, |_m| {
            Err::<(), &'static str>("rejected")
        });
        assert_eq!(failed, Err("rejected"));
        let first = perf
            .mutation
            .get_untracked()
            .expect("failed call publishes");
        assert_eq!(first.seq, 1);
        assert_eq!(first.outcome, MutationOutcome::Err);
        assert_eq!(
            first.evaluation,
            EvaluationOutcome::NotRun,
            "the closure failed, so evaluation never ran"
        );

        let ok = try_mutate(model, EvaluationMode::Immediate, |_m| {
            Ok::<(), &'static str>(())
        });
        assert_eq!(ok, Ok(()));
        let second = perf
            .mutation
            .get_untracked()
            .expect("successful call publishes");
        assert_eq!(second.seq, 2);
        assert_eq!(second.outcome, MutationOutcome::Ok);
        assert!(
            matches!(second.evaluation, EvaluationOutcome::Measured { ms } if ms >= 0.0),
            "immediate evaluation reports its own duration, got {:?}",
            second.evaluation
        );
        assert!(second.started_at_ms > 0.0);
    });
}

#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn sink_receives_every_wrapper_result_after_the_model_borrow_ends() {
    use std::{cell::RefCell, rc::Rc};

    Owner::new().with(|| {
        let perf = PerfTimings::new();
        provide_context(perf);
        let model = make_model();
        let samples = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::clone(&samples);
        perf.set_sample_sink(Some(Rc::new(move |sample| {
            // The sink must be able to inspect the completed model update.
            model.with_value(|m| assert!(m.get_cell_content(0, 1, 1).is_ok()));
            received.borrow_mut().push(*sample);
        })));

        for mode in [EvaluationMode::Immediate, EvaluationMode::Deferred] {
            mutate(model, mode, |m| {
                m.set_user_input(0, 1, 1, "=1+2").unwrap();
            });
            assert_eq!(try_mutate(model, mode, |_| Ok::<_, &str>(())), Ok(()));
            assert_eq!(
                try_mutate(model, mode, |_| Err("rejected")),
                Err("rejected")
            );
        }

        let samples = samples.borrow();
        assert_eq!(samples.len(), 6, "no sample is coalesced within one task");
        for (index, sample) in samples.iter().enumerate() {
            assert_eq!(sample.seq, index as u64 + 1);
            assert!(sample.apply_ms.is_finite() && sample.apply_ms >= 0.0);
            if index % 3 == 2 {
                assert_eq!(sample.outcome, MutationOutcome::Err);
                assert_eq!(sample.evaluation, EvaluationOutcome::NotRun);
            } else {
                assert_eq!(sample.outcome, MutationOutcome::Ok);
                if index < 3 {
                    assert!(matches!(sample.evaluation,
                        EvaluationOutcome::Measured { ms } if ms.is_finite() && ms >= 0.0));
                } else {
                    assert_eq!(sample.evaluation, EvaluationOutcome::Deferred);
                }
            }
        }
        assert_eq!(perf.mutation.get_untracked(), samples.last().copied());
    });
}

#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn sink_can_remove_itself_during_delivery() {
    use std::{cell::Cell, rc::Rc};

    Owner::new().with(|| {
        let perf = PerfTimings::new();
        provide_context(perf);
        let model = make_model();
        let calls = Rc::new(Cell::new(0));
        let received = Rc::clone(&calls);
        perf.set_sample_sink(Some(Rc::new(move |_| {
            received.set(received.get() + 1);
            perf.set_sample_sink(None);
        })));
        mutate(model, EvaluationMode::Deferred, |_| {});
        mutate(model, EvaluationMode::Deferred, |_| {});
        assert_eq!(calls.get(), 1);
        assert_eq!(perf.mutation.get_untracked().unwrap().seq, 2);
    });
}

#[cfg(not(feature = "dev-tools"))]
#[wasm_bindgen_test]
fn wrappers_do_not_publish_samples_without_dev_tools() {
    Owner::new().with(|| {
        let perf = PerfTimings::new();
        provide_context(perf);
        let model = make_model();
        mutate(model, EvaluationMode::Immediate, |_| {});
        assert_eq!(
            try_mutate(model, EvaluationMode::Deferred, |_| Err("rejected")),
            Err("rejected")
        );
        assert_eq!(perf.mutation.get_untracked(), None);
        assert_eq!(perf.render_call.get_untracked(), None);
    });
}

#[wasm_bindgen_test]
fn wrappers_work_without_timing_context() {
    Owner::new().with(|| {
        let model = make_model();
        mutate(model, EvaluationMode::Immediate, |m| {
            m.set_user_input(0, 1, 1, "=1+2").unwrap();
        });
        model.with_value(|m| assert_eq!(m.get_formatted_cell_value(0, 1, 1).unwrap(), "3"));
    });
}
