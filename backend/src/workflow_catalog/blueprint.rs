//! Strict parsing for repo-authored `fkst.workflow.v1` JSON blueprints.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowBlueprint {
    pub id: String,
    pub version: String,
    pub summary: Option<String>,
    pub applies_when: Option<String>,
    pub selector: Option<Selector>,
    pub steps: Vec<WorkflowStep>,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub labels_any: Vec<String>,
    pub title_contains_any: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStep {
    pub id: String,
    pub title: String,
    pub kind: StepKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Static,
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintError {
    detail: String,
}

impl BlueprintError {
    fn new(path: &str, detail: impl Into<String>) -> Self {
        Self {
            detail: format!("{path}: {}", detail.into()),
        }
    }
}

impl fmt::Display for BlueprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for BlueprintError {}

pub fn parse_blueprint(path: &str, bytes: &[u8]) -> Result<WorkflowBlueprint, BlueprintError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| BlueprintError::new("document", format!("invalid JSON: {error}")))?;
    let root = object(&value, "$")?;
    reject_unknown(
        root,
        "$",
        &["schema", "id", "version", "summary", "applies_when", "selector", "steps"],
    )?;

    let schema = required_string(root, "schema", "$.schema", None)?;
    if schema != "fkst.workflow.v1" {
        return Err(BlueprintError::new(
            "$.schema",
            "must equal fkst.workflow.v1",
        ));
    }

    let id = required_string(root, "id", "$.id", Some(128))?;
    let version = required_string(root, "version", "$.version", Some(64))?;
    let summary = optional_string(root, "summary", "$.summary", 512)?;
    let applies_when = optional_string(root, "applies_when", "$.applies_when", 1024)?;
    let selector = root
        .get("selector")
        .map(parse_selector)
        .transpose()?;
    let steps_value = root
        .get("steps")
        .ok_or_else(|| BlueprintError::new("$.steps", "is required"))?;
    let steps_array = steps_value
        .as_array()
        .ok_or_else(|| BlueprintError::new("$.steps", "must be an array"))?;
    if !(1..=16).contains(&steps_array.len()) {
        return Err(BlueprintError::new(
            "$.steps",
            "must contain between 1 and 16 entries",
        ));
    }

    let mut step_ids = BTreeSet::new();
    let mut steps = Vec::with_capacity(steps_array.len());
    for (index, step_value) in steps_array.iter().enumerate() {
        let step_path = format!("$.steps[{index}]");
        let step = object(step_value, &step_path)?;
        reject_unknown(step, &step_path, &["id", "title", "content"])?;
        let step_id = required_string(step, "id", &format!("{step_path}.id"), Some(128))?;
        if !step_ids.insert(step_id.clone()) {
            return Err(BlueprintError::new(
                &format!("{step_path}.id"),
                "must be unique within the blueprint",
            ));
        }
        let title = required_string(step, "title", &format!("{step_path}.title"), Some(200))?;
        let content_path = format!("{step_path}.content");
        let content = step
            .get("content")
            .ok_or_else(|| BlueprintError::new(&content_path, "is required"))?;
        let content = object(content, &content_path)?;
        reject_unknown(content, &content_path, &["kind", "intent", "generator"])?;
        let kind_path = format!("{content_path}.kind");
        let kind = required_string(content, "kind", &kind_path, None)?;
        let kind = match kind.as_str() {
            "static" => {
                required_string(
                    content,
                    "intent",
                    &format!("{content_path}.intent"),
                    Some(8000),
                )?;
                if content.contains_key("generator") {
                    return Err(BlueprintError::new(
                        &format!("{content_path}.generator"),
                        "is forbidden when kind is static",
                    ));
                }
                StepKind::Static
            }
            "generated" => {
                required_string(
                    content,
                    "generator",
                    &format!("{content_path}.generator"),
                    Some(8000),
                )?;
                if content.contains_key("intent") {
                    return Err(BlueprintError::new(
                        &format!("{content_path}.intent"),
                        "is forbidden when kind is generated",
                    ));
                }
                StepKind::Generated
            }
            _ => {
                return Err(BlueprintError::new(
                    &kind_path,
                    "must equal static or generated",
                ));
            }
        };
        steps.push(WorkflowStep {
            id: step_id,
            title,
            kind,
        });
    }

    Ok(WorkflowBlueprint {
        id,
        version,
        summary,
        applies_when,
        selector,
        steps,
        source_path: path.to_string(),
    })
}

fn parse_selector(value: &Value) -> Result<Selector, BlueprintError> {
    let selector = object(value, "$.selector")?;
    reject_unknown(
        selector,
        "$.selector",
        &["labels_any", "title_contains_any"],
    )?;
    Ok(Selector {
        labels_any: optional_string_array(selector, "labels_any", "$.selector.labels_any")?,
        title_contains_any: optional_string_array(
            selector,
            "title_contains_any",
            "$.selector.title_contains_any",
        )?,
    })
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, BlueprintError> {
    value
        .as_object()
        .ok_or_else(|| BlueprintError::new(path, "must be an object"))
}

fn reject_unknown(
    object: &Map<String, Value>,
    path: &str,
    allowed: &[&str],
) -> Result<(), BlueprintError> {
    if let Some(field) = object.keys().find(|field| !allowed.contains(&field.as_str())) {
        return Err(BlueprintError::new(
            &format!("{path}.{field}"),
            "is not allowed",
        ));
    }
    Ok(())
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
    limit: Option<usize>,
) -> Result<String, BlueprintError> {
    let value = object
        .get(field)
        .ok_or_else(|| BlueprintError::new(path, "is required"))?;
    let value = value
        .as_str()
        .ok_or_else(|| BlueprintError::new(path, "must be a string"))?;
    if let Some(limit) = limit {
        enforce_byte_limit(value, path, limit)?;
    }
    Ok(value.to_string())
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
    limit: usize,
) -> Result<Option<String>, BlueprintError> {
    object
        .get(field)
        .map(|_| required_string(object, field, path, Some(limit)))
        .transpose()
}

fn optional_string_array(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Vec<String>, BlueprintError> {
    let Some(value) = object.get(field) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| BlueprintError::new(path, "must be an array"))?;
    if values.len() > 16 {
        return Err(BlueprintError::new(path, "must contain at most 16 entries"));
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let item_path = format!("{path}[{index}]");
            let value = value
                .as_str()
                .ok_or_else(|| BlueprintError::new(&item_path, "must be a string"))?;
            enforce_byte_limit(value, &item_path, 128)?;
            Ok(value.to_string())
        })
        .collect()
}

fn enforce_byte_limit(value: &str, path: &str, limit: usize) -> Result<(), BlueprintError> {
    if value.len() > limit {
        return Err(BlueprintError::new(
            path,
            format!("must be at most {limit} UTF-8 bytes"),
        ));
    }
    Ok(())
}
