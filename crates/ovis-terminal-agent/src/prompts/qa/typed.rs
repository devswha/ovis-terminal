//! Answers delivered through the terminal input path and answer schemas.

use std::collections::BTreeMap;

use ovis_terminal_protocol::errors::FieldPath;
use ovis_terminal_protocol::schema::methods::status::{
    AnswerDelivery, PromptChoice, PromptField, PromptState,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::*;

#[test]
fn a_typed_answer_is_admitted_once_as_literal_text_for_the_prompts_program() -> TestResult {
    let h = harness()?;
    let mut definition = question(PromptMechanism::Keys);
    definition
        .choices
        .push(PromptChoice::new("y".to_owned(), "Yes".to_owned()));
    h.open(AGENT_TOKEN, ("p1", definition))?;

    let choice = h.answer("p1", (1, PromptAnswer::Choice("y".to_owned())))?;
    *h.keys.refuse.lock().map_err(|_| "keys")? = Some(KeyInputError::NoLease);
    let no_lease = h.answer("p1", (1, text("y")))?;
    let typed = result(&h.answer("p1", (1, text("y")))?)?;

    assert_eq!(kind(&choice), Some("delivery_unsupported"));
    assert_eq!(kind(&no_lease), Some("no_lease"));
    assert_eq!(typed.state, OpenEnum::Known(PromptState::Answered));
    let receipt = typed.answer.ok_or("receipt")?;
    assert_eq!(receipt.delivery, OpenEnum::Known(AnswerDelivery::Typed));
    let admitted = h.keys.admitted.lock().map_err(|_| "keys")?.clone();
    let owner = ProcessOwner::new(terminal(AGENT)?, Incarnation::new(1));
    assert_eq!(admitted.len(), 1);
    let keys = admitted.first().ok_or("admitted keys")?;
    assert_eq!((keys.owner, keys.text.as_str()), (owner, "y"));
    Ok(())
}

#[test]
fn without_an_input_path_a_typed_prompt_refuses_its_answer_and_stays_open() -> TestResult {
    let h = harness()?;
    let bare = PromptService::new(h.identity.runtime_id(), Arc::clone(&h.status));
    let mut open = Open::new(
        terminal(AGENT)?,
        token(AGENT_TOKEN),
        question(PromptMechanism::Keys),
    );
    open.prompt_id = Some(prompt_id("p1"));
    h.run(|snapshot| bare.open(snapshot, open))?;
    let place = (prompt_id("p1"), RecordVersion::new(1));
    let op = OpId::new(uuid::Uuid::now_v7().to_string());
    let answer = Answer::new((terminal(AGENT)?, op), place, text("y"));

    let refused = h.run(|snapshot| bare.answer(snapshot, answer))?;

    assert_eq!(kind(&refused), Some("delivery_unsupported"));
    let still = h.get("p1")?;
    assert_eq!(
        (still.version, still.is_open()),
        (RecordVersion::new(1), true)
    );
    Ok(())
}

#[test]
fn answers_must_match_the_declared_choices_or_fields() -> TestResult {
    let h = harness()?;
    let mut form = question(PromptMechanism::Extension);
    form.fields = vec![
        PromptField::new("name".to_owned(), true),
        PromptField::new("note".to_owned(), false),
    ];
    h.open(AGENT_TOKEN, ("form", form))?;
    let fields = |pairs: &[(&str, &str)]| {
        PromptAnswer::Fields(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        )
    };

    let missing = h.answer("form", (1, fields(&[("note", "n")])))?;
    let unknown = h.answer("form", (1, fields(&[("name", "a"), ("extra", "b")])))?;
    let as_text = h.answer("form", (1, text("a")))?;
    let accepted = result(&h.answer("form", (1, fields(&[("name", "a")])))?)?;

    for refused in [&missing, &unknown] {
        let error = refused.error.as_ref().ok_or("refusal")?;
        assert_eq!(error.kind.token(), "answer_invalid");
        assert_eq!(
            error.field_path.as_ref().map(FieldPath::as_str),
            Some("params.answer.fields")
        );
    }
    assert_eq!(kind(&as_text), Some("answer_invalid"));
    let receipt = accepted.answer.ok_or("receipt")?;
    assert_eq!(receipt.delivery, OpenEnum::Known(AnswerDelivery::Extension));
    assert_eq!(receipt.answer, fields(&[("name", "a")]));
    Ok(())
}
