use fkst_control_plane::schedule::{
    decide, parse_cron_job, parse_marker, render_marker, JobDef, RunRecord, RunStatus,
    ScheduleAction, ScheduleState,
};
use k8s_openapi::chrono::{TimeZone, Utc};

const TRIGGER_BODY: &str = r#"### Schedule
cron: 0 3 * * *
timezone: UTC

### Job Type
raise

### Raise Label
fkst-dev

### Raise Title
Daily maintenance

### Raise Body
Run the daily maintenance workflow.
"#;

#[test]
fn due_utc_daily_raise_job_round_trips_its_run_marker() {
    let trigger_created_at = Utc
        .with_ymd_and_hms(2026, 7, 27, 2, 0, 0)
        .single()
        .expect("valid creation timestamp");
    let before_slot = Utc
        .with_ymd_and_hms(2026, 7, 27, 2, 59, 59)
        .single()
        .expect("valid pre-slot timestamp");
    let slot = Utc
        .with_ymd_and_hms(2026, 7, 27, 3, 0, 0)
        .single()
        .expect("valid slot timestamp");

    let spec = parse_cron_job(TRIGGER_BODY).expect("accepted raise trigger");
    assert_eq!(spec.schedule.timezone, "UTC");
    assert_eq!(
        &spec.job,
        &JobDef::Raise {
            label: "fkst-dev".to_string(),
            title: "Daily maintenance".to_string(),
            body: "Run the daily maintenance workflow.".to_string(),
        }
    );
    let state = ScheduleState { trigger_created_at };

    assert_eq!(
        decide(&spec, &state, before_slot).expect("decision"),
        ScheduleAction::Nothing
    );
    assert_eq!(
        decide(&spec, &state, slot).expect("decision"),
        ScheduleAction::ExecuteRaise { slot }
    );

    let record = RunRecord {
        slot,
        manual: false,
        status: RunStatus::Ok,
        started: slot,
        ended: Some(slot),
        issue: Some(3901),
    };
    let marker = render_marker(&record);
    assert_eq!(parse_marker(&marker).expect("rendered marker"), record);
}
