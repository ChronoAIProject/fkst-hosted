#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use base64::Engine as _;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const CONTRACT_REGISTRY: &str = include_str!("../../contracts/registry.json");
const FOUNDATION_SCHEMA: &str =
    include_str!("../../contracts/qa.contract-foundation/v1/schema.json");
const FOUNDATION_SCHEMA_NAME: &str = "qa.contract-foundation/v1";
const FOUNDATION_SCHEMA_PATH: &str = "contracts/qa.contract-foundation/v1/schema.json";
const FOUNDATION_SCHEMA_ID: &str = "urn:chronoai:fkst:qa-contracts:qa.contract-foundation:v1";
const MAX_DEPTH: usize = 128;
const MAX_SAFE_INTEGER_TEXT: &str = "9007199254740991";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Rejection {
    pub category: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    pub reason: String,
    pub path: String,
}

impl Rejection {
    fn canonical(code: &'static str, reason: &'static str) -> Self {
        Self {
            category: "canonicalization",
            code: Some(code),
            reason: reason.into(),
            path: "/".into(),
        }
    }

    fn contract(code: &'static str, reason: &'static str, path: impl Into<String>) -> Self {
        Self {
            category: "contract",
            code: Some(code),
            reason: reason.into(),
            path: path.into(),
        }
    }

    fn validation(reason: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            category: "validation",
            code: None,
            reason: reason.into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Error)]
#[error("{0:?}")]
pub struct ContractError(pub Rejection);

#[derive(Clone, Debug)]
pub struct AdmittedJson(Value);

impl AdmittedJson {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ValidatedValue(Value);

impl ValidatedValue {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum FoundationType {
    ContractMeta,
    HostScopedMeta,
    ResourceRef,
    ActorRef,
    DigestBoundRef,
    SignatureBlock,
    ProjectionSpecimen,
    StrictUnionSpecimen,
}

impl FoundationType {
    pub const ALL: [Self; 8] = [
        Self::ContractMeta,
        Self::HostScopedMeta,
        Self::ResourceRef,
        Self::ActorRef,
        Self::DigestBoundRef,
        Self::SignatureBlock,
        Self::ProjectionSpecimen,
        Self::StrictUnionSpecimen,
    ];

    pub const fn definition(self) -> &'static str {
        match self {
            Self::ContractMeta => "ContractMeta",
            Self::HostScopedMeta => "HostScopedMeta",
            Self::ResourceRef => "ResourceRef",
            Self::ActorRef => "ActorRef",
            Self::DigestBoundRef => "DigestBoundRef",
            Self::SignatureBlock => "SignatureBlock",
            Self::ProjectionSpecimen => "ProjectionSpecimen",
            Self::StrictUnionSpecimen => "StrictUnionSpecimen",
        }
    }

    const fn fixture_only(self) -> bool {
        matches!(self, Self::ProjectionSpecimen | Self::StrictUnionSpecimen)
    }
}

#[derive(Debug, Deserialize)]
struct Registry {
    registry_version: String,
    profile: String,
    schemas: BTreeMap<String, RegistrySchemaEntry>,
    types: BTreeMap<String, RegistryTypeEntry>,
}

#[derive(Debug, Deserialize)]
struct RegistrySchemaEntry {
    path: String,
    id: String,
    major: u64,
}

#[derive(Debug, Deserialize)]
struct RegistryTypeEntry {
    schema: String,
    pointer: String,
    #[serde(default)]
    fixture_only: bool,
}

pub fn contract_registry() -> Result<Value, ContractError> {
    validate_registry()?;
    serde_json::from_str(CONTRACT_REGISTRY)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_registry", "/")))
}

pub fn admit_json(raw: &[u8]) -> Result<AdmittedJson, ContractError> {
    let text = std::str::from_utf8(raw).map_err(|_| {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_utf8",
            "invalid_utf8",
        ))
    })?;
    preflight_depth(text)?;
    preflight_numbers(text)?;
    validate_json_syntax(text)?;
    preflight_unicode_scalars(text)?;
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(classify_json_error)?
        .0;
    deserializer.end().map_err(classify_json_error)?;
    ensure_depth(&value, 0)?;
    Ok(AdmittedJson(value))
}

pub fn validate_foundation(
    raw: &[u8],
    foundation_type: FoundationType,
) -> Result<ValidatedValue, ContractError> {
    validate_value(admit_json(raw)?, foundation_type)
}

pub fn validate_value(
    admitted: AdmittedJson,
    foundation_type: FoundationType,
) -> Result<ValidatedValue, ContractError> {
    let value = admitted.0;
    validate_special_rules(&value, foundation_type)?;
    let schema = schema_for_type(foundation_type)?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    if let Err(error) = validator.validate(&value) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            pointer_or_root(error.instance_path().as_str()),
        )));
    }
    Ok(ValidatedValue(value))
}

pub fn validate_scalar(name: &str, value: &str) -> Result<(), ContractError> {
    let valid = match name {
        "ISO8601" => validate_iso8601(value),
        "Sha256" => validate_sha256(value),
        "Base64UrlNoPad" => validate_base64url_no_pad(value),
        "UUID" => validate_uuid(value),
        "SchemaVersion" => parse_schema_major(value).is_some(),
        _ => return Err(ContractError(Rejection::validation("unknown_scalar", "/"))),
    };
    if valid {
        Ok(())
    } else if name == "Base64UrlNoPad" {
        Err(ContractError(Rejection::contract(
            "contract.invalid_encoding",
            "invalid_encoding",
            "/",
        )))
    } else {
        Err(ContractError(Rejection::validation("invalid_scalar", "/")))
    }
}

pub fn canonical_bytes(value: &ValidatedValue) -> Result<Vec<u8>, ContractError> {
    canonicalize(&value.0)
}

pub fn canonical_admitted_bytes(value: &AdmittedJson) -> Result<Vec<u8>, ContractError> {
    canonicalize(&value.0)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

pub fn contract_content_projection(value: &ValidatedValue) -> Result<Vec<u8>, ContractError> {
    let mut projected = value.0.clone();
    let root = projected
        .as_object_mut()
        .ok_or_else(|| ContractError(Rejection::validation("projection_requires_object", "/")))?;
    root.remove("content_digest");
    root.remove("signature");
    canonicalize(&projected)
}

pub fn contract_content_digest(value: &ValidatedValue) -> Result<String, ContractError> {
    Ok(sha256_digest(&contract_content_projection(value)?))
}

pub fn verify_contract_content_digest(value: &ValidatedValue) -> Result<(), ContractError> {
    let observed = value
        .0
        .get("content_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "missing_content_digest",
                "/content_digest",
            ))
        })?;
    let expected = contract_content_digest(value)?;
    if observed == expected {
        Ok(())
    } else {
        Err(ContractError(Rejection::contract(
            "contract.digest_mismatch",
            "digest_mismatch",
            "/content_digest",
        )))
    }
}

fn validate_registry() -> Result<Registry, ContractError> {
    let registry: Registry = serde_json::from_str(CONTRACT_REGISTRY)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_registry", "/")))?;
    if registry.registry_version != "qa.contract-registry/v1"
        || registry.profile != "local_qa_host_mvp"
    {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_registry",
            "/",
        )));
    }
    let schema = registry
        .schemas
        .get(FOUNDATION_SCHEMA_NAME)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "invalid_embedded_registry",
                "/schemas",
            ))
        })?;
    if registry.schemas.len() != 1
        || schema.path != FOUNDATION_SCHEMA_PATH
        || schema.id != FOUNDATION_SCHEMA_ID
        || schema.major != 1
    {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_registry",
            "/schemas/qa.contract-foundation~1v1",
        )));
    }
    let expected_types: BTreeSet<_> = FoundationType::ALL
        .into_iter()
        .map(FoundationType::definition)
        .collect();
    let registered_types: BTreeSet<_> = registry.types.keys().map(String::as_str).collect();
    if expected_types != registered_types {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_registry",
            "/types",
        )));
    }
    for foundation_type in FoundationType::ALL {
        let entry = registry
            .types
            .get(foundation_type.definition())
            .expect("type sets were compared above");
        if entry.schema != FOUNDATION_SCHEMA_NAME
            || entry.pointer != format!("#/$defs/{}", foundation_type.definition())
            || entry.fixture_only != foundation_type.fixture_only()
        {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{}", foundation_type.definition()),
            )));
        }
    }
    Ok(registry)
}

fn schema_for_type(foundation_type: FoundationType) -> Result<Value, ContractError> {
    let registry = validate_registry()?;
    let type_entry = registry
        .types
        .get(foundation_type.definition())
        .expect("validated registry covers every foundation type");
    let mut schema: Value = serde_json::from_str(FOUNDATION_SCHEMA)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    if schema.get("$id").and_then(Value::as_str) != Some(FOUNDATION_SCHEMA_ID) {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_schema",
            "/$id",
        )));
    }
    schema
        .as_object_mut()
        .expect("foundation schema is an object")
        .insert("$ref".into(), Value::String(type_entry.pointer.clone()));
    Ok(schema)
}

fn canonicalize(value: &Value) -> Result<Vec<u8>, ContractError> {
    serde_json_canonicalizer::to_vec(value)
        .map_err(|_| ContractError(Rejection::validation("canonicalization_failed", "/")))
}

fn validate_special_rules(
    value: &Value,
    foundation_type: FoundationType,
) -> Result<(), ContractError> {
    let object = value
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("expected_object", "/")))?;
    let allowed = allowed_fields(foundation_type, object)?;
    for key in object.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError(Rejection::contract(
                "contract.forbidden_field",
                "unknown_field",
                json_pointer(key),
            )));
        }
    }

    if let Some(version) = object.get("schema_version").and_then(Value::as_str) {
        let major = parse_schema_major(version).ok_or_else(|| {
            ContractError(Rejection::validation(
                "invalid_schema_version",
                "/schema_version",
            ))
        })?;
        if major != "1" {
            return Err(ContractError(Rejection::contract(
                "contract.unsupported_version",
                "unsupported_version",
                "/schema_version",
            )));
        }
    }

    match foundation_type {
        FoundationType::ContractMeta => validate_meta_scalars(object, true)?,
        FoundationType::HostScopedMeta => validate_meta_scalars(object, false)?,
        FoundationType::ActorRef => {
            validate_closed_enum(object, "type", &["user", "service", "device", "module"], "")?
        }
        FoundationType::SignatureBlock => validate_signature_block(object, "")?,
        FoundationType::DigestBoundRef => {
            validate_optional_scalar(object, "content_digest", "Sha256", "/content_digest")?
        }
        FoundationType::ResourceRef => {
            validate_optional_scalar(object, "digest", "Sha256", "/digest")?
        }
        FoundationType::ProjectionSpecimen => {
            validate_optional_scalar(object, "content_digest", "Sha256", "/content_digest")?;
            if let Some(signature) = object.get("signature").and_then(Value::as_object) {
                validate_signature_block(signature, "/signature")?;
            }
        }
        FoundationType::StrictUnionSpecimen => validate_strict_union(object)?,
    }
    Ok(())
}

fn validate_optional_scalar(
    object: &Map<String, Value>,
    field: &str,
    scalar: &str,
    path: &str,
) -> Result<(), ContractError> {
    if let Some(value) = object.get(field).and_then(Value::as_str) {
        validate_scalar(scalar, value).map_err(|mut error| {
            error.0.path = path.into();
            error
        })?;
    }
    Ok(())
}

fn validate_signature_block(
    object: &Map<String, Value>,
    path_prefix: &str,
) -> Result<(), ContractError> {
    let allowed = ["algorithm", "key_id", "value"];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ContractError(Rejection::contract(
                "contract.forbidden_field",
                "unknown_field",
                format!("{path_prefix}{}", json_pointer(key)),
            )));
        }
    }
    validate_closed_enum(object, "algorithm", &["ed25519", "es256"], path_prefix)?;
    if let Some(value) = object.get("value").and_then(Value::as_str) {
        validate_scalar("Base64UrlNoPad", value).map_err(|mut error| {
            error.0.path = format!("{path_prefix}/value");
            error
        })?;
    }
    Ok(())
}

fn allowed_fields(
    foundation_type: FoundationType,
    object: &Map<String, Value>,
) -> Result<BTreeSet<&'static str>, ContractError> {
    let fields: &[&str] = match foundation_type {
        FoundationType::ContractMeta => &[
            "schema_version",
            "content_digest",
            "run_id",
            "created_at",
            "producer_version",
            "correlation_id",
        ],
        FoundationType::HostScopedMeta => &[
            "schema_version",
            "content_digest",
            "host_instance_id",
            "created_at",
            "producer_version",
            "correlation_id",
        ],
        FoundationType::ResourceRef => &["kind", "id", "digest", "version"],
        FoundationType::ActorRef => &["type", "id", "display_name"],
        FoundationType::DigestBoundRef => {
            &["kind", "id", "schema_version", "content_digest", "version"]
        }
        FoundationType::SignatureBlock => &["algorithm", "key_id", "value"],
        FoundationType::ProjectionSpecimen => {
            &["schema_version", "content_digest", "signature", "payload"]
        }
        FoundationType::StrictUnionSpecimen => return strict_union_allowed_fields(object),
    };
    Ok(fields.iter().copied().collect())
}

fn strict_union_allowed_fields(
    object: &Map<String, Value>,
) -> Result<BTreeSet<&'static str>, ContractError> {
    let kind = object.get("kind").and_then(Value::as_str).ok_or_else(|| {
        ContractError(Rejection::contract(
            "contract.invalid_variant",
            "missing_required_field",
            "/kind",
        ))
    })?;
    let fields: &[&str] = match kind {
        "alpha" => &["kind", "common", "alpha_value"],
        "beta" => &["kind", "common", "beta_count"],
        _ => {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_variant",
                "unknown_discriminator",
                "/kind",
            )))
        }
    };
    let other = if kind == "alpha" {
        "beta_count"
    } else {
        "alpha_value"
    };
    if object.contains_key(other) {
        return Err(ContractError(Rejection::contract(
            "contract.forbidden_field",
            "mixed_variant_fields",
            json_pointer(other),
        )));
    }
    Ok(fields.iter().copied().collect())
}

fn validate_strict_union(object: &Map<String, Value>) -> Result<(), ContractError> {
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let required = if kind == "alpha" {
        ["common", "alpha_value"]
    } else {
        ["common", "beta_count"]
    };
    for field in required {
        if !object.contains_key(field) {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_variant",
                "missing_required_field",
                json_pointer(field),
            )));
        }
    }
    Ok(())
}

fn validate_meta_scalars(
    object: &Map<String, Value>,
    run_scoped: bool,
) -> Result<(), ContractError> {
    for (field, scalar) in [
        ("schema_version", "SchemaVersion"),
        ("content_digest", "Sha256"),
        ("created_at", "ISO8601"),
    ] {
        validate_optional_scalar(object, field, scalar, &json_pointer(field))?;
    }
    if run_scoped {
        validate_optional_scalar(object, "run_id", "UUID", "/run_id")?;
    }
    Ok(())
}

fn validate_closed_enum(
    object: &Map<String, Value>,
    field: &str,
    allowed: &[&str],
    path_prefix: &str,
) -> Result<(), ContractError> {
    if let Some(value) = object.get(field).and_then(Value::as_str) {
        if !allowed.contains(&value) {
            return Err(ContractError(Rejection::contract(
                "contract.unsupported_enum",
                "unsupported_enum",
                format!("{path_prefix}{}", json_pointer(field)),
            )));
        }
    }
    Ok(())
}

fn validate_iso8601(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || *bytes.last().expect("length checked") != b'Z'
    {
        return false;
    }
    let fixed_digits = bytes[..19]
        .iter()
        .enumerate()
        .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16))
        .all(|(_, byte)| byte.is_ascii_digit());
    if !fixed_digits {
        return false;
    }
    match bytes.get(19) {
        Some(b'Z') if bytes.len() == 20 => {}
        Some(b'.') if bytes.len() > 21 => {
            let fraction = &bytes[20..bytes.len() - 1];
            if !fraction.iter().all(u8::is_ascii_digit) || fraction.last() == Some(&b'0') {
                return false;
            }
        }
        _ => return false,
    }

    let Some(year) = decimal_field(bytes, 0, 4) else {
        return false;
    };
    let Some(month) = decimal_field(bytes, 5, 7) else {
        return false;
    };
    let Some(day) = decimal_field(bytes, 8, 10) else {
        return false;
    };
    let Some(hour) = decimal_field(bytes, 11, 13) else {
        return false;
    };
    let Some(minute) = decimal_field(bytes, 14, 16) else {
        return false;
    };
    let Some(second) = decimal_field(bytes, 17, 19) else {
        return false;
    };

    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap_year { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    day >= 1 && day <= days_in_month[(month - 1) as usize]
}

fn decimal_field(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    bytes.get(start..end)?.iter().try_fold(0, |value, byte| {
        byte.is_ascii_digit()
            .then_some(value * 10 + u32::from(byte - b'0'))
    })
}

fn validate_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_base64url_no_pad(value: &str) -> bool {
    !value.contains('=')
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .map(|bytes| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes) == value)
            .unwrap_or(false)
}

fn validate_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn parse_schema_major(value: &str) -> Option<&str> {
    let (domain, major) = value.rsplit_once("/v")?;
    if !domain.starts_with("qa.")
        || major.starts_with('0')
        || major.is_empty()
        || !major.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let name = &domain[3..];
    if name.is_empty()
        || !name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    {
        return None;
    }
    Some(major)
}

fn validate_json_syntax(text: &str) -> Result<(), ContractError> {
    let probe = syntax_probe_text(text);
    let mut deserializer = serde_json::Deserializer::from_str(&probe);
    deserializer.disable_recursion_limit();
    Value::deserialize(&mut deserializer)
        .map_err(|_| ContractError(Rejection::validation("invalid_json", "/")))?;
    deserializer
        .end()
        .map_err(|_| ContractError(Rejection::validation("invalid_json", "/")))
}

fn syntax_probe_text(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                if bytes.get(index + 1) == Some(&b'u') && unicode_escape(&bytes, index).is_some() {
                    bytes[index + 2..index + 6].fill(b'0');
                    index += 6;
                } else {
                    index += 2;
                }
            }
            _ => index += 1,
        }
    }
    String::from_utf8(bytes).expect("syntax probe preserves UTF-8")
}

fn preflight_unicode_scalars(text: &str) -> Result<(), ContractError> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                if bytes.get(index + 1) != Some(&b'u') {
                    index += 2;
                    continue;
                }
                let Some(code_unit) = unicode_escape(bytes, index) else {
                    index += 2;
                    continue;
                };
                if (0xd800..=0xdbff).contains(&code_unit) {
                    let Some(low_surrogate) = unicode_escape(bytes, index + 6) else {
                        return Err(invalid_unicode_scalar());
                    };
                    if !(0xdc00..=0xdfff).contains(&low_surrogate) {
                        return Err(invalid_unicode_scalar());
                    }
                    index += 12;
                } else if (0xdc00..=0xdfff).contains(&code_unit) {
                    return Err(invalid_unicode_scalar());
                } else {
                    index += 6;
                }
            }
            _ => index += 1,
        }
    }
    Ok(())
}

fn unicode_escape(bytes: &[u8], index: usize) -> Option<u16> {
    if bytes.get(index..index + 2)? != b"\\u" {
        return None;
    }
    bytes
        .get(index + 2..index + 6)?
        .iter()
        .try_fold(0, |value, byte| {
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return None,
            };
            Some(value * 16 + digit)
        })
}

fn invalid_unicode_scalar() -> ContractError {
    ContractError(Rejection::canonical(
        "canonicalization.invalid_unicode_scalar",
        "invalid_unicode_scalar",
    ))
}

fn preflight_depth(text: &str) -> Result<(), ContractError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if matches!(byte, b'{' | b'[') {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(ContractError(Rejection::validation("depth_overflow", "/")));
            }
        } else if matches!(byte, b'}' | b']') {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

fn ensure_depth(value: &Value, depth: usize) -> Result<(), ContractError> {
    if depth > MAX_DEPTH {
        return Err(ContractError(Rejection::validation("depth_overflow", "/")));
    }
    match value {
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| ensure_depth(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| ensure_depth(value, depth + 1)),
        _ => Ok(()),
    }
}

fn preflight_numbers(text: &str) -> Result<(), ContractError> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() || matches!(byte, b'+' | b'N' | b'I') {
            let start = index;
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}' | b':'
                )
            {
                index += 1;
            }
            check_number_token(&text[start..index])?;
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn check_number_token(token: &str) -> Result<(), ContractError> {
    if !valid_json_number(token) {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        )));
    }
    let number = token.parse::<f64>().map_err(|_| {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        ))
    })?;
    if !number.is_finite() {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        )));
    }
    let plain_integer_token = !token.contains(['.', 'e', 'E']);
    let renders_as_plain_integer = number.abs() < 1e21;
    if (plain_integer_token || renders_as_plain_integer) && exact_integer_exceeds_safe(token) {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.unsafe_integer",
            "unsafe_integer",
        )));
    }
    Ok(())
}

fn exact_integer_exceeds_safe(token: &str) -> bool {
    let unsigned = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (
            &unsigned[..index],
            unsigned[index + 1..].parse::<i64>().ok(),
        ),
        None => (unsigned, Some(0)),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{integer}{fraction}");
    let trimmed = digits.trim_start_matches('0').len();
    if trimmed == 0 {
        return false;
    }
    digits.drain(..digits.len() - trimmed);
    let Some(exponent) = exponent else {
        return false;
    };
    let scale = fraction.len() as i128 - i128::from(exponent);
    let integer_digits = if scale <= 0 {
        let zero_count = -scale;
        if digits.len() as i128 + zero_count > MAX_SAFE_INTEGER_TEXT.len() as i128 {
            return true;
        }
        digits.extend(std::iter::repeat_n('0', zero_count as usize));
        digits.as_str()
    } else {
        if scale >= digits.len() as i128 {
            return false;
        }
        let split = digits.len() - scale as usize;
        if !digits[split..].bytes().all(|byte| byte == b'0') {
            return false;
        }
        &digits[..split]
    };
    integer_digits.len() > MAX_SAFE_INTEGER_TEXT.len()
        || (integer_digits.len() == MAX_SAFE_INTEGER_TEXT.len()
            && integer_digits > MAX_SAFE_INTEGER_TEXT)
}

fn valid_json_number(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut index = 0;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    index == bytes.len()
}

fn classify_json_error(error: serde_json::Error) -> ContractError {
    let message = error.to_string();
    if message.contains("duplicate member") {
        ContractError(Rejection::canonical(
            "canonicalization.duplicate_member",
            "duplicate_member",
        ))
    } else if message.contains("surrogate") || message.contains("unicode") {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_unicode_scalar",
            "invalid_unicode_scalar",
        ))
    } else if message.contains("number") {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        ))
    } else {
        ContractError(Rejection::validation("invalid_json", "/"))
    }
}

fn pointer_or_root(path: &str) -> String {
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}

fn json_pointer(field: &str) -> String {
    format!("/{}", field.replace('~', "~0").replace('/', "~1"))
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("I-JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(|number| StrictValue(Value::Number(number)))
            .ok_or_else(|| E::custom("invalid number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate member"));
            }
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}
