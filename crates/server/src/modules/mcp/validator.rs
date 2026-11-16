//! Strict Parameter Validator for MCP Tools
//!
//! Validates agent-provided arguments against tool JSON Schema before dispatching HTTP calls.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    MissingRequiredProperty(String),
    TypeMismatch {
        property: String,
        expected: String,
        found: String,
    },
    ToolDisabled(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRequiredProperty(prop) => {
                write!(f, "missing required argument property `{prop}`")
            }
            Self::TypeMismatch {
                property,
                expected,
                found,
            } => {
                write!(
                    f,
                    "type mismatch for property `{property}`: expected {expected}, found {found}"
                )
            }
            Self::ToolDisabled(name) => {
                write!(f, "tool `{name}` is disabled by administrative policy")
            }
        }
    }
}

pub struct ParameterValidator;

impl ParameterValidator {
    /// Validates `arguments` against `schema`.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] if arguments violate schema constraints.
    pub fn validate(arguments: &Value, schema: &Value) -> Result<(), ValidationError> {
        let empty_map = serde_json::Map::new();
        let args_obj = arguments.as_object().unwrap_or(&empty_map);

        if let Some(req_array) = schema.get("required").and_then(Value::as_array) {
            for req in req_array {
                if let Some(prop_name) = req.as_str()
                    && !args_obj.contains_key(prop_name)
                {
                    return Err(ValidationError::MissingRequiredProperty(
                        prop_name.to_owned(),
                    ));
                }
            }
        }

        if let Some(props_obj) = schema.get("properties").and_then(Value::as_object) {
            for (prop_name, prop_schema) in props_obj {
                if let Some(arg_val) = args_obj.get(prop_name) {
                    Self::validate_type(prop_name, arg_val, prop_schema)?;
                }
            }
        }

        Ok(())
    }

    fn validate_type(name: &str, val: &Value, schema: &Value) -> Result<(), ValidationError> {
        let expected_type = match schema.get("type").and_then(Value::as_str) {
            Some(t) => t,
            None => return Ok(()),
        };

        let matches = match expected_type {
            "string" => val.is_string(),
            "number" => val.is_number(),
            "integer" => val.is_i64() || val.is_u64(),
            "boolean" => val.is_boolean(),
            "array" => val.is_array(),
            "object" => val.is_object(),
            "null" => val.is_null(),
            _ => true,
        };

        if !matches {
            let found_type = match val {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            return Err(ValidationError::TypeMismatch {
                property: name.to_owned(),
                expected: expected_type.to_owned(),
                found: found_type.to_owned(),
            });
        }

        Ok(())
    }
}
