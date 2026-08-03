use super::blueprint::{parse_blueprint, StepKind};

#[test]
fn parses_the_walking_skeleton_blueprint_without_retaining_prompt_text() {
    let prompt = "Inspect the release candidate and report blocking defects.";
    let document = format!(
        r#"{{
          "schema": "fkst.workflow.v1",
          "id": "release-hardening",
          "version": "1.0.0",
          "summary": "Check a release candidate before publication.",
          "applies_when": "A release candidate needs final review.",
          "selector": {{
            "labels_any": ["release"],
            "title_contains_any": ["release"]
          }},
          "steps": [{{
            "id": "verify",
            "title": "Verify the release candidate",
            "content": {{
              "kind": "static",
              "intent": "{prompt}"
            }}
          }}]
        }}"#
    );

    let blueprint = parse_blueprint(".fkst/packages/release-hardening.json", document.as_bytes())
        .expect("valid blueprint");

    assert_eq!(blueprint.id, "release-hardening");
    assert_eq!(blueprint.version, "1.0.0");
    assert_eq!(blueprint.steps.len(), 1);
    assert_eq!(blueprint.steps[0].id, "verify");
    assert_eq!(blueprint.steps[0].kind, StepKind::Static);
    assert!(!format!("{blueprint:?}").contains(prompt));
}
