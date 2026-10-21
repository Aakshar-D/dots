//! The check-in tools' input rules and the follow-up limits (check-ins spec sections 2 and 3):
//! the same for both engines, so they are checked here once.

use chrono::{DateTime, Local, TimeDelta, Utc};
use dots_core::checkins::{check, local_label, Request, Tool, When};
use dots_core::followups::{due_at, MAX_AHEAD, MAX_PENDING, MIN_AHEAD};
use dots_core::model::EngineKind;
use dots_core::policy::CHECK_IN_TOOLS;
use serde_json::{json, Value};

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn err(tool: Tool, input: Value) -> String {
    check(tool, &input).unwrap_err()
}

/// When a `follow_up` call with `input` is due, or the rule it breaks.
fn when(input: Value) -> Result<When, String> {
    match check(Tool::FollowUp, &input) {
        Ok(Request::FollowUp { when, .. }) => Ok(when),
        Ok(other) => panic!("not a follow-up: {other:?}"),
        Err(e) => Err(e),
    }
}

#[test]
fn the_tools_have_their_names_on_both_engines() {
    let names: Vec<&str> = Tool::ALL.iter().map(|t| t.name()).collect();
    assert_eq!(names, ["notify", "ask", "follow_up"]);
    assert_eq!(Tool::ALL.map(Tool::policy_name), CHECK_IN_TOOLS);
    assert_eq!(Tool::from_name("follow_up"), Some(Tool::FollowUp));
    assert_eq!(Tool::from_name("FollowUp"), None);
    assert_eq!(Tool::from_policy_name("Ask"), Some(Tool::Ask));
    assert_eq!(Tool::from_policy_name("ask"), None);
    assert_eq!(Tool::Ask.model_name(EngineKind::Claude), "mcp__dots__ask");
    assert_eq!(
        Tool::FollowUp.model_name(EngineKind::Claude),
        "mcp__dots__follow_up"
    );
    assert_eq!(Tool::FollowUp.model_name(EngineKind::Local), "follow_up");
    let schema = Tool::FollowUp.mcp_schema();
    assert_eq!(schema["name"], "follow_up");
    assert_eq!(schema["inputSchema"]["required"], json!(["note"]));
    assert_eq!(
        Tool::Ask.mcp_schema()["inputSchema"]["properties"]["choices"]["type"],
        "array"
    );
    for tool in Tool::ALL {
        assert!(!tool.description().is_empty());
    }
}

#[test]
fn text_is_counted_in_characters_after_trimming() {
    let at_limit = "é".repeat(2_000);
    assert_eq!(
        check(Tool::Notify, &json!({"text": format!("  {at_limit}\n")})),
        Ok(Request::Notify {
            text: at_limit.clone()
        })
    );
    assert_eq!(
        err(Tool::Notify, json!({"text": format!("{at_limit}é")})),
        "`text` must be at most 2,000 characters."
    );
    assert_eq!(
        err(Tool::Notify, json!({"text": " \n\t "})),
        "`text` must not be blank."
    );
    for missing in [json!({}), json!({"text": 5}), json!({"text": null})] {
        assert_eq!(
            err(Tool::Notify, missing),
            "`text` must be a string of 1 to 2,000 characters."
        );
    }
    assert_eq!(
        err(Tool::Ask, json!({"question": ""})),
        "`question` must not be blank."
    );
    assert_eq!(
        err(
            Tool::FollowUp,
            json!({"note": "x".repeat(2_001), "in": "1h"})
        ),
        "`note` must be at most 2,000 characters."
    );
}

#[test]
fn a_question_offers_one_to_five_choices_of_up_to_80_characters() {
    let emoji = "🙂".repeat(80);
    assert_eq!(
        check(
            Tool::Ask,
            &json!({"question": " Ship it? ", "choices": [" yes ", emoji.clone()]})
        ),
        Ok(Request::Ask {
            question: "Ship it?".into(),
            choices: Some(vec!["yes".into(), emoji.clone()]),
        })
    );
    assert_eq!(
        check(Tool::Ask, &json!({"question": "Why?", "choices": null})),
        Ok(Request::Ask {
            question: "Why?".into(),
            choices: None,
        })
    );
    let list = "`choices` must be a list of 1 to 5 strings.";
    for choices in [
        json!([]),
        json!(["a", "b", "c", "d", "e", "f"]),
        json!("yes"),
        json!([1]),
    ] {
        assert_eq!(
            err(Tool::Ask, json!({"question": "q", "choices": choices})),
            list
        );
    }
    let each = "Each choice must be 1 to 80 characters.";
    assert_eq!(
        err(Tool::Ask, json!({"question": "q", "choices": ["  "]})),
        each
    );
    assert_eq!(
        err(
            Tool::Ask,
            json!({"question": "q", "choices": [format!("{emoji}🙂")]})
        ),
        each
    );
}

#[test]
fn follow_up_times_are_read_and_limited_at_their_edges() {
    assert_eq!(
        when(json!({"note": "n", "in": "90m"})),
        Ok(When::In(TimeDelta::minutes(90)))
    );
    assert_eq!(
        when(json!({"note": "n", "in": " 2h "})),
        Ok(When::In(TimeDelta::hours(2)))
    );
    // A blank `at` next to `in` counts as absent.
    assert_eq!(
        when(json!({"note": "n", "in": "1d", "at": ""})),
        Ok(When::In(TimeDelta::days(1)))
    );
    assert_eq!(
        when(json!({"note": "n", "at": "2030-01-07T10:00:00+01:00"})),
        Ok(When::At(t0()))
    );
    // A number too large for a duration reads as the longest one, which the limit refuses.
    assert_eq!(
        when(json!({"note": "n", "in": "99999999999999999999d"})),
        Ok(When::In(TimeDelta::MAX))
    );
    let unit = "`in` must be a whole number and a unit m, h or d, such as 90m, 2h or 1d.";
    for bad in ["2 hours", "90", "1.5h", "-5m", "h", "2w", "2H", "1d2h"] {
        assert_eq!(
            when(json!({"note": "n", "in": bad})),
            Err(unit.to_string()),
            "{bad}"
        );
    }
    assert_eq!(when(json!({"note": "n", "in": 90})), Err(unit.to_string()));
    let offset =
        "`at` must be a time in RFC 3339 with an offset, such as 2026-10-21T09:00:00+02:00.";
    for bad in ["2030-01-07T10:00:00", "tomorrow 9am", "2030-01-07"] {
        assert_eq!(
            when(json!({"note": "n", "at": bad})),
            Err(offset.to_string()),
            "{bad}"
        );
    }
    let one = "Give exactly one of `at` or `in`.";
    assert_eq!(when(json!({"note": "n"})), Err(one.to_string()));
    assert_eq!(when(json!({"note": "n", "in": " "})), Err(one.to_string()));
    assert_eq!(
        when(json!({"note": "n", "in": "2h", "at": "2030-01-07T10:00:00Z"})),
        Err(one.to_string())
    );

    // The limits, against the clock of the follow-up loop.
    let soon = "A follow-up must be at least 5 minutes away.";
    let far = "A follow-up must be at most 30 days away.";
    assert_eq!(due_at(When::In(MIN_AHEAD), t0()), Ok(t0() + MIN_AHEAD));
    assert_eq!(due_at(When::In(TimeDelta::minutes(4)), t0()), Err(soon));
    assert_eq!(due_at(When::In(TimeDelta::zero()), t0()), Err(soon));
    assert_eq!(
        due_at(When::At(t0() - TimeDelta::hours(1)), t0()),
        Err(soon)
    );
    assert_eq!(
        due_at(When::At(t0() + TimeDelta::seconds(299)), t0()),
        Err(soon)
    );
    assert_eq!(
        due_at(When::In(TimeDelta::days(30)), t0()),
        Ok(t0() + MAX_AHEAD)
    );
    assert_eq!(due_at(When::In(TimeDelta::hours(721)), t0()), Err(far));
    assert_eq!(due_at(When::In(TimeDelta::MAX), t0()), Err(far));
    assert_eq!(MAX_PENDING, 3);
}

#[test]
fn times_are_shown_in_local_time_as_weekday_day_month_and_time() {
    let label = local_label(t0());
    assert_eq!(
        label,
        t0().with_timezone(&Local)
            .format("%a %-d %b %H:%M")
            .to_string()
    );
    assert_eq!(label.split(' ').count(), 4, "{label}");
}
